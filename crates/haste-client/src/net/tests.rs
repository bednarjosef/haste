//! Driver tests: packets are forwarded to the world/entity sinks, the player state and the
//! serverbound replies follow the vanilla order. Recording fakes, no network.

use std::collections::HashMap;

use super::*;
use haste_protocol::nbt::{Nbt, NbtCompound};
use haste_protocol::packets::play::clientbound::{MoveDeltas, SpawnInfo};

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
fn bob_follows_horizontal_speed_on_the_ground_only() {
    let mut r = Rig::new();
    // On the ground moving at 0.05: the bob eases towards min(0.1, 0.05) by 0.4 per tick.
    r.session.player.state.on_ground = true;
    r.session.player.state.velocity = glam::DVec3::new(0.03, 0.0, 0.04);
    end_tick_once(&mut r);
    assert!((r.session.bob - 0.02).abs() < 1e-6, "{}", r.session.bob);
    assert_eq!(r.session.bob_o, 0.0);
    // In the air the target is 0.
    r.session.player.state.on_ground = false;
    end_tick_once(&mut r);
    assert!((r.session.bob - 0.012).abs() < 1e-6, "{}", r.session.bob);
    // Swimming also stops it.
    r.session.player.state.on_ground = true;
    r.session.player.state.swimming = true;
    end_tick_once(&mut r);
    assert!((r.session.bob - 0.0072).abs() < 1e-6, "{}", r.session.bob);
}

#[test]
fn walk_distance_grows_by_sixtenths_of_the_tick_displacement() {
    let mut r = Rig::new();
    // The tick snapshots the start position into `prev_pos`; the camera step that follows the
    // move (the game thread's order) sees the displacement. A tick in place adds nothing.
    r.session.player.state.pos = glam::DVec3::new(3.0, 64.0, 0.0);
    end_tick_once(&mut r);
    assert_eq!(r.session.walk_dist, 0.0);
    r.session.player.state.pos = glam::DVec3::new(6.0, 64.0, 0.0);
    r.session.tick_camera(None, false);
    assert!(
        (r.session.walk_dist - 1.8).abs() < 1e-6,
        "{}",
        r.session.walk_dist
    );
    assert_eq!(r.session.walk_dist_o, 0.0);
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
}
