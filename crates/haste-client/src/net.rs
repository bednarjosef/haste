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
use haste_core::input::InputState;
use haste_core::view::{BlockView, CollisionShapes};
use haste_data::GameData;
use haste_entity::{EntityDimensions, EntityStore};
use haste_protocol::haste_core::chunk::{ChunkData, LightData};
use haste_protocol::haste_core::entity::{EntitySpawn, ItemStackData, MetadataEntry, PlayerInfo};
use haste_protocol::haste_core::game::{DimensionInfo, GameMode};
use haste_protocol::haste_core::{BlockPos, BlockStateId, ChunkPos, EntityId, Identifier};
#[cfg(test)]
use haste_protocol::packets::play::clientbound as cb;
use haste_protocol::packets::play::clientbound::teleport_flags;
use haste_protocol::{
    ClientboundPlay, ConnectOptions, Connection, Login, ServerboundPlay, StatsSnapshot,
    SyncedRegistries,
};
use haste_world::World;

use crate::player::{AttributeIds, LocalPlayer};

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
    /// What the local player's physics reads; `None` (test fakes) freezes the player in place.
    fn physics(&self) -> Option<(&dyn BlockView, &GameData)> {
        None
    }
}

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
    /// Position and rotation of an entity.
    fn pose(&self, id: EntityId) -> Option<(DVec3, f32, f32)>;
    /// Forgets every entity (respawn / dimension change).
    fn clear(&mut self);
    /// `EntityStore::tick`.
    fn tick(&mut self);
    /// `EntityStore::len`.
    fn len(&self) -> usize;
}

/// TEMPORARY shim (player workstream; delete when haste-world is fixed): `haste_world` reads
/// packed palette entries from the most significant bits of each long, vanilla packs entry 0
/// into the least significant bits. Seen on the vanilla server: a 3x3 stone pad at x 20..22 was
/// decoded at the mirrored x positions within each 16-entry row, so the player fell through it
/// and the server corrected every other tick. Rewrites the block containers' longs into the
/// order `haste_world` reads (biome containers are copied unchanged).
fn mirror_block_entries(data: &[u8], section_count: usize) -> Option<Vec<u8>> {
    use haste_protocol::wire::{Reader, Writer};
    let mut r = Reader::new(data);
    let mut out = Writer::new();
    let skip_varint = |r: &mut Reader<'_>, out: &mut Writer| -> Option<i32> {
        let v = r.var_int().ok()?;
        out.var_int(v);
        Some(v)
    };
    for _ in 0..section_count {
        out.bytes(r.take(4).ok()?); // i16 block count, i16 fluid count
        let bpe = r.u8().ok()?;
        out.u8(bpe);
        if bpe == 0 {
            skip_varint(&mut r, &mut out)?;
        } else {
            if bpe <= 8 {
                let n = skip_varint(&mut r, &mut out)?;
                for _ in 0..n {
                    skip_varint(&mut r, &mut out)?;
                }
            }
            let bits = bpe as usize;
            let per = 64 / bits;
            let mask = (1u64 << bits) - 1;
            for _ in 0..4096usize.div_ceil(per) {
                let l = u64::from_be_bytes(r.take(8).ok()?.try_into().ok()?);
                let mut m = 0u64;
                for k in 0..per {
                    m |= ((l >> (k * bits)) & mask) << (64 - (k + 1) * bits);
                }
                out.bytes(&m.to_be_bytes());
            }
        }
        // Biome container: bits, then a single value, or a palette, then the longs.
        let bpe = r.u8().ok()?;
        out.u8(bpe);
        if bpe == 0 {
            skip_varint(&mut r, &mut out)?;
        } else {
            if bpe <= 3 {
                let n = skip_varint(&mut r, &mut out)?;
                for _ in 0..n {
                    skip_varint(&mut r, &mut out)?;
                }
            }
            out.bytes(r.take(64usize.div_ceil(64 / bpe as usize) * 8).ok()?);
        }
    }
    out.bytes(r.take(r.remaining()).ok()?);
    Some(out.into_inner())
}

impl WorldSink for World {
    fn load_chunk(&mut self, data: &ChunkData) -> anyhow::Result<()> {
        let mut fixed = data.clone();
        fixed.sections = mirror_block_entries(&data.sections, self.dimension().section_count())
            .ok_or_else(|| anyhow::anyhow!("section payload does not parse"))?
            .into();
        World::load_chunk(self, &fixed).map_err(anyhow::Error::from)
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
    fn physics(&self) -> Option<(&dyn BlockView, &GameData)> {
        Some((self, self.game_data()))
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
    /// The local player (player.rs): its packets in, its movement packets out.
    pub player: LocalPlayer,
    /// The local player's entity id (from `login`).
    pub entity_id: EntityId,
    /// The current dimension (from `login`, then each `respawn`).
    pub dimension: DimensionInfo,
    /// Counters.
    pub counters: NetCounters,
}

impl Session {
    /// Fresh state after `Connection::connect` (from the `Login` it returned).
    pub fn new(entity_id: EntityId, game_mode: GameMode, dimension: DimensionInfo) -> Self {
        Self {
            player: LocalPlayer::with_entity(entity_id, game_mode),
            counters: NetCounters {
                got_login: true,
                ..Default::default()
            },
            entity_id,
            dimension,
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
        let player_packet = self.player.handle(&p, out);
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
            }
            P::PlayerPosition(_) => self.counters.teleports += 1,
            _ if player_packet => {}
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
            P::SetEntityMotion(m) => entities.set_velocity(EntityId(m.entity_id), m.velocity),
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

    /// The local player's tick with `input` and its packets (`LocalPlayer::tick`); in a world
    /// without physics data (test fakes) the player stays in place.
    pub fn end_tick<W: WorldSink + ?Sized>(
        &mut self,
        input: &InputState,
        world: &W,
        out: &mut Vec<ServerboundPlay>,
    ) {
        match world.physics() {
            Some((view, data)) => {
                if !self.player.has_attribute_ids() {
                    if let Some(ids) = AttributeIds::from_data(data) {
                        self.player.set_attribute_ids(ids);
                    }
                }
                self.player.tick_with(input, view, &data.shapes, data, out)
            }
            None => self.player.tick_with(
                input,
                &NoBlocks,
                &NoBlocks,
                &haste_physics::DefaultBlockPhysics,
                out,
            ),
        }
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

    /// Ends the tick: entity interpolation step, the local player's tick with `input` and this
    /// tick's serverbound packets.
    pub fn end_tick<W: WorldSink + ?Sized, E: EntitySink + ?Sized>(
        &mut self,
        input: &InputState,
        world: &W,
        entities: &mut E,
    ) -> anyhow::Result<()> {
        entities.tick();
        self.session.end_tick(input, world, &mut self.out);
        self.flush()
    }

    /// Queues a serverbound packet; it goes out with this tick's packets.
    pub fn queue(&mut self, p: ServerboundPlay) {
        self.out.push(p);
    }

    /// One 20 Hz tick: [`Self::receive`] then [`Self::end_tick`].
    pub fn tick<W: WorldSink + ?Sized, E: EntitySink + ?Sized>(
        &mut self,
        input: &InputState,
        world: &mut W,
        entities: &mut E,
    ) -> anyhow::Result<()> {
        self.receive(world, entities)?;
        self.end_tick(input, world, entities)
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
