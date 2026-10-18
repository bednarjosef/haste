// The world and chunk packets: the view-distance and biome-container bookkeeping the server
// sends around the chunk stream, and the chunk-batch flow control, plus the chunk, block and
// block-event updates themselves.

use haste_core::Direction;
use haste_protocol::haste_core::chunk::BlockEntityData;
use haste_protocol::packets::play::serverbound as sb;
use haste_protocol::{ClientboundPlay, ServerboundPlay};

use super::{Session, WorldSink};

impl Session {
    /// The chunk-cache and chunk-batch packets (they run even for a packet the player handled);
    /// returns the packet when it is none of them.
    pub(super) fn apply_world<W: WorldSink + ?Sized>(
        &mut self,
        p: ClientboundPlay,
        world: &mut W,
        out: &mut Vec<ServerboundPlay>,
    ) -> Option<ClientboundPlay> {
        use ClientboundPlay as P;
        match p {
            // Vanilla `handleSetChunkCacheCenter` moves the ClientChunkCache's window; our
            // world keeps every chunk it is given, so there is nothing to move. Debug-logged:
            // it fires on every chunk border crossing, which would flood long sessions.
            P::SetChunkCacheCenter(c) => {
                tracing::debug!("set_chunk_cache_center ({}, {})", c.x, c.z);
            }
            // Vanilla `ClientPacketListener.handleSetChunkCacheRadius`: the new server view
            // distance (the game thread caps the meshed radius with it every drain).
            P::SetChunkCacheRadius(r) => {
                self.view_distance = r.radius;
            }
            // Vanilla `ClientPacketListener.handleChunksBiomes`: new biome containers for
            // loaded chunks (`/fillbiome`); the world re-tints the 3x3 columns.
            P::ChunksBiomes(b) => {
                for c in &b.chunks {
                    world.replace_biomes(c.pos, &c.data);
                }
            }
            // The chunk batch flow control (vanilla `ClientPacketListener.handleChunkBatchStart`/
            // `handleChunkBatchFinished` + `ChunkBatchSizeCalculator`): time the batch, fold the
            // per-chunk duration into the running average, and tell the server the rate we want.
            // Without the reply the server never sends the next batch, so a teleport's new area
            // never arrives.
            P::ChunkBatchStart(_) => {
                tracing::debug!("chunk batch start");
                self.batch_started_at = Some(std::time::Instant::now());
            }
            P::ChunkBatchFinished(b) => {
                tracing::debug!("chunk batch finished: {} chunks", b.batch_size);
                if b.batch_size > 0 {
                    if let Some(start) = self.batch_started_at.take() {
                        let nanos_per_chunk =
                            start.elapsed().as_secs_f64() * 1e9 / b.batch_size as f64;
                        // Clamp to a third..three times the running average, then the weighted
                        // mean (vanilla `onBatchFinished`).
                        let clamped = nanos_per_chunk.clamp(
                            self.aggregated_nanos_per_chunk / 3.0,
                            self.aggregated_nanos_per_chunk * 3.0,
                        );
                        self.aggregated_nanos_per_chunk = (self.aggregated_nanos_per_chunk
                            * self.old_samples_weight as f64
                            + clamped)
                            / (self.old_samples_weight as f64 + 1.0);
                        self.old_samples_weight = (self.old_samples_weight + 1).min(49);
                    }
                }
                out.push(ServerboundPlay::ChunkBatchReceived(
                    sb::ChunkBatchReceived {
                        // Vanilla `getDesiredChunksPerTick`: 7 ms over the per-chunk duration.
                        chunks_per_tick: (7_000_000.0 / self.aggregated_nanos_per_chunk) as f32,
                    },
                ));
            }
            _ => return Some(p),
        }
        None
    }

    /// The chunk, block, block-event and light updates; returns the packet when it is none of
    /// them.
    pub(super) fn apply_chunk_updates<W: WorldSink + ?Sized>(
        &mut self,
        p: ClientboundPlay,
        world: &mut W,
    ) -> Option<ClientboundPlay> {
        use ClientboundPlay as P;
        match p {
            P::LevelChunkWithLight(c) => match world.load_chunk(&c.chunk) {
                Ok(()) => self.counters.chunks_loaded += 1,
                Err(e) => {
                    self.counters.chunk_errors += 1;
                    if self.counters.chunk_errors <= 3 {
                        tracing::warn!("chunk {:?} not loaded: {e:#}", c.chunk.pos);
                    }
                    self.counters.last_chunk_error = Some(format!("{e:#}"));
                }
            },
            P::ForgetLevelChunk(f) => {
                // The chunk's block entities (and their lids) are gone with it.
                self.lids.retain(|pos, _| pos.chunk() != f.pos);
                self.bells.retain(|pos, _| pos.chunk() != f.pos);
                self.pots.retain(|pos, _| pos.chunk() != f.pos);
                world.unload_chunk(f.pos);
                self.counters.chunks_unloaded += 1;
            }
            P::BlockUpdate(b) => {
                let c = b.change();
                // The block entity, and with it its lid/bell/pot animation, only goes when the
                // block changes (`LevelChunk.setBlockState`), not on a state change of it.
                let block_changed = world.physics().is_none_or(|(_, data)| {
                    data.blocks.block_of(world.block_state_at(c.pos))
                        != data.blocks.block_of(c.state)
                });
                if block_changed {
                    self.lids.remove(&c.pos);
                    self.bells.remove(&c.pos);
                    self.pots.remove(&c.pos);
                }
                if !self.player.defer_block_update(c.pos, c.state) {
                    world.set_block(c.pos, c.state);
                }
            }
            // A block entity's `block_event` action 1: the opener count for a lid (chests use
            // `count > 0`, shulker boxes 0 to close and 1 to open, which `> 0` matches
            // exactly), the click direction for a bell (`BellBlockEntity.onHit`) and the wobble
            // style for a decorated pot (`DecoratedPotBlockEntity.wobble`).
            P::BlockEvent(b) if b.action == 1 => {
                let state = world.block_state_at(b.pos);
                if self.lid_states.as_ref().is_some_and(|s| s.contains(&state)) {
                    let lid = self.lids.entry(b.pos).or_default();
                    lid.open = b.param > 0;
                } else if self
                    .bell_states
                    .as_ref()
                    .is_some_and(|s| s.contains(&state))
                {
                    // `Direction.from3DDataValue(b1)`; vertical values leave the shake
                    // direction unset, which `BellModel.setupAnim` reads as no shake.
                    self.bells.insert(b.pos, Direction::from_ordinal(b.param));
                } else if self.pot_states.as_ref().is_some_and(|s| s.contains(&state)) {
                    // `WobbleStyle.values()[data]`: POSITIVE 7 ticks, NEGATIVE 10.
                    let duration = if b.param == 0 { 7 } else { 10 };
                    self.pots.insert(b.pos, (self.world_age, duration));
                }
            }
            P::BlockEntityData(b) => {
                let pos = b.pos;
                world.set_block_entity(
                    pos,
                    BlockEntityData {
                        pos,
                        kind: b.kind as u32,
                        nbt: b.nbt.clone(),
                    },
                );
            }
            P::SectionBlocksUpdate(u) => {
                for c in u.changes() {
                    if !self.player.defer_block_update(c.pos, c.state) {
                        world.set_block(c.pos, c.state);
                    }
                }
            }
            P::BlockDestruction(d) => {
                tracing::debug!(
                    "block_destruction {} {:?} stage {}",
                    d.entity_id,
                    d.pos,
                    d.stage
                );
                if d.stage <= 9 {
                    self.breaking.insert(d.entity_id, (d.pos, d.stage));
                } else {
                    self.breaking.remove(&d.entity_id);
                }
            }
            P::LightUpdate(l) => world.apply_light(l.pos, &l.light),
            _ => return Some(p),
        }
        None
    }
}
