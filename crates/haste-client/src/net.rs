//! Network driver (protocol): owns the `Connection`, drains packets each tick and
//! applies them to the world, the entity store and the local player, and
//! sends the per-tick movement packets.
//!
//! `haste_world::World` and `haste_entity::EntityStore` are not on this
//! branch yet, so the driver talks to them through the [`WorldSink`] and
//! [`EntitySink`] traits, whose methods mirror docs/INTERFACES.md one to one.
//! [`StubWorld`] and [`StubEntities`] are minimal stand-ins; at merge the
//! lead implements the two traits for the real types (each method is a
//! one-line forward) and drops the stubs.

use std::collections::{HashMap, HashSet};

use anyhow::Context;
use glam::DVec3;
use haste_core::input::InputState;
use haste_core::view::{BlockPhysics, BlockView, CollisionShapes};
use haste_protocol::haste_core::chunk::{ChunkData, LightData};
use haste_protocol::haste_core::entity::{EntitySpawn, ItemStackData, MetadataEntry, PlayerInfo};
use haste_protocol::haste_core::game::{DimensionInfo, GameMode};
use haste_protocol::haste_core::{BlockPos, BlockStateId, ChunkPos, EntityId};
use haste_protocol::packets::play::clientbound::teleport_flags;
use haste_protocol::{
    ClientboundPlay, ConnectOptions, Connection, Login, ServerboundPlay, StatsSnapshot,
};

use crate::player::LocalPlayer;

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
    /// Number of loaded chunks (`World::loaded_chunks().count()`).
    fn chunk_count(&self) -> usize;
    /// Number of loaded sections with at least one non-air block.
    fn non_empty_sections(&self) -> usize;
    /// What the player's physics reads (blocks, shapes, block properties); `None` for a
    /// world without block data, where the player stays frozen in place.
    fn physics_view(&self) -> Option<PhysicsView<'_>> {
        None
    }
}

/// The three views the player's physics tick reads.
pub type PhysicsView<'a> = (
    &'a dyn BlockView,
    &'a dyn CollisionShapes,
    &'a dyn BlockPhysics,
);

/// A world with no chunks: the player does not move in it.
struct NoBlocks;
impl BlockView for NoBlocks {
    fn block_state(&self, _pos: BlockPos) -> BlockStateId {
        BlockStateId::AIR
    }
    fn is_chunk_loaded(&self, _pos: ChunkPos) -> bool {
        false
    }
    fn sky_light(&self, _pos: BlockPos) -> u8 {
        15
    }
    fn block_light(&self, _pos: BlockPos) -> u8 {
        0
    }
    fn min_y(&self) -> i32 {
        0
    }
    fn height(&self) -> i32 {
        0
    }
}
impl CollisionShapes for NoBlocks {
    fn collision_boxes(&self, _state: BlockStateId) -> &[haste_core::Aabb] {
        &[]
    }
    fn outline_boxes(&self, _state: BlockStateId) -> &[haste_core::Aabb] {
        &[]
    }
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
    /// Position and rotation of an entity (`EntityStore::get(id).map(|e| (e.pos, e.yaw, e.pitch))`).
    fn pose(&self, id: EntityId) -> Option<(DVec3, f32, f32)>;
    /// `EntityStore::tick`.
    fn tick(&mut self);
    /// `EntityStore::len`.
    fn len(&self) -> usize;
}

/// Per-connection counters for the HUD/headless printout.
#[derive(Clone, Debug, Default)]
pub struct NetCounters {
    /// Play packets drained by the driver.
    pub packets: u64,
    /// Teleports accepted.
    pub teleports: u64,
    /// Chunks loaded / unloaded.
    pub chunks_loaded: u64,
    /// Chunk unloads.
    pub chunks_unloaded: u64,
    /// Chunks the world refused to load.
    pub chunk_errors: u64,
    /// Whether the play `login` packet was seen.
    pub got_login: bool,
    /// Respawns / dimension changes.
    pub respawns: u64,
    /// Last chat/system lines (most recent last, at most 20).
    pub chat: Vec<String>,
}

/// Owns the connection and the client-side game state it feeds.
pub struct NetDriver<W: WorldSink, E: EntitySink> {
    conn: Connection,
    /// The world.
    pub world: W,
    /// The entities.
    pub entities: E,
    /// The local player.
    pub player: LocalPlayer,
    /// What `login` said.
    pub login: Login,
    /// Counters.
    pub counters: NetCounters,
    make_world: Box<dyn FnMut(&DimensionInfo) -> W + Send>,
    outgoing: Vec<ServerboundPlay>,
    ticks: u64,
}

impl<W: WorldSink, E: EntitySink> NetDriver<W, E> {
    /// Connects and builds the world for the login dimension with `make_world`.
    pub async fn connect(
        opts: ConnectOptions,
        mut make_world: Box<dyn FnMut(&DimensionInfo) -> W + Send>,
        entities: E,
    ) -> anyhow::Result<Self> {
        let (conn, login) = Connection::connect(opts).await.context("connecting")?;
        let world = make_world(&login.dimension);
        Ok(Self {
            conn,
            world,
            entities,
            player: LocalPlayer::new(&login),
            counters: NetCounters {
                got_login: true,
                ..Default::default()
            },
            login,
            make_world,
            outgoing: Vec::new(),
            ticks: 0,
        })
    }

    /// The connection (registries, stats, close reason).
    pub fn connection(&self) -> &Connection {
        &self.conn
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

    /// One 20 Hz tick with no movement input.
    pub fn tick(&mut self) -> anyhow::Result<()> {
        self.tick_input(&InputState::default())
    }

    /// One 20 Hz tick: drain and apply every queued packet, then the local
    /// player's tick with `input` and its packets.
    pub fn tick_input(&mut self, input: &InputState) -> anyhow::Result<()> {
        while let Some(p) = self.conn.try_recv() {
            self.counters.packets += 1;
            self.apply(p)?;
        }
        self.entities.tick();
        self.ticks += 1;
        match self.world.physics_view() {
            Some((w, s, b)) => self.player.tick_with(input, w, s, b, &mut self.outgoing),
            None => self.player.tick_with(
                input,
                &NoBlocks,
                &NoBlocks,
                &haste_physics::DefaultBlockPhysics,
                &mut self.outgoing,
            ),
        }
        self.flush()
    }

    fn flush(&mut self) -> anyhow::Result<()> {
        for p in self.outgoing.drain(..) {
            self.conn
                .send(p)
                .map_err(|e| anyhow::anyhow!("send: {e}"))?;
        }
        Ok(())
    }

    fn apply(&mut self, p: ClientboundPlay) -> anyhow::Result<()> {
        use ClientboundPlay as P;
        let own = self.login.entity_id;
        if self.player.handle(&p, &mut self.outgoing) {
            if matches!(p, P::PlayerPosition(_)) {
                self.counters.teleports += 1;
            }
            if let P::Respawn(r) = &p {
                self.respawn(r);
            }
            return self.flush();
        }
        match p {
            P::Login(l) => {
                self.counters.got_login = true;
                if let Some(m) = GameMode::from_id(l.spawn.game_mode) {
                    self.player.game_mode = m;
                }
            }
            P::SetEntityMotion(m) => self
                .entities
                .set_velocity(EntityId(m.entity_id), m.velocity),
            P::LevelChunkWithLight(c) => match self.world.load_chunk(&c.chunk) {
                Ok(()) => self.counters.chunks_loaded += 1,
                Err(e) => {
                    self.counters.chunk_errors += 1;
                    if self.counters.chunk_errors <= 3 {
                        tracing::warn!("chunk {:?} not loaded: {e:#}", c.chunk.pos);
                    }
                }
            },
            P::ForgetLevelChunk(f) => {
                self.world.unload_chunk(f.pos);
                self.counters.chunks_unloaded += 1;
            }
            P::BlockUpdate(b) => {
                let c = b.change();
                self.world.set_block(c.pos, c.state);
            }
            P::SectionBlocksUpdate(u) => {
                for c in u.changes() {
                    self.world.set_block(c.pos, c.state);
                }
            }
            P::LightUpdate(l) => self.world.apply_light(l.pos, &l.light),
            P::AddEntity(a) => self.entities.spawn(&a.spawn()),
            P::RemoveEntities(r) => {
                let ids: Vec<EntityId> = r.entity_ids.iter().map(|i| EntityId(*i)).collect();
                self.entities.remove(&ids);
            }
            P::MoveEntityPos(m) => {
                self.entities
                    .move_delta(EntityId(m.entity_id), m.deltas.total(), m.on_ground)
            }
            P::MoveEntityPosRot(m) => {
                let id = EntityId(m.entity_id);
                self.entities.move_delta(id, m.deltas.total(), m.on_ground);
                self.entities.set_rotation(id, m.yaw, m.pitch, m.on_ground);
            }
            P::MoveEntityRot(m) => {
                self.entities
                    .set_rotation(EntityId(m.entity_id), m.yaw, m.pitch, m.on_ground)
            }
            P::EntityPositionSync(s) => {
                if let Some(pos) = s.end_pos() {
                    self.entities
                        .teleport(EntityId(s.entity_id), pos, s.yaw, s.pitch, s.on_ground);
                }
            }
            P::TeleportEntity(t) => {
                let id = EntityId(t.entity_id);
                let (old, oy, op) = self.entities.pose(id).unwrap_or((DVec3::ZERO, 0.0, 0.0));
                let rel = |bit: i32| t.flags & bit != 0;
                let pos = DVec3::new(
                    if rel(teleport_flags::X) {
                        old.x + t.pos.x
                    } else {
                        t.pos.x
                    },
                    if rel(teleport_flags::Y) {
                        old.y + t.pos.y
                    } else {
                        t.pos.y
                    },
                    if rel(teleport_flags::Z) {
                        old.z + t.pos.z
                    } else {
                        t.pos.z
                    },
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
                self.entities.teleport(id, pos, yaw, pitch, t.on_ground);
            }
            P::RotateHead(h) => self
                .entities
                .set_head_yaw(EntityId(h.entity_id), h.head_yaw),
            P::SetEntityData(d) => {
                if EntityId(d.entity_id) != own {
                    self.entities
                        .set_metadata(EntityId(d.entity_id), &d.metadata);
                }
            }
            P::SetEquipment(e) => {
                for (slot, item) in e.equipment {
                    self.entities
                        .set_equipment(EntityId(e.entity_id), slot, item);
                }
            }
            P::PlayerInfoUpdate(u) => self.entities.player_info(&u.infos()),
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
        Ok(())
    }

    fn respawn(&mut self, r: &haste_protocol::packets::play::clientbound::Respawn) {
        self.counters.respawns += 1;
        let dim = self
            .conn
            .registries()
            .dimension_info(r.spawn.dimension_type, &r.spawn.dimension)
            .unwrap_or_else(|| DimensionInfo {
                name: r.spawn.dimension.clone(),
                ..DimensionInfo::overworld()
            });
        self.world = (self.make_world)(&dim);
        self.login.dimension = dim;
    }

    fn push_chat(&mut self, line: String) {
        tracing::info!("chat: {line}");
        self.counters.chat.push(line);
        if self.counters.chat.len() > 20 {
            self.counters.chat.remove(0);
        }
    }
}

// ------------------------------------------------------------ stand-ins

/// Minimal world stand-in: tracks loaded chunks and, per chunk, how many
/// sections have a non-zero block count (read from the section payload;
/// the full palette decode is `haste_world`'s job).
#[derive(Debug, Default)]
pub struct StubWorld {
    chunks: HashMap<ChunkPos, usize>,
    section_count: usize,
    /// Bits per entry of the direct biome palette (from the biome registry size).
    biome_direct_bits: u8,
}

impl StubWorld {
    /// A stub for a dimension; `biome_count` is the synced biome registry size.
    pub fn new(dim: &DimensionInfo, biome_count: usize) -> Self {
        let bits = (usize::BITS - biome_count.saturating_sub(1).leading_zeros()) as u8;
        Self {
            chunks: HashMap::new(),
            section_count: dim.section_count(),
            biome_direct_bits: bits.max(1),
        }
    }

    /// Counts sections with a non-zero block count. Approximation for the
    /// stand-in: a direct block palette is assumed to use 15 bits (the
    /// wiki's vanilla value); `None` if the payload does not parse.
    fn count_non_empty(&self, data: &[u8]) -> Option<usize> {
        let mut r = haste_protocol::wire::Reader::new(data);
        let mut n = 0;
        for _ in 0..self.section_count {
            let blocks = r.i16().ok()?;
            let _fluids = r.i16().ok()?;
            skip_container(&mut r, 4096, 4, 15)?;
            skip_container(&mut r, 64, 1, self.biome_direct_bits)?;
            if blocks > 0 {
                n += 1;
            }
        }
        Some(n)
    }
}

/// Skips a paletted container (wiki "Paletted Container structure"; the data
/// array is not length-prefixed, its size follows from the bits per entry).
fn skip_container(
    r: &mut haste_protocol::wire::Reader<'_>,
    entries: usize,
    min_indirect: u8,
    direct_bits: u8,
) -> Option<()> {
    let bits = r.u8().ok()?;
    let bits = if bits == 0 {
        r.var_int().ok()?;
        0
    } else if (entries == 4096 && bits <= 8) || (entries == 64 && bits <= 3) {
        let n = r.len_prefix(4096).ok()?;
        for _ in 0..n {
            r.var_int().ok()?;
        }
        bits.max(min_indirect)
    } else {
        direct_bits
    };
    if bits > 0 {
        let per_long = 64 / bits as usize;
        r.take(entries.div_ceil(per_long) * 8).ok()?;
    }
    Some(())
}

impl WorldSink for StubWorld {
    fn load_chunk(&mut self, data: &ChunkData) -> anyhow::Result<()> {
        let n = self
            .count_non_empty(&data.sections)
            .ok_or_else(|| anyhow::anyhow!("stub could not walk the section payload"))?;
        self.chunks.insert(data.pos, n);
        Ok(())
    }
    fn unload_chunk(&mut self, pos: ChunkPos) {
        self.chunks.remove(&pos);
    }
    fn set_block(&mut self, _pos: BlockPos, _state: BlockStateId) {}
    fn apply_light(&mut self, _pos: ChunkPos, _light: &LightData) {}
    fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
    fn non_empty_sections(&self) -> usize {
        self.chunks.values().sum()
    }
}

/// Minimal entity store stand-in: positions only.
#[derive(Debug, Default)]
pub struct StubEntities {
    entities: HashMap<EntityId, (DVec3, f32, f32)>,
    players: HashSet<u128>,
}

impl EntitySink for StubEntities {
    fn spawn(&mut self, s: &EntitySpawn) {
        self.entities.insert(s.id, (s.pos, s.yaw, s.pitch));
    }
    fn remove(&mut self, ids: &[EntityId]) {
        for id in ids {
            self.entities.remove(id);
        }
    }
    fn move_delta(&mut self, id: EntityId, delta: DVec3, _on_ground: bool) {
        if let Some(e) = self.entities.get_mut(&id) {
            e.0 += delta;
        }
    }
    fn set_rotation(&mut self, id: EntityId, yaw: f32, pitch: f32, _on_ground: bool) {
        if let Some(e) = self.entities.get_mut(&id) {
            e.1 = yaw;
            e.2 = pitch;
        }
    }
    fn teleport(&mut self, id: EntityId, pos: DVec3, yaw: f32, pitch: f32, _on_ground: bool) {
        if let Some(e) = self.entities.get_mut(&id) {
            *e = (pos, yaw, pitch);
        }
    }
    fn set_head_yaw(&mut self, _id: EntityId, _yaw: f32) {}
    fn set_velocity(&mut self, _id: EntityId, _v: DVec3) {}
    fn set_metadata(&mut self, _id: EntityId, _entries: &[MetadataEntry]) {}
    fn set_equipment(&mut self, _id: EntityId, _slot: u8, _item: ItemStackData) {}
    fn player_info(&mut self, info: &[PlayerInfo]) {
        for i in info {
            if !i.name.is_empty() {
                self.players.insert(i.uuid.as_u128());
            }
        }
    }
    fn pose(&self, id: EntityId) -> Option<(DVec3, f32, f32)> {
        self.entities.get(&id).copied()
    }
    fn tick(&mut self) {}
    fn len(&self) -> usize {
        self.entities.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_world_counts_sections() {
        let dim = DimensionInfo {
            min_y: 0,
            height: 32,
            ..DimensionInfo::overworld()
        };
        let w = StubWorld::new(&dim, 64);
        // Section 1: 0 blocks, single-valued air, single-valued biome.
        // Section 2: 10 blocks, indirect 4-bit palette of 2, single biome.
        let mut data = vec![0, 0, 0, 0, 0, 0, 0, 0];
        data.extend([0, 10, 0, 0, 4, 2, 0, 1]);
        data.extend(vec![0u8; 256 * 8]);
        data.extend([0, 0]);
        assert_eq!(w.count_non_empty(&data), Some(1));
        assert_eq!(w.count_non_empty(&data[..10]), None);
    }
}
