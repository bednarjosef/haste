//! Driver tests: packets are forwarded to the world/entity sinks, the player state and the
//! serverbound replies follow the vanilla order. Recording fakes, no network.

use std::collections::HashMap;

use super::*;
use haste_protocol::nbt::{Nbt, NbtCompound};
use haste_protocol::packets::configuration::clientbound::RegistryEntry;
use haste_protocol::packets::play::clientbound::{MoveDeltas, SpawnInfo};
use haste_protocol::wire::particle::Particle;

#[derive(Default)]
struct FakeWorld {
    log: Vec<String>,
    chunks: Vec<ChunkPos>,
}

impl WorldSink for FakeWorld {
    fn load_chunk(&mut self, data: &ChunkData) -> anyhow::Result<()> {
        if data.pos.x == 99 {
            anyhow::bail!("bad chunk");
        }
        self.chunks.push(data.pos);
        self.log.push(format!("load {} {}", data.pos.x, data.pos.z));
        Ok(())
    }
    fn unload_chunk(&mut self, pos: ChunkPos) {
        self.chunks.retain(|p| *p != pos);
        self.log.push(format!("unload {} {}", pos.x, pos.z));
    }
    fn set_block(&mut self, pos: BlockPos, state: BlockStateId) {
        self.log
            .push(format!("set {} {} {} {}", pos.x, pos.y, pos.z, state.0));
    }
    fn apply_light(&mut self, pos: ChunkPos, _light: &LightData) {
        self.log.push(format!("light {} {}", pos.x, pos.z));
    }
    fn set_block_entity(&mut self, pos: BlockPos, data: BlockEntityData) {
        self.log.push(format!(
            "be {} {} {} {} {:?}",
            pos.x, pos.y, pos.z, data.kind, data.nbt
        ));
    }
    fn reset(&mut self, dim: &DimensionInfo) {
        self.chunks.clear();
        self.log.push(format!("reset {}", dim.name));
    }
    fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
    fn non_empty_sections(&self) -> usize {
        0
    }
}

#[derive(Default)]
struct FakeEntities {
    poses: HashMap<EntityId, (DVec3, f32, f32)>,
    log: Vec<String>,
    ticks: u32,
}

impl EntitySink for FakeEntities {
    fn spawn(&mut self, s: &EntitySpawn) {
        self.poses.insert(s.id, (s.pos, s.yaw, s.pitch));
    }
    fn remove(&mut self, ids: &[EntityId]) {
        for id in ids {
            self.poses.remove(id);
        }
    }
    fn move_delta(&mut self, id: EntityId, delta: DVec3, _on_ground: bool) {
        if let Some(p) = self.poses.get_mut(&id) {
            p.0 += delta;
        }
    }
    fn set_rotation(&mut self, id: EntityId, yaw: f32, pitch: f32, _on_ground: bool) {
        if let Some(p) = self.poses.get_mut(&id) {
            p.1 = yaw;
            p.2 = pitch;
        }
    }
    fn teleport(&mut self, id: EntityId, pos: DVec3, yaw: f32, pitch: f32, _on_ground: bool) {
        self.poses.insert(id, (pos, yaw, pitch));
    }
    fn set_head_yaw(&mut self, id: EntityId, yaw: f32) {
        self.log.push(format!("head {} {yaw}", id.0));
    }
    fn set_velocity(&mut self, id: EntityId, v: DVec3) {
        self.log.push(format!("velocity {} {}", id.0, v.y));
    }
    fn mark_hurt(&mut self, id: EntityId) {
        self.log.push(format!("hurt {}", id.0));
    }
    fn mark_dead(&mut self, id: EntityId) {
        self.log.push(format!("dead {}", id.0));
    }
    fn set_metadata(&mut self, id: EntityId, entries: &[MetadataEntry]) {
        self.log
            .push(format!("metadata {} {}", id.0, entries.len()));
    }
    fn set_equipment(&mut self, id: EntityId, slot: u8, _item: ItemStackData) {
        self.log.push(format!("equipment {} {slot}", id.0));
    }
    fn player_info(&mut self, info: &[PlayerInfo]) {
        self.log.push(format!("player_info {}", info.len()));
    }
    fn pose(&self, id: EntityId) -> Option<(DVec3, f32, f32)> {
        self.poses.get(&id).copied()
    }
    fn clear(&mut self) {
        self.poses.clear();
        self.log.push("clear".into());
    }
    fn tick(&mut self) {
        self.ticks += 1;
    }
    fn len(&self) -> usize {
        self.poses.len()
    }
}

struct Rig {
    session: Session,
    world: FakeWorld,
    entities: FakeEntities,
    regs: SyncedRegistries,
    out: Vec<ServerboundPlay>,
}

impl Rig {
    fn new() -> Self {
        Self {
            session: Session::new(EntityId(7), GameMode::Creative, DimensionInfo::overworld()),
            world: FakeWorld::default(),
            entities: FakeEntities::default(),
            regs: SyncedRegistries::default(),
            out: Vec::new(),
        }
    }
    fn apply(&mut self, p: ClientboundPlay) {
        self.session.apply(
            p,
            &mut self.world,
            &mut self.entities,
            &self.regs,
            &mut self.out,
        );
    }
    fn names(&mut self) -> Vec<&'static str> {
        self.out
            .drain(..)
            .map(|p| match p {
                ServerboundPlay::AcceptTeleportation(_) => "accept_teleportation",
                ServerboundPlay::PlayerLoaded(_) => "player_loaded",
                ServerboundPlay::MovePlayerPosRot(_) => "move_player_pos_rot",
                ServerboundPlay::PlayerInput(_) => "player_input",
                ServerboundPlay::ClientTickEnd(_) => "client_tick_end",
                _ => "other",
            })
            .collect()
    }
}

fn chunk(x: i32, z: i32) -> ChunkData {
    ChunkData {
        pos: ChunkPos::new(x, z),
        heightmaps: Vec::new(),
        sections: Default::default(),
        block_entities: Vec::new(),
        light: Default::default(),
    }
}

/// Network NBT (unnamed root compound with one string), as the protocol crate's tests build it.
fn nbt() -> Nbt {
    let mut c = NbtCompound::new();
    c.put_string("color", "red".into());
    Nbt::new(String::new(), c)
}

fn level_chunk(x: i32, z: i32) -> ClientboundPlay {
    ClientboundPlay::LevelChunkWithLight(cb::LevelChunkWithLight { chunk: chunk(x, z) })
}

fn teleport(id: i32, pos: DVec3, flags: i32) -> ClientboundPlay {
    ClientboundPlay::PlayerPosition(cb::PlayerPosition {
        teleport_id: id,
        pos,
        velocity: DVec3::ZERO,
        yaw: 90.0,
        pitch: 10.0,
        flags,
    })
}

fn spawn_packet(id: i32, pos: DVec3) -> ClientboundPlay {
    ClientboundPlay::AddEntity(cb::AddEntity {
        entity_id: id,
        uuid: Default::default(),
        kind: 3,
        pos,
        velocity: DVec3::ZERO,
        pitch: 0.0,
        yaw: 0.0,
        head_yaw: 0.0,
        data: 0,
    })
}

#[test]
fn chunk_and_block_packets_reach_the_world() {
    let mut r = Rig::new();
    r.apply(level_chunk(1, 2));
    r.apply(level_chunk(99, 0));
    r.apply(ClientboundPlay::BlockUpdate(cb::BlockUpdate {
        pos: BlockPos::new(17, 64, 33),
        state: 5,
    }));
    r.apply(ClientboundPlay::ForgetLevelChunk(cb::ForgetLevelChunk {
        pos: ChunkPos::new(1, 2),
    }));
    assert_eq!(r.world.log, ["load 1 2", "set 17 64 33 5", "unload 1 2"]);
    let c = &r.session.counters;
    assert_eq!(
        (c.chunks_loaded, c.chunk_errors, c.chunks_unloaded),
        (1, 1, 1)
    );
    assert_eq!(c.last_chunk_error.as_deref(), Some("bad chunk"));
    assert_eq!(r.world.chunk_count(), 0);
}

#[test]
fn block_entity_data_reaches_the_world() {
    let mut r = Rig::new();
    let nbt = nbt().write_unnamed();
    r.apply(ClientboundPlay::BlockEntityData(cb::BlockEntityData {
        pos: BlockPos::new(17, 64, 33),
        kind: 7,
        nbt: nbt.clone(),
    }));
    assert_eq!(r.world.log, [format!("be 17 64 33 7 {:?}", nbt)]);
}

#[test]
fn entity_packets_reach_the_store() {
    let mut r = Rig::new();
    r.apply(spawn_packet(3, DVec3::new(1.0, 2.0, 3.0)));
    r.apply(ClientboundPlay::MoveEntityPos(cb::MoveEntityPos {
        entity_id: 3,
        deltas: MoveDeltas::Linear([4096, 0, -2048]),
        on_ground: true,
    }));
    assert_eq!(
        r.entities.pose(EntityId(3)).unwrap().0,
        DVec3::new(2.0, 2.0, 2.5)
    );
    // Relative x, absolute y/z.
    r.apply(ClientboundPlay::TeleportEntity(cb::TeleportEntity {
        entity_id: 3,
        pos: DVec3::new(10.0, 70.0, 0.0),
        velocity: DVec3::ZERO,
        yaw: 45.0,
        pitch: 0.0,
        flags: teleport_flags::X,
        on_ground: false,
    }));
    assert_eq!(
        r.entities.pose(EntityId(3)).unwrap(),
        (DVec3::new(12.0, 70.0, 0.0), 45.0, 0.0)
    );
    r.apply(ClientboundPlay::RotateHead(cb::RotateHead {
        entity_id: 3,
        head_yaw: 30.0,
    }));
    // Motion of our own entity goes to the player, others' to the store.
    r.apply(ClientboundPlay::SetEntityMotion(cb::SetEntityMotion {
        entity_id: 7,
        velocity: DVec3::new(0.0, 0.5, 0.0),
    }));
    r.apply(ClientboundPlay::SetEntityMotion(cb::SetEntityMotion {
        entity_id: 3,
        velocity: DVec3::new(0.0, -1.0, 0.0),
    }));
    assert_eq!(r.session.player.state.velocity.y, 0.5);
    assert_eq!(r.entities.log, ["head 3 30", "velocity 3 -1"]);
    r.apply(ClientboundPlay::RemoveEntities(cb::RemoveEntities {
        entity_ids: vec![3],
    }));
    assert_eq!(r.entities.len(), 0);
}

#[test]
fn teleport_is_accepted_one_tick_later_then_movement_follows() {
    let mut r = Rig::new();
    // Nothing is sent before the first teleport.
    r.session.end_tick(
        &InputState::default(),
        &mut r.world,
        &r.entities,
        &mut r.out,
    );
    assert!(r.out.is_empty());
    r.apply(teleport(1, DVec3::new(0.5, -60.0, 0.5), 0));
    // The accept is not sent from the packet handler (a join teleport answered within
    // microseconds of arrival races the server's join bookkeeping: "moved too quickly!"),
    // and not on the tick it arrived in either.
    assert!(r.out.is_empty());
    r.session.end_tick(
        &InputState::default(),
        &mut r.world,
        &r.entities,
        &mut r.out,
    );
    assert!(r.out.is_empty());
    r.session.end_tick(
        &InputState::default(),
        &mut r.world,
        &r.entities,
        &mut r.out,
    );
    assert_eq!(
        r.names(),
        [
            "accept_teleportation",
            "player_loaded",
            "player_input",
            "client_tick_end"
        ]
    );
    // The fake world has no chunks: the player stays put, so no movement packet.
    // player_input only when it changes; player_loaded only once.
    r.session.end_tick(
        &InputState::default(),
        &mut r.world,
        &r.entities,
        &mut r.out,
    );
    assert_eq!(r.names(), ["client_tick_end"]);
    r.apply(teleport(2, DVec3::new(1.0, 0.0, 0.0), teleport_flags::X));
    assert!(r.out.is_empty());
    // x relative, y and z absolute.
    assert_eq!(r.session.player.state.pos, DVec3::new(1.5, 0.0, 0.0));
    r.session.end_tick(
        &InputState::default(),
        &mut r.world,
        &r.entities,
        &mut r.out,
    );
    assert_eq!(r.names(), ["client_tick_end"]);
    r.session.end_tick(
        &InputState::default(),
        &mut r.world,
        &r.entities,
        &mut r.out,
    );
    assert_eq!(r.names(), ["accept_teleportation", "client_tick_end"]);
}

#[test]
fn respawn_resets_world_and_entities() {
    let mut r = Rig::new();
    r.apply(teleport(1, DVec3::ZERO, 0));
    r.apply(spawn_packet(3, DVec3::ZERO));
    r.apply(level_chunk(0, 0));
    r.apply(ClientboundPlay::Respawn(cb::Respawn {
        spawn: SpawnInfo {
            dimension_type: 0,
            dimension: Identifier::minecraft("the_nether"),
            hashed_seed: 0,
            game_mode: 0,
            previous_game_mode: 0,
            is_debug: false,
            is_flat: false,
            death_location: None,
            portal_cooldown: 0,
            sea_level: 32,
        },
        data_kept: 0,
    }));
    assert_eq!(r.world.chunk_count(), 0);
    assert_eq!(r.world.log.last().unwrap(), "reset minecraft:the_nether");
    assert_eq!(r.entities.len(), 0);
    assert_eq!(
        r.session.dimension.name,
        Identifier::minecraft("the_nether")
    );
    assert_eq!(r.session.player.game_mode, GameMode::Survival);
    // Movement waits for the next teleport after a respawn.
    r.out.clear();
    r.session.end_tick(
        &InputState::default(),
        &mut r.world,
        &r.entities,
        &mut r.out,
    );
    assert!(r.out.is_empty());
}

/// The real `World` behind the sink: a synthetic chunk (our own encoder) loads and counts,
/// and a respawn keeps the biome width. Needs the data generator reports in the cache.
#[test]
#[ignore = "needs cache/datagen/26.3/reports (GameData::load)"]
fn real_world_through_the_sink() {
    let data = load_game_data().unwrap();
    let dim = DimensionInfo::overworld();
    let mut world = World::new_with_biomes(dim.clone(), data, 67);
    assert_eq!(world.biome_bits(), 7);
    let stone = BlockStateId(1);
    let c = haste_world::chunk::synthetic_chunk_data(
        ChunkPos::new(0, 0),
        dim.min_section(),
        dim.section_count(),
        |s, _, _, _| if s < 2 { stone } else { BlockStateId::AIR },
    );
    let mut s = Session::new(EntityId(1), GameMode::Creative, dim);
    let mut ents = new_entity_store(0);
    let mut out = Vec::new();
    s.apply(
        ClientboundPlay::LevelChunkWithLight(cb::LevelChunkWithLight { chunk: c }),
        &mut world,
        &mut ents,
        &SyncedRegistries::default(),
        &mut out,
    );
    assert_eq!(
        s.counters.chunk_errors, 0,
        "{:?}",
        s.counters.last_chunk_error
    );
    assert_eq!(WorldSink::chunk_count(&world), 1);
    assert_eq!(world.non_empty_sections(), 2);
    // A block_entity_data packet updates the block entity for the loaded chunk.
    let sign = BlockPos::new(3, 8, 3);
    let nbt = nbt().write_unnamed();
    s.apply(
        ClientboundPlay::BlockEntityData(cb::BlockEntityData {
            pos: sign,
            kind: 7,
            nbt: nbt.clone(),
        }),
        &mut world,
        &mut ents,
        &SyncedRegistries::default(),
        &mut out,
    );
    let be = world.block_entity(sign).unwrap();
    assert_eq!((be.kind, &be.nbt), (7, &nbt));
    WorldSink::reset(&mut world, &DimensionInfo::overworld());
    assert_eq!(WorldSink::chunk_count(&world), 0);
    assert_eq!(world.biome_bits(), 7);
}

#[test]
fn hurt_animation_marks_and_tilts() {
    let mut r = Rig::new();
    // Another entity's hurt: marked, no camera tilt.
    r.apply(ClientboundPlay::HurtAnimation(cb::HurtAnimation {
        entity_id: 42,
        yaw: 0.0,
    }));
    assert!(r.entities.log.iter().any(|l| l == "hurt 42"));
    assert_eq!(r.session.hurt_tilt_degrees(), 0.0);
    // Our own hurt (session id 7): the tilt starts and eases out over 10 ticks.
    r.apply(ClientboundPlay::HurtAnimation(cb::HurtAnimation {
        entity_id: 7,
        yaw: 0.0,
    }));
    assert_eq!(r.session.hurt_tilt_degrees().abs(), 14.0);
    // 5 ticks in: (5/10)^2 of 14 degrees.
    for _ in 0..5 {
        end_tick_once(&mut r);
    }
    let half = r.session.hurt_tilt_degrees().abs();
    assert!((half - 14.0 * 0.25).abs() < 1e-6, "{half}");
    // 10 ticks in: gone.
    for _ in 0..5 {
        end_tick_once(&mut r);
    }
    assert_eq!(r.session.hurt_tilt_degrees(), 0.0);
    // A second hurt restarts the tilt.
    r.apply(ClientboundPlay::HurtAnimation(cb::HurtAnimation {
        entity_id: 7,
        yaw: 0.0,
    }));
    assert_eq!(r.session.hurt_tilt_degrees().abs(), 14.0);
}

#[test]
fn hurt_tilt_direction_follows_the_damage_yaw() {
    let mut r = Rig::new();
    // Damage from the player's right (yaw 90 degrees off our facing) rolls the other way
    // than damage from the left.
    r.session.player.state.yaw = 0.0;
    r.apply(ClientboundPlay::HurtAnimation(cb::HurtAnimation {
        entity_id: 7,
        yaw: 90.0,
    }));
    let right = r.session.hurt_tilt_degrees();
    let mut r2 = Rig::new();
    r2.session.player.state.yaw = 0.0;
    r2.apply(ClientboundPlay::HurtAnimation(cb::HurtAnimation {
        entity_id: 7,
        yaw: -90.0,
    }));
    let left = r2.session.hurt_tilt_degrees();
    assert!(right * left < 0.0, "opposite rolls: {right} vs {left}");
    assert!((right.abs() - 14.0).abs() < 1e-6);
}

#[test]
fn damage_event_and_death_mark_entities() {
    let mut r = Rig::new();
    r.apply(ClientboundPlay::DamageEvent(cb::DamageEvent {
        entity_id: 42,
        source_type: 1,
        source_cause: 0,
        source_direct: 0,
        source_pos: None,
    }));
    assert!(r.entities.log.iter().any(|l| l == "hurt 42"));
    r.apply(ClientboundPlay::EntityEvent(cb::EntityEvent {
        entity_id: 42,
        status: 3,
    }));
    assert!(r.entities.log.iter().any(|l| l == "dead 42"));
}

/// One session tick (the part of `NetDriver::end_tick` that lives in `Session`).
fn end_tick_once(r: &mut Rig) {
    let mut out = Vec::new();
    let mut world = std::mem::take(&mut r.world);
    let entities = std::mem::take(&mut r.entities);
    r.session.end_tick(
        &haste_core::input::InputState::default(),
        &mut world,
        &entities,
        &mut out,
    );
    r.world = world;
    r.entities = entities;
}

/// A synced `minecraft:particle_type` registry (id 0 `flame`, 1 `smoke`).
fn particle_registry() -> SyncedRegistries {
    let mut regs = SyncedRegistries::default();
    regs.insert(
        Identifier::minecraft("particle_type"),
        vec![
            RegistryEntry {
                id: Identifier::minecraft("flame"),
                data: None,
            },
            RegistryEntry {
                id: Identifier::minecraft("smoke"),
                data: None,
            },
        ],
    );
    regs
}

/// A `level_particles` packet (flame by default).
fn level_particles(kind: i32, count: i32) -> ClientboundPlay {
    ClientboundPlay::LevelParticles(cb::LevelParticles {
        particle: Particle {
            kind,
            data: bytes::Bytes::new(),
        },
        override_limiter: false,
        always_visible: false,
        pos: DVec3::new(10.0, 64.0, -5.0),
        offset_x: 0.5,
        offset_y: 0.0,
        offset_z: 0.25,
        max_speed_x: 0.1,
        max_speed_y: 0.0,
        max_speed_z: 0.05,
        count,
        randomization: 0,
    })
}

#[test]
fn level_particles_queue_a_spawn_per_packet() {
    let mut r = Rig::new();
    r.regs = particle_registry();
    r.apply(level_particles(0, 10));
    r.apply(level_particles(1, 0));
    assert_eq!(r.session.particle_spawns.len(), 2);
    let first = &r.session.particle_spawns[0];
    assert_eq!(first.kind, Identifier::minecraft("flame"));
    assert_eq!(first.pos, DVec3::new(10.0, 64.0, -5.0));
    assert_eq!(first.offset, [0.5, 0.0, 0.25]);
    assert_eq!(first.max_speed, [0.1, 0.0, 0.05]);
    assert_eq!(first.count, 10);
    assert_eq!(
        r.session.particle_spawns[1].kind,
        Identifier::minecraft("smoke")
    );
    assert_eq!(r.session.particle_spawns[1].count, 0);
}

#[test]
fn level_particles_with_an_unknown_type_id_are_dropped() {
    let mut r = Rig::new();
    r.regs = particle_registry();
    // No id 7 in the synced registry: dropped silently (no panic, nothing queued).
    r.apply(level_particles(7, 10));
    assert_eq!(r.session.particle_spawns.len(), 0);
    // Without a synced particle registry at all: same.
    let mut r2 = Rig::new();
    r2.apply(level_particles(0, 10));
    assert_eq!(r2.session.particle_spawns.len(), 0);
}
