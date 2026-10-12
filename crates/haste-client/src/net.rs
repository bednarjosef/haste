//! Network driver (client-app): owns the `Connection`, drains packets each tick and applies them
//! to the world, the entity store and the local player, and sends the per-tick packets.
//!
//! The driver does not own the world or the entities: each tick borrows them (`&mut`), so the
//! client can keep them behind its own locks and the render thread can read them between ticks.
//! [`WorldSink`] and [`EntitySink`] are the narrow surface the driver needs; they are implemented
//! for `haste_world::World` and `haste_entity::EntityStore` (one-line forwards) and for the
//! recording fakes in the tests.

use std::sync::Arc;

use anyhow::Context;
use glam::DVec3;
use haste_data::GameData;
use haste_entity::{EntityDimensions, EntityStore};
use haste_protocol::haste_core::chunk::{ChunkData, LightData};
use haste_protocol::haste_core::entity::{EntitySpawn, ItemStackData, MetadataEntry, PlayerInfo};
use haste_protocol::haste_core::game::{DimensionInfo, GameMode};
use haste_protocol::haste_core::{BlockPos, BlockStateId, ChunkPos, EntityId, Identifier};
use haste_protocol::packets::play::clientbound::{self as cb, teleport_flags};
use haste_protocol::packets::play::serverbound::{self as sb, move_flags};
use haste_protocol::{
    ClientboundPlay, ConnectOptions, Connection, Login, ServerboundPlay, StatsSnapshot,
    SyncedRegistries,
};
use haste_world::World;

/// What the driver needs from the world (`haste_world::World`).
pub trait WorldSink {
    /// `World::load_chunk`.
    fn load_chunk(&mut self, data: &ChunkData) -> anyhow::Result<()>;
    /// `World::unload_chunk`.
    fn unload_chunk(&mut self, pos: ChunkPos);
    /// `World::set_block`.
    fn set_block(&mut self, pos: BlockPos, state: BlockStateId);
    /// `World::apply_light`.
    fn apply_light(&mut self, pos: ChunkPos, light: &LightData);
    /// Drops every chunk and switches to `dim` (respawn / dimension change).
    fn reset(&mut self, dim: &DimensionInfo);
    /// Number of loaded chunks.
    fn chunk_count(&self) -> usize;
    /// Number of loaded sections with at least one non-air block.
    fn non_empty_sections(&self) -> usize;
}

/// What the driver needs from the entity store (`haste_entity::EntityStore`).
pub trait EntitySink {
    /// `EntityStore::spawn`.
    fn spawn(&mut self, s: &EntitySpawn);
    /// `EntityStore::remove`.
    fn remove(&mut self, ids: &[EntityId]);
    /// `EntityStore::move_delta`.
    fn move_delta(&mut self, id: EntityId, delta: DVec3, on_ground: bool);
    /// `EntityStore::set_rotation`.
    fn set_rotation(&mut self, id: EntityId, yaw: f32, pitch: f32, on_ground: bool);
    /// `EntityStore::teleport`.
    fn teleport(&mut self, id: EntityId, pos: DVec3, yaw: f32, pitch: f32, on_ground: bool);
    /// `EntityStore::set_head_yaw`.
    fn set_head_yaw(&mut self, id: EntityId, yaw: f32);
    /// `EntityStore::set_velocity`.
    fn set_velocity(&mut self, id: EntityId, v: DVec3);
    /// `EntityStore::set_metadata`.
    fn set_metadata(&mut self, id: EntityId, entries: &[MetadataEntry]);
    /// `EntityStore::set_equipment`.
    fn set_equipment(&mut self, id: EntityId, slot: u8, item: ItemStackData);
    /// `EntityStore::player_info`.
    fn player_info(&mut self, info: &[PlayerInfo]);
    /// Position and rotation of an entity.
    fn pose(&self, id: EntityId) -> Option<(DVec3, f32, f32)>;
    /// Forgets every entity (respawn / dimension change).
    fn clear(&mut self);
    /// `EntityStore::tick`.
    fn tick(&mut self);
    /// `EntityStore::len`.
    fn len(&self) -> usize;
}

impl WorldSink for World {
    fn load_chunk(&mut self, data: &ChunkData) -> anyhow::Result<()> {
        World::load_chunk(self, data).map_err(anyhow::Error::from)
    }
    fn unload_chunk(&mut self, pos: ChunkPos) {
        World::unload_chunk(self, pos)
    }
    fn set_block(&mut self, pos: BlockPos, state: BlockStateId) {
        World::set_block(self, pos, state)
    }
    fn apply_light(&mut self, pos: ChunkPos, light: &LightData) {
        World::apply_light(self, pos, light)
    }
    fn reset(&mut self, dim: &DimensionInfo) {
        let biomes = 1usize << self.biome_bits();
        *self = World::new_with_biomes(dim.clone(), self.game_data().clone(), biomes);
    }
    fn chunk_count(&self) -> usize {
        self.loaded_chunks().count()
    }
    fn non_empty_sections(&self) -> usize {
        let dim = self.dimension();
        let (min, count) = (dim.min_section(), dim.section_count() as i32);
        self.loaded_chunks()
            .filter_map(|p| self.chunk(p))
            .map(|c| {
                (min..min + count)
                    .filter(|y| c.section(*y).is_some_and(|s| !s.is_empty()))
                    .count()
            })
            .sum()
    }
}

impl EntitySink for EntityStore {
    fn spawn(&mut self, s: &EntitySpawn) {
        EntityStore::spawn(self, s)
    }
    fn remove(&mut self, ids: &[EntityId]) {
        EntityStore::remove(self, ids)
    }
    fn move_delta(&mut self, id: EntityId, delta: DVec3, on_ground: bool) {
        EntityStore::move_delta(self, id, delta, on_ground)
    }
    fn set_rotation(&mut self, id: EntityId, yaw: f32, pitch: f32, on_ground: bool) {
        EntityStore::set_rotation(self, id, yaw, pitch, on_ground)
    }
    fn teleport(&mut self, id: EntityId, pos: DVec3, yaw: f32, pitch: f32, on_ground: bool) {
        EntityStore::teleport(self, id, pos, yaw, pitch, on_ground)
    }
    fn set_head_yaw(&mut self, id: EntityId, yaw: f32) {
        EntityStore::set_head_yaw(self, id, yaw)
    }
    fn set_velocity(&mut self, id: EntityId, v: DVec3) {
        EntityStore::set_velocity(self, id, v)
    }
    fn set_metadata(&mut self, id: EntityId, entries: &[MetadataEntry]) {
        EntityStore::set_metadata(self, id, entries)
    }
    fn set_equipment(&mut self, id: EntityId, slot: u8, item: ItemStackData) {
        EntityStore::set_equipment(self, id, slot, item)
    }
    fn player_info(&mut self, info: &[PlayerInfo]) {
        EntityStore::player_info(self, info)
    }
    fn pose(&self, id: EntityId) -> Option<(DVec3, f32, f32)> {
        self.get(id).map(|e| (e.pos, e.yaw, e.pitch))
    }
    fn clear(&mut self) {
        // EntityStore has no clear(); a fresh store re-reads the (small, optional) dimensions
        // oracle. The tab list (`player_info`) is lost with it, which vanilla keeps across a
        // respawn: requested `EntityStore::clear()` that keeps players.
        *self = new_entity_store(0);
    }
    fn tick(&mut self) {
        EntityStore::tick(self)
    }
    fn len(&self) -> usize {
        EntityStore::len(self)
    }
}

/// The entity store (dimensions oracle when present).
pub fn new_entity_store(entity_types: usize) -> EntityStore {
    EntityStore::new(EntityDimensions::from_registry_len(entity_types))
}

/// Loads the game data (block states, registries, shapes) once for the whole client.
pub fn load_game_data() -> anyhow::Result<Arc<GameData>> {
    let t = std::time::Instant::now();
    let data = GameData::load().context(
        "loading game data from the data generator reports (run scripts/rebuild-cache.sh)",
    )?;
    tracing::info!(
        "game data: {} block states in {:.0} ms",
        data.blocks.len(),
        t.elapsed().as_secs_f64() * 1000.0
    );
    Ok(Arc::new(data))
}

/// Number of entries in the synced `minecraft:worldgen/biome` registry (the direct biome
/// palette's bit width depends on it).
pub fn synced_biome_count(regs: &SyncedRegistries) -> Option<usize> {
    regs.entries(&Identifier::minecraft("worldgen/biome"))
        .map(|e| e.len())
}

/// The local player as the server last told us.
#[derive(Clone, Debug)]
pub struct LocalPlayerState {
    /// Feet position.
    pub pos: DVec3,
    /// Velocity from the last teleport.
    pub velocity: DVec3,
    /// Yaw in degrees.
    pub yaw: f32,
    /// Pitch in degrees.
    pub pitch: f32,
    /// On ground (sent in movement packets).
    pub on_ground: bool,
    /// Health (20 = full).
    pub health: f32,
    /// Food level.
    pub food: i32,
    /// Selected hotbar slot.
    pub held_slot: i32,
    /// Game mode.
    pub game_mode: GameMode,
    /// Whether a position has been received (movement is sent only after).
    pub has_position: bool,
}

impl LocalPlayerState {
    fn new(game_mode: GameMode) -> Self {
        Self {
            pos: DVec3::ZERO,
            velocity: DVec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: true,
            health: 20.0,
            food: 20,
            held_slot: 0,
            game_mode,
            has_position: false,
        }
    }

    /// Applies a `player_position` teleport with its relative flags.
    pub fn apply_teleport(&mut self, p: &cb::PlayerPosition) {
        let rel = |bit: i32| p.flags & bit != 0;
        let pick = |bit: i32, old: f64, new: f64| if rel(bit) { old + new } else { new };
        let old_yaw = self.yaw;
        self.pos = DVec3::new(
            pick(teleport_flags::X, self.pos.x, p.pos.x),
            pick(teleport_flags::Y, self.pos.y, p.pos.y),
            pick(teleport_flags::Z, self.pos.z, p.pos.z),
        );
        self.yaw = if rel(teleport_flags::YAW) {
            self.yaw + p.yaw
        } else {
            p.yaw
        };
        self.pitch = (if rel(teleport_flags::PITCH) {
            self.pitch + p.pitch
        } else {
            p.pitch
        })
        .clamp(-90.0, 90.0);
        let mut v = self.velocity;
        if rel(teleport_flags::ROTATE_VELOCITY) {
            // Rotate the old velocity by the change in yaw (pitch rotation of velocity is
            // approximate here: not applied).
            let dyaw = (self.yaw - old_yaw).to_radians() as f64;
            let (s, c) = dyaw.sin_cos();
            v = DVec3::new(v.x * c - v.z * s, v.y, v.x * s + v.z * c);
        }
        self.velocity = DVec3::new(
            pick(teleport_flags::VEL_X, v.x, p.velocity.x),
            pick(teleport_flags::VEL_Y, v.y, p.velocity.y),
            pick(teleport_flags::VEL_Z, v.z, p.velocity.z),
        );
        self.has_position = true;
    }
}

/// Per-connection counters for the HUD/headless printout.
#[derive(Clone, Debug, Default)]
pub struct NetCounters {
    /// Play packets drained by the driver.
    pub packets: u64,
    /// Teleports accepted.
    pub teleports: u64,
    /// Chunks loaded.
    pub chunks_loaded: u64,
    /// Chunk unloads.
    pub chunks_unloaded: u64,
    /// Chunks the world refused to load.
    pub chunk_errors: u64,
    /// Last chunk error message.
    pub last_chunk_error: Option<String>,
    /// Whether the play `login` packet was seen.
    pub got_login: bool,
    /// Respawns / dimension changes.
    pub respawns: u64,
    /// Last chat/system lines (most recent last, at most 20).
    pub chat: Vec<String>,
}

/// Client-side session state the packets drive (everything except world and entities).
pub struct Session {
    /// The local player.
    pub player: LocalPlayerState,
    /// The local player's entity id (from `login`).
    pub entity_id: EntityId,
    /// The current dimension (from `login`, then each `respawn`).
    pub dimension: DimensionInfo,
    /// Counters.
    pub counters: NetCounters,
    sent_loaded: bool,
    last_input: Option<u8>,
}

impl Session {
    /// Fresh state after `Connection::connect` (from the `Login` it returned).
    pub fn new(entity_id: EntityId, game_mode: GameMode, dimension: DimensionInfo) -> Self {
        Self {
            player: LocalPlayerState::new(game_mode),
            counters: NetCounters {
                got_login: true,
                ..Default::default()
            },
            entity_id,
            dimension,
            sent_loaded: false,
            last_input: None,
        }
    }

    /// Applies one clientbound packet; serverbound replies go to `out`.
    pub fn apply<W: WorldSink + ?Sized, E: EntitySink + ?Sized>(
        &mut self,
        p: ClientboundPlay,
        world: &mut W,
        entities: &mut E,
        regs: &SyncedRegistries,
        out: &mut Vec<ServerboundPlay>,
    ) {
        use ClientboundPlay as P;
        self.counters.packets += 1;
        let own = self.entity_id;
        match p {
            P::Login(l) => {
                self.counters.got_login = true;
                self.player.game_mode =
                    GameMode::from_id(l.spawn.game_mode).unwrap_or(self.player.game_mode);
            }
            P::Respawn(r) => {
                self.counters.respawns += 1;
                let dim = regs
                    .dimension_info(r.spawn.dimension_type, &r.spawn.dimension)
                    .unwrap_or_else(|| DimensionInfo {
                        name: r.spawn.dimension.clone(),
                        ..DimensionInfo::overworld()
                    });
                world.reset(&dim);
                entities.clear();
                self.dimension = dim;
                self.player.game_mode =
                    GameMode::from_id(r.spawn.game_mode).unwrap_or(self.player.game_mode);
                self.player.has_position = false;
                self.sent_loaded = false;
            }
            P::PlayerPosition(t) => {
                tracing::debug!("teleport {t:?}");
                self.player.apply_teleport(&t);
                self.counters.teleports += 1;
                let pl = &self.player;
                out.push(ServerboundPlay::AcceptTeleportation(
                    sb::AcceptTeleportation {
                        teleport_id: t.teleport_id,
                        pos: pl.pos,
                        yaw: pl.yaw,
                        pitch: pl.pitch,
                    },
                ));
                // Observed on the 26.3 server: a movement packet sent between
                // accept_teleportation and player_loaded gets the client kicked
                // with invalid_player_movement, so movement starts on the next tick.
                if !self.sent_loaded {
                    out.push(ServerboundPlay::PlayerLoaded(sb::PlayerLoaded {}));
                    self.sent_loaded = true;
                }
            }
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
                world.unload_chunk(f.pos);
                self.counters.chunks_unloaded += 1;
            }
            P::BlockUpdate(b) => {
                let c = b.change();
                world.set_block(c.pos, c.state);
            }
            P::SectionBlocksUpdate(u) => {
                for c in u.changes() {
                    world.set_block(c.pos, c.state);
                }
            }
            P::LightUpdate(l) => world.apply_light(l.pos, &l.light),
            P::AddEntity(a) => entities.spawn(&a.spawn()),
            P::RemoveEntities(r) => {
                let ids: Vec<EntityId> = r.entity_ids.iter().map(|i| EntityId(*i)).collect();
                entities.remove(&ids);
            }
            P::MoveEntityPos(m) => {
                entities.move_delta(EntityId(m.entity_id), m.deltas.total(), m.on_ground)
            }
            P::MoveEntityPosRot(m) => {
                let id = EntityId(m.entity_id);
                entities.move_delta(id, m.deltas.total(), m.on_ground);
                entities.set_rotation(id, m.yaw, m.pitch, m.on_ground);
            }
            P::MoveEntityRot(m) => {
                entities.set_rotation(EntityId(m.entity_id), m.yaw, m.pitch, m.on_ground)
            }
            P::EntityPositionSync(s) => {
                if let Some(pos) = s.end_pos() {
                    entities.teleport(EntityId(s.entity_id), pos, s.yaw, s.pitch, s.on_ground);
                }
            }
            P::TeleportEntity(t) => {
                let id = EntityId(t.entity_id);
                let (old, oy, op) = entities.pose(id).unwrap_or((DVec3::ZERO, 0.0, 0.0));
                let rel = |bit: i32| t.flags & bit != 0;
                let pick = |bit: i32, old: f64, new: f64| if rel(bit) { old + new } else { new };
                let pos = DVec3::new(
                    pick(teleport_flags::X, old.x, t.pos.x),
                    pick(teleport_flags::Y, old.y, t.pos.y),
                    pick(teleport_flags::Z, old.z, t.pos.z),
                );
                let yaw = if rel(teleport_flags::YAW) {
                    oy + t.yaw
                } else {
                    t.yaw
                };
                let pitch = if rel(teleport_flags::PITCH) {
                    op + t.pitch
                } else {
                    t.pitch
                };
                entities.teleport(id, pos, yaw, pitch, t.on_ground);
            }
            P::RotateHead(h) => entities.set_head_yaw(EntityId(h.entity_id), h.head_yaw),
            P::SetEntityMotion(m) => {
                if EntityId(m.entity_id) == own {
                    self.player.velocity = m.velocity;
                } else {
                    entities.set_velocity(EntityId(m.entity_id), m.velocity);
                }
            }
            P::SetEntityData(d) => {
                if EntityId(d.entity_id) != own {
                    entities.set_metadata(EntityId(d.entity_id), &d.metadata);
                }
            }
            P::SetEquipment(e) => {
                for (slot, item) in e.equipment {
                    entities.set_equipment(EntityId(e.entity_id), slot, item);
                }
            }
            P::PlayerInfoUpdate(u) => entities.player_info(&u.infos()),
            P::SetHealth(h) => {
                self.player.health = h.health;
                self.player.food = h.food;
            }
            P::SetHeldSlot(s) => self.player.held_slot = s.slot,
            P::GameEvent(g) if g.event == 3 => {
                if let Some(m) = GameMode::from_id(g.value as i32) {
                    self.player.game_mode = m;
                }
            }
            P::SystemChat(c) if !c.overlay => self.push_chat(c.content.to_plain(&|_| None)),
            P::DisguisedChat(c) => self.push_chat(format!(
                "<{}> {}",
                c.sender_name.to_plain(&|_| None),
                c.message.to_plain(&|_| None)
            )),
            P::PlayerChat(c) => self.push_chat(format!(
                "<{}> {}",
                c.sender_name.to_plain(&|_| None),
                c.message
            )),
            P::Disconnect(d) => {
                tracing::warn!("server disconnected us: {}", d.reason.to_plain(&|_| None))
            }
            _ => {}
        }
    }

    /// The packets a vanilla client sends at the end of a tick while it stands still: the
    /// position/rotation, the (empty) player input when it changed, and `client_tick_end`.
    /// Nothing before the first teleport has been accepted and `player_loaded` sent.
    pub fn end_tick(&mut self, out: &mut Vec<ServerboundPlay>) {
        if !(self.player.has_position && self.sent_loaded) {
            return;
        }
        let pl = &self.player;
        let flags = if pl.on_ground {
            move_flags::ON_GROUND
        } else {
            0
        };
        out.push(ServerboundPlay::MovePlayerPosRot(sb::MovePlayerPosRot {
            pos: pl.pos,
            yaw: pl.yaw,
            pitch: pl.pitch,
            flags,
        }));
        if self.last_input != Some(0) {
            out.push(ServerboundPlay::PlayerInput(sb::PlayerInput { flags: 0 }));
            self.last_input = Some(0);
        }
        out.push(ServerboundPlay::ClientTickEnd(sb::ClientTickEnd {}));
    }

    fn push_chat(&mut self, line: String) {
        tracing::info!("chat: {line}");
        self.counters.chat.push(line);
        if self.counters.chat.len() > 20 {
            self.counters.chat.remove(0);
        }
    }
}

/// Owns the connection and the session state; world and entities are borrowed per tick.
pub struct NetDriver {
    conn: Connection,
    login: Login,
    /// Session state.
    pub session: Session,
    out: Vec<ServerboundPlay>,
}

impl NetDriver {
    /// Connects (handshake, login, configuration) and returns in the play state.
    pub async fn connect(opts: ConnectOptions) -> anyhow::Result<Self> {
        let (conn, login) = Connection::connect(opts).await.context("connecting")?;
        let session = Session::new(login.entity_id, login.game_mode, login.dimension.clone());
        Ok(Self {
            conn,
            login,
            session,
            out: Vec::new(),
        })
    }

    /// The connection (registries, stats, close reason).
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// What `login` said (the dimension may since have changed: `session.dimension`).
    pub fn login(&self) -> &Login {
        &self.login
    }

    /// Connection counters.
    pub fn stats(&self) -> StatsSnapshot {
        self.conn.stats()
    }

    /// Whether the connection has closed.
    pub fn is_closed(&self) -> bool {
        self.conn.is_closed()
    }

    /// Closes the connection.
    pub fn disconnect(self) {
        self.conn.disconnect();
    }

    /// Drains every queued packet into the session, world and entities, and sends the replies.
    pub fn receive<W: WorldSink + ?Sized, E: EntitySink + ?Sized>(
        &mut self,
        world: &mut W,
        entities: &mut E,
    ) -> anyhow::Result<()> {
        while let Some(p) = self.conn.try_recv() {
            let regs = self.conn.registries();
            self.session.apply(p, world, entities, regs, &mut self.out);
        }
        self.flush()
    }

    /// Ends the tick: entity interpolation step and this tick's serverbound packets.
    pub fn end_tick<E: EntitySink + ?Sized>(&mut self, entities: &mut E) -> anyhow::Result<()> {
        entities.tick();
        self.session.end_tick(&mut self.out);
        self.flush()
    }

    /// One 20 Hz tick: [`Self::receive`] then [`Self::end_tick`].
    pub fn tick<W: WorldSink + ?Sized, E: EntitySink + ?Sized>(
        &mut self,
        world: &mut W,
        entities: &mut E,
    ) -> anyhow::Result<()> {
        self.receive(world, entities)?;
        self.end_tick(entities)
    }

    fn flush(&mut self) -> anyhow::Result<()> {
        for p in self.out.drain(..) {
            self.conn
                .send(p)
                .map_err(|e| anyhow::anyhow!("send: {e}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
