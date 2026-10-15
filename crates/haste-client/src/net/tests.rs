//! Driver tests: packets are forwarded to the world/entity sinks, the player state and the
//! serverbound replies follow the vanilla order. Recording fakes, no network.

use std::collections::HashMap;

use super::*;
use haste_protocol::nbt::{Nbt, NbtCompound};
use haste_protocol::packets::play::clientbound::{MoveDeltas, SpawnInfo};
use haste_protocol::wire::particle::Particle;

#[derive(Default)]
struct FakeWorld {
    log: Vec<String>,
    chunks: Vec<ChunkPos>,
    /// Block states the lid tests place (the `WorldSink` reads them).
    states: HashMap<BlockPos, BlockStateId>,
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
    fn block_state_at(&self, pos: BlockPos) -> BlockStateId {
        self.states.get(&pos).copied().unwrap_or(BlockStateId::AIR)
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
    /// Riding bookkeeping, mirroring the store's (vehicle -> passengers).
    riding: HashMap<EntityId, Vec<EntityId>>,
    /// Registry index per entity id (spawn's `kind`), for the seat offset.
    kinds: HashMap<EntityId, i32>,
    dims: HashMap<EntityId, (f32, f32)>,
}

impl EntitySink for FakeEntities {
    fn spawn(&mut self, s: &EntitySpawn) {
        self.poses.insert(s.id, (s.pos, s.yaw, s.pitch));
        self.kinds.insert(s.id, s.kind.0 as i32);
        // The default box, like a store without the oracle dump.
        self.dims.insert(s.id, (0.6, 1.8));
    }
    fn remove(&mut self, ids: &[EntityId]) {
        for id in ids {
            self.poses.remove(id);
            self.kinds.remove(id);
            self.dims.remove(id);
            self.riding.remove(id);
            for list in self.riding.values_mut() {
                list.retain(|p| !ids.contains(p));
            }
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
    fn swing(&mut self, id: EntityId, hand: i32, animation: i32, duration: i32) {
        self.log
            .push(format!("swing {} {hand} {animation} {duration}", id.0));
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
    fn set_passengers(&mut self, vehicle: EntityId, passengers: &[EntityId]) {
        self.riding.insert(vehicle, passengers.to_vec());
        self.log
            .push(format!("passengers {} {:?}", vehicle.0, passengers));
    }
    fn entity_type_info(&self, id: EntityId) -> Option<(haste_core::RegistryIndex, (f32, f32))> {
        self.kinds.get(&id).and_then(|k| {
            self.dims
                .get(&id)
                .map(|d| (haste_core::RegistryIndex(*k as u32), *d))
        })
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

/// The static particle registry the test sessions resolve ids against (id 0 `flame`, 1
/// `smoke`).
fn test_particle_names() -> std::sync::Arc<Vec<Identifier>> {
    std::sync::Arc::new(vec![
        Identifier::minecraft("flame"),
        Identifier::minecraft("smoke"),
    ])
}

impl Rig {
    fn new() -> Self {
        Self {
            session: Session::new(
                EntityId(7),
                GameMode::Creative,
                DimensionInfo::overworld(),
                test_particle_names(),
            ),
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
    spawn_packet_kind(id, pos, 3)
}

/// Spawn with a chosen entity type index (the synced registry's order matters for the seat).
fn spawn_packet_kind(id: i32, pos: DVec3, kind: i32) -> ClientboundPlay {
    ClientboundPlay::AddEntity(cb::AddEntity {
        entity_id: id,
        uuid: Default::default(),
        kind,
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

/// A chest's lid: `block_event` action 1 opens it, the animation steps 0.1 per tick
/// (vanilla `ChestLidController`), the snapshot lerps by partial tick, and closing runs
/// back down. Other actions and non-lid blocks (pistons use action 1 too) are ignored.
#[test]
fn block_events_drive_chest_lids() {
    let mut r = Rig::new();
    let chest = BlockPos::new(2, 64, 3);
    let piston = BlockPos::new(5, 64, 3);
    // The session knows the chest's state is a lid; the piston's is not.
    r.world.states.insert(chest, BlockStateId(9));
    r.session
        .set_lid_states(std::iter::once(BlockStateId(9)).collect());
    let event = |pos, action, param| {
        ClientboundPlay::BlockEvent(cb::BlockEvent {
            pos,
            action,
            param,
            block: 0,
        })
    };
    // Not a lid block: no entry ever.
    r.apply(event(piston, 1, 1));
    assert!(r.session.lids.is_empty());
    // Other actions on the chest are ignored too.
    r.apply(event(chest, 0, 1));
    assert!(r.session.lids.is_empty());
    // Open: one player. The lid starts stepping on the next end_tick.
    r.apply(event(chest, 1, 1));
    let tick = |r: &mut Rig| {
        r.session.end_tick(
            &InputState::default(),
            &mut r.world,
            &r.entities,
            &mut r.out,
        );
    };
    tick(&mut r);
    let mut lids = Vec::new();
    r.session.lid_states(&mut lids);
    assert_eq!(lids.len(), 1);
    let pair = lids[0];
    assert_eq!(pair.0, chest);
    let near = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5;
    assert!(near(pair.1, (0.0, 0.1)), "{:?}", pair.1);
    // Ten open ticks reach fully open; the lerp pair then brackets 1.
    for _ in 0..9 {
        tick(&mut r);
    }
    lids.clear();
    r.session.lid_states(&mut lids);
    assert!(near(lids[0].1, (0.9, 1.0)), "{:?}", lids[0].1);
    tick(&mut r);
    lids.clear();
    r.session.lid_states(&mut lids);
    assert!(near(lids[0].1, (1.0, 1.0)), "{:?}", lids[0].1);
    // Close: count 0.
    r.apply(event(chest, 1, 0));
    tick(&mut r);
    lids.clear();
    r.session.lid_states(&mut lids);
    assert!(near(lids[0].1, (1.0, 0.9)), "{:?}", lids[0].1);
    for _ in 0..10 {
        tick(&mut r);
    }
    // At rest closed, the snapshot stops listing it (nothing to draw).
    lids.clear();
    r.session.lid_states(&mut lids);
    assert!(lids.is_empty());
    assert_eq!(r.session.lids[&chest].value, 0.0);
    // A block update drops the lid with the chest.
    r.apply(event(chest, 1, 1));
    r.apply(ClientboundPlay::BlockUpdate(cb::BlockUpdate {
        pos: chest,
        state: 0,
    }));
    assert!(!r.session.lids.contains_key(&chest));
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
fn swing_animation_reaches_the_store() {
    let mut r = Rig::new();
    r.apply(ClientboundPlay::SwingAnimation(cb::SwingAnimation {
        entity_id: 3,
        hand: 0,
        animation: 1,
        duration: 6,
    }));
    assert_eq!(r.entities.log, ["swing 3 0 1 6"]);
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
    let mut s = Session::new(EntityId(1), GameMode::Creative, dim, test_particle_names());
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
fn hurt_animation_marks_and_starts_the_hurt_ticks() {
    let mut r = Rig::new();
    // Another entity's hurt: marked, no camera tilt.
    r.apply(ClientboundPlay::HurtAnimation(cb::HurtAnimation {
        entity_id: 42,
        yaw: 0.0,
    }));
    assert!(r.entities.log.iter().any(|l| l == "hurt 42"));
    assert_eq!(r.session.hurt_ticks, 0);
    // Our own hurt (session id 7): the hurt time starts at 10 and counts down per tick.
    r.apply(ClientboundPlay::HurtAnimation(cb::HurtAnimation {
        entity_id: 7,
        yaw: 0.0,
    }));
    assert_eq!(r.session.hurt_ticks, 10);
    for _ in 0..5 {
        end_tick_once(&mut r);
    }
    assert_eq!(r.session.hurt_ticks, 5);
    for _ in 0..5 {
        end_tick_once(&mut r);
    }
    assert_eq!(r.session.hurt_ticks, 0);
    // A second hurt restarts it.
    r.apply(ClientboundPlay::HurtAnimation(cb::HurtAnimation {
        entity_id: 7,
        yaw: 0.0,
    }));
    assert_eq!(r.session.hurt_ticks, 10);
}

#[test]
fn hurt_tilt_direction_follows_the_damage_yaw() {
    let mut r = Rig::new();
    // The packet's yaw is the roll axis direction, taken as-is (vanilla's hurtDir).
    r.apply(ClientboundPlay::HurtAnimation(cb::HurtAnimation {
        entity_id: 7,
        yaw: 90.0,
    }));
    assert_eq!(r.session.hurt_dir, 90.0);
    let mut r2 = Rig::new();
    r2.apply(ClientboundPlay::HurtAnimation(cb::HurtAnimation {
        entity_id: 7,
        yaw: -90.0,
    }));
    assert_eq!(r2.session.hurt_dir, -90.0);
}

#[test]
fn death_ticks_count_while_health_is_zero() {
    let mut r = Rig::new();
    r.apply(ClientboundPlay::SetHealth(cb::SetHealth {
        health: 0.0,
        food: 20,
        saturation: 5.0,
    }));
    end_tick_once(&mut r);
    assert_eq!(r.session.death_ticks, 1);
    end_tick_once(&mut r);
    assert_eq!(r.session.death_ticks, 2);
    // Healing resets the counter.
    r.apply(ClientboundPlay::SetHealth(cb::SetHealth {
        health: 20.0,
        food: 20,
        saturation: 5.0,
    }));
    end_tick_once(&mut r);
    assert_eq!(r.session.death_ticks, 0);
}

#[test]
fn fov_modifier_target_matches_vanilla() {
    use crate::net::UseItemKind as K;
    // Walking: the attribute equals the walking speed.
    assert_eq!(
        fov_modifier_target(false, 0.1, 0.1, None, 0, false, 1.0),
        1.0
    );
    // Sprinting: the attribute carries the x1.3 sprinting modifier.
    assert_eq!(
        fov_modifier_target(false, 0.13, 0.1, None, 0, false, 1.0),
        1.15
    );
    // Flying: x1.1; sprint-flying compounds.
    assert_eq!(
        fov_modifier_target(true, 0.1, 0.1, None, 0, false, 1.0),
        1.1
    );
    assert_eq!(
        fov_modifier_target(true, 0.13, 0.1, None, 0, false, 1.0),
        1.265
    );
    // The effect scale lerps between 1 and the modifier.
    assert_eq!(
        fov_modifier_target(false, 0.13, 0.1, None, 0, false, 0.5),
        1.075
    );
    assert_eq!(
        fov_modifier_target(false, 0.13, 0.1, None, 0, false, 0.0),
        1.0
    );
    // A zero walking speed skips the speed factor (vanilla's guard).
    assert_eq!(
        fov_modifier_target(false, 0.13, 0.0, None, 0, false, 1.0),
        1.0
    );
    // The bow draw: 1 - min(ticks/20, 1)^2 * 0.15.
    assert!(
        (fov_modifier_target(false, 0.1, 0.1, Some(K::Bow), 20, false, 1.0) - 0.85).abs() < 1e-6
    );
    assert!(
        (fov_modifier_target(false, 0.1, 0.1, Some(K::Bow), 10, false, 1.0) - (1.0 - 0.25 * 0.15))
            .abs()
            < 1e-6
    );
    // The spyglass is a full 0.1 in first person, whatever the effect scale; in third
    // person it does nothing.
    assert_eq!(
        fov_modifier_target(false, 0.1, 0.1, Some(K::Spyglass), 5, true, 1.0),
        0.1
    );
    assert_eq!(
        fov_modifier_target(false, 0.1, 0.1, Some(K::Spyglass), 5, true, 0.0),
        0.1
    );
    assert_eq!(
        fov_modifier_target(false, 0.1, 0.1, Some(K::Spyglass), 5, false, 1.0),
        1.0
    );
}

#[test]
fn fov_modifier_smooths_by_half_a_tick_and_clamps() {
    let mut r = Rig::new();
    // From vanilla's initial 0: 0.5, 0.75, 0.875, ... towards the walking target of 1.
    for want in [0.5, 0.75, 0.875] {
        end_tick_once(&mut r);
        assert!(
            (r.session.fov_modifier - want).abs() < 1e-6,
            "{}",
            r.session.fov_modifier
        );
    }
    // Sprinting (target 1.15) from 1.0: 1.075.
    r.session.fov_modifier = 1.0;
    r.session.old_fov_modifier = 1.0;
    r.session.player.attrs.movement_speed = 0.13;
    end_tick_once(&mut r);
    assert!((r.session.fov_modifier - 1.075).abs() < 1e-6);
    // The clamp holds the upper bound however large the target.
    r.session.player.attrs.movement_speed = 100.0;
    for _ in 0..50 {
        end_tick_once(&mut r);
    }
    assert_eq!(r.session.fov_modifier, 1.5);
    // A zero movement speed gives speedFactor 0, so the target is (0/0.1 + 1)/2 = 0.5, not 0:
    // smoothing approaches 0.5 (the lower clamp 0.1 only matters for the spyglass target).
    r.session.player.attrs.movement_speed = 0.0;
    for _ in 0..50 {
        end_tick_once(&mut r);
    }
    assert!(
        (r.session.fov_modifier - 0.5).abs() < 1e-6,
        "{}",
        r.session.fov_modifier
    );
}

#[test]
fn use_ticks_count_the_held_draw_and_reset_on_release_or_item_change() {
    let mut r = Rig::new();
    use haste_core::input::InteractInput;
    r.session.player.set_interact(InteractInput {
        use_item: true,
        ..Default::default()
    });
    r.session.player.int.hotbar[0].item = Some(RegistryIndex(3));
    r.session.player.held_slot = 0;
    end_tick_once(&mut r);
    assert_eq!(r.session.use_ticks, 1);
    end_tick_once(&mut r);
    assert_eq!(r.session.use_ticks, 2);
    // Releasing the button ends the draw.
    r.session.player.set_interact(InteractInput::default());
    end_tick_once(&mut r);
    assert_eq!(r.session.use_ticks, 0);
    // Holding again with a different item restarts it.
    r.session.player.set_interact(InteractInput {
        use_item: true,
        ..Default::default()
    });
    r.session.player.int.hotbar[0].item = Some(RegistryIndex(4));
    end_tick_once(&mut r);
    assert_eq!(r.session.use_ticks, 1);
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

/// One session tick: the driver's `end_tick` part that lives in `Session`, then the per-tick
/// camera effects (`tick_camera`; the game thread's `tick_once` does the same two steps).
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
    r.session.tick_camera(None, false);
    r.world = world;
    r.entities = entities;
    r.out.extend(out);
}

/// The `minecraft:entity_type` names in id order, as `set_entity_type_names` takes them
/// (entry i at index i, like the real registry: entry order defines the ids).
fn names(names: &[&str]) -> Vec<Identifier> {
    names.iter().map(|n| Identifier::minecraft(n)).collect()
}

#[test]
fn set_passengers_mounts_and_dismounts_the_local_player() {
    let mut r = Rig::new();
    r.session
        .set_entity_type_names(names(&["player", "zombie", "minecart", "horse", "pig"]));
    r.apply(spawn_packet_kind(5, DVec3::new(40.0, 70.0, 40.0), 2)); // minecart
                                                                    // Mount: our entity id is 7.
    r.apply(ClientboundPlay::SetPassengers(cb::SetPassengers {
        vehicle: 5,
        passengers: vec![7],
    }));
    assert_eq!(r.session.player.vehicle(), Some(EntityId(5)));
    assert_eq!(
        r.entities.riding.get(&EntityId(5)).unwrap(),
        &vec![EntityId(7)]
    );
    // The minecart's seat: `passengerAttachments(0.1875)` minus the player's own vehicle
    // attachment (`Avatar.DEFAULT_VEHICLE_ATTACHMENT`, 0.6) is 0.4125 *below* the cart's
    // position. The ride tick runs once the join is through (accept, player_loaded: two ticks
    // after the teleport); the player's own teleport position is irrelevant while riding: the
    // vehicle's position is.
    r.apply(teleport(1, DVec3::new(40.0, 71.0, 40.0), 0));
    end_tick_once(&mut r);
    end_tick_once(&mut r);
    end_tick_once(&mut r);
    assert_eq!(
        r.session.player.state.pos,
        DVec3::new(40.0, 69.5875, 40.0),
        "the passenger sits at the vehicle's position plus the seat offset"
    );
    // The passenger follows the vehicle's moves.
    r.apply(ClientboundPlay::MoveEntityPos(cb::MoveEntityPos {
        entity_id: 5,
        deltas: MoveDeltas::Linear([20480, 0, 0]),
        on_ground: false,
    }));
    end_tick_once(&mut r);
    assert_eq!(r.session.player.state.pos, DVec3::new(45.0, 69.5875, 40.0));
    // Dismount: the list replaces the previous one and no longer names us.
    r.apply(ClientboundPlay::SetPassengers(cb::SetPassengers {
        vehicle: 5,
        passengers: vec![],
    }));
    assert_eq!(r.session.player.vehicle(), None);
    // Normal physics resumes from where we are. The fake world has no chunk loaded at the
    // player's chunk, so vanilla's unloaded-chunk step (`Player.aiStep`'s `travelInAir`
    // pin) moves it down by the unloaded-chunk fall speed this tick; x and z stay.
    end_tick_once(&mut r);
    assert_eq!(
        r.session.player.state.pos,
        DVec3::new(
            45.0,
            69.5875 + crate::player::UNLOADED_CHUNK_FALL_SPEED,
            40.0
        ),
        "normal physics resumes, falling at the unloaded-chunk rate"
    );
}

#[test]
fn a_set_passengers_for_another_vehicle_does_not_dismount_us() {
    let mut r = Rig::new();
    r.apply(spawn_packet_kind(5, DVec3::ZERO, 1));
    r.apply(spawn_packet_kind(6, DVec3::ZERO, 1));
    r.apply(ClientboundPlay::SetPassengers(cb::SetPassengers {
        vehicle: 5,
        passengers: vec![7],
    }));
    // Another vehicle's list, without us: we keep riding 5.
    r.apply(ClientboundPlay::SetPassengers(cb::SetPassengers {
        vehicle: 6,
        passengers: vec![9, 10],
    }));
    assert_eq!(r.session.player.vehicle(), Some(EntityId(5)));
}

#[test]
fn removing_our_vehicle_dismounts_us() {
    let mut r = Rig::new();
    r.apply(spawn_packet(5, DVec3::ZERO));
    r.apply(ClientboundPlay::SetPassengers(cb::SetPassengers {
        vehicle: 5,
        passengers: vec![7],
    }));
    assert_eq!(r.session.player.vehicle(), Some(EntityId(5)));
    r.apply(ClientboundPlay::RemoveEntities(cb::RemoveEntities {
        entity_ids: vec![5],
    }));
    assert_eq!(r.session.player.vehicle(), None);
    assert!(!r.entities.riding.contains_key(&EntityId(5)));
}

#[test]
fn the_seat_offset_comes_from_the_entity_type() {
    let mut r = Rig::new();
    // Registry: index 4 is horse.
    r.session
        .set_entity_type_names(names(&["player", "zombie", "minecart", "pig", "horse"]));
    r.apply(spawn_packet_kind(5, DVec3::new(0.0, 64.0, 0.0), 4));
    r.apply(ClientboundPlay::SetPassengers(cb::SetPassengers {
        vehicle: 5,
        passengers: vec![7],
    }));
    r.apply(teleport(1, DVec3::new(0.0, 64.0, 0.0), 0));
    end_tick_once(&mut r);
    end_tick_once(&mut r);
    end_tick_once(&mut r);
    // The horse's seat: `passengerAttachments(1.44375)` minus the player's own 0.6 is 0.84375
    // above its position (not its 1.8 default-box height).
    assert!((r.session.player.state.pos.y - 64.84375).abs() < 1e-6);
}

#[test]
fn riding_sends_rotation_and_input_only() {
    let mut r = Rig::new();
    r.apply(spawn_packet(5, DVec3::new(1.0, 2.0, 3.0)));
    // Join: teleport, accept, loaded.
    r.apply(teleport(1, DVec3::new(1.0, 2.0, 3.0), 0));
    end_tick_once(&mut r);
    end_tick_once(&mut r);
    r.out.clear();
    // Mount.
    r.apply(ClientboundPlay::SetPassengers(cb::SetPassengers {
        vehicle: 5,
        passengers: vec![7],
    }));
    // Look changes and a key is held while riding: input (the vehicle's steering keys) and
    // rotation only, never a position packet.
    let look = haste_core::input::InputState {
        yaw: 90.0,
        forward: true,
        ..Default::default()
    };
    let mut out = Vec::new();
    let mut world = std::mem::take(&mut r.world);
    let entities = std::mem::take(&mut r.entities);
    r.session.end_tick(&look, &mut world, &entities, &mut out);
    r.world = world;
    r.entities = entities;
    let names: Vec<&str> = out
        .iter()
        .map(|p| match p {
            ServerboundPlay::MovePlayerPosRot(_) => "pos_rot",
            ServerboundPlay::MovePlayerPos(_) => "pos",
            ServerboundPlay::MovePlayerRot(_) => "rot",
            ServerboundPlay::MovePlayerStatusOnly(_) => "status",
            ServerboundPlay::PlayerInput(_) => "input",
            ServerboundPlay::ClientTickEnd(_) => "tick_end",
            _ => "other",
        })
        .collect();
    // While riding, the vehicle's position is ours; only rotation packets go out.
    assert_eq!(names, ["input", "rot", "tick_end"]);
    // Sneak while riding: player_input carries the shift (sneak) flag, the server answers
    // with set_passengers to dismount.
    let sneak = haste_core::input::InputState {
        yaw: 90.0,
        sneak: true,
        ..Default::default()
    };
    let mut out = Vec::new();
    let mut world = std::mem::take(&mut r.world);
    let entities = std::mem::take(&mut r.entities);
    r.session.end_tick(&sneak, &mut world, &entities, &mut out);
    r.world = world;
    r.entities = entities;
    let input = out.iter().find_map(|p| match p {
        ServerboundPlay::PlayerInput(i) => Some(i.flags),
        _ => None,
    });
    assert_eq!(
        input,
        Some(haste_protocol::packets::play::serverbound::input_flags::SNEAK)
    );
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
    // The ids come from the session's static particle registry (flame 0, smoke 1).
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
    // No id 7 in the static registry: dropped silently (no panic, nothing queued).
    r.apply(level_particles(7, 10));
    assert_eq!(r.session.particle_spawns.len(), 0);
}

/// The attached firework rocket's push (vanilla `FireworkRocketEntity.tick`): the rocket is
/// tracked by its entity data (index 9, the attached-to id) and its push is applied before the
/// gliding player's tick.
#[test]
fn an_attached_firework_boosts_the_gliding_player() {
    use haste_core::entity::MetadataValue;
    let mut r = Rig::new();
    r.session
        .set_entity_type_names(names(&["player", "firework_rocket"]));
    r.apply(spawn_packet_kind(20, DVec3::new(0.0, 100.0, 0.0), 1)); // firework_rocket
                                                                    // The rocket's attached-to target is us (entity 7): `OPTIONAL_UNSIGNED_INT` (type 19), a
                                                                    // VarInt of id + 1 = 8.
    r.apply(ClientboundPlay::SetEntityData(cb::SetEntityData {
        entity_id: 20,
        metadata: vec![MetadataEntry {
            index: 9,
            value: MetadataValue::Other {
                type_id: 19,
                raw: bytes::Bytes::from(vec![8u8]),
            },
        }],
    }));
    assert_eq!(r.session.attached_firework, Some(EntityId(20)));

    // Gliding: the push is `v = look * 0.85` from rest = (0, 0, 0.85) at yaw 0, pitch 0. The
    // player is not loaded, so the physics step does not run to change it.
    r.session.player.state.fall_flying = true;
    r.session.player.state.velocity = DVec3::ZERO;
    r.session.player.state.yaw = 0.0;
    r.session.player.state.pitch = 0.0;
    end_tick_once(&mut r);
    assert!(
        (r.session.player.state.velocity.z - 0.85).abs() < 1e-9,
        "{:?}",
        r.session.player.state.velocity
    );

    // Attached but not gliding: vanilla applies no push.
    r.session.player.state.velocity = DVec3::ZERO;
    r.session.player.state.fall_flying = false;
    end_tick_once(&mut r);
    assert_eq!(r.session.player.state.velocity.z, 0.0);

    // A rocket that is not ours (attached-to id 0 = empty) does not attach to us.
    r.session.attached_firework = None;
    r.apply(ClientboundPlay::SetEntityData(cb::SetEntityData {
        entity_id: 20,
        metadata: vec![MetadataEntry {
            index: 9,
            value: MetadataValue::Other {
                type_id: 19,
                raw: bytes::Bytes::from(vec![0u8]),
            },
        }],
    }));
    assert_eq!(r.session.attached_firework, None);

    // Removing the rocket clears the attachment.
    r.apply(ClientboundPlay::SetEntityData(cb::SetEntityData {
        entity_id: 20,
        metadata: vec![MetadataEntry {
            index: 9,
            value: MetadataValue::Other {
                type_id: 19,
                raw: bytes::Bytes::from(vec![8u8]),
            },
        }],
    }));
    assert_eq!(r.session.attached_firework, Some(EntityId(20)));
    r.apply(ClientboundPlay::RemoveEntities(cb::RemoveEntities {
        entity_ids: vec![20],
    }));
    assert_eq!(r.session.attached_firework, None);
}

#[test]
fn lightmap_boss_darkening_follows_the_vanilla_ramp() {
    // GameRenderer.tick: +0.05/tick while a boss bar asks to darken (cap 1), -0.0125/tick
    // otherwise. The per-frame value interpolates boss_darkening_o -> boss_darkening.
    let mut s = LightmapState::default();
    s.boss_bars.insert(1, (true, false));
    s.tick();
    assert!((s.boss_darkening - 0.05).abs() < 1e-6);
    assert_eq!(s.boss_darkening_o, 0.0);
    for _ in 0..40 {
        s.tick();
    }
    assert_eq!(s.boss_darkening, 1.0);
    // BossEvent.setDarkenScreen / createWorldFog / removal through the packet action.
    s.boss_bars.insert(7, (false, false));
    s.boss_event(7, &cb::BossEventAction::UpdateFlags(0x05));
    assert_eq!(s.boss_bars[&7], (true, true));
    s.tick();
    assert!(s.boss_world_fog);
    s.boss_event(7, &cb::BossEventAction::Remove);
    assert!(!s.boss_bars.contains_key(&7));
    // Clearing the flag ramps back down by 0.0125.
    s.boss_bars.insert(1, (false, false));
    s.tick();
    assert!((s.boss_darkening - (1.0 - 0.0125)).abs() < 1e-6);
    assert!(!s.boss_world_fog);
}

#[test]
fn lightmap_effects_and_flicker_tick() {
    let ids = crate::player::interact::LightmapEffectIds {
        night_vision: Some(1),
        darkness: Some(2),
        ..Default::default()
    };
    let mut s = LightmapState::default();
    s.update_effect(ids, 1, 100);
    s.update_effect(ids, 2, 50);
    assert_eq!(s.night_vision, Some(100));
    assert_eq!(s.darkness, Some(50));
    // An unrelated effect changes nothing.
    s.update_effect(ids, 9, 10);
    assert_eq!(s.night_vision, Some(100));
    s.tick();
    // MobEffectInstance.tickClient decrements both durations by one.
    assert_eq!(s.night_vision, Some(99));
    assert_eq!(s.darkness, Some(49));
    // Darkness blends in over 22 ticks (one step per tick); its factor brackets the frame.
    assert!((s.darkness_blend.factor(1.0) - 1.0 / 22.0).abs() < 1e-6);
    // Infinite durations stay -1.
    s.update_effect(ids, 1, -1);
    s.tick();
    assert_eq!(s.night_vision, Some(-1));
    // A finite effect that reaches 0 is removed (LivingEntity.tickEffects).
    s.update_effect(ids, 1, 1);
    s.tick();
    assert_eq!(s.night_vision, None);
    s.remove_effect(ids, 2);
    assert_eq!(s.darkness, None);
    // The flicker random walk stays in a sane range.
    assert!(s.flicker.abs() < 1.0);
}

#[test]
fn lightmap_lightning_flashes_for_two_ticks() {
    // LightningBolt.tick on the client -> ClientLevel.setSkyFlashTime(2), which ClientLevel.tick
    // decrements by one each tick.
    let mut s = LightmapState::default();
    s.lightning();
    assert_eq!(s.sky_flash_time, 2);
    s.tick();
    assert_eq!(s.sky_flash_time, 1);
    s.tick();
    assert_eq!(s.sky_flash_time, 0);
    s.tick();
    assert_eq!(s.sky_flash_time, 0);
}
