// The minecart track packet end to end through the session into the real entity store:
// `AbstractMinecart.useExperimentalMovement` picks the behaviour at the spawn, and
// `ClientPacketListener.handleMinecartAlongTrack` feeds the steps the cart then walks.

use glam::DVec3;
use haste_core::game::GameMode;
use haste_core::Identifier;
use haste_entity::EntityStore;
use haste_protocol::haste_core::chunk::{BlockEntityData, ChunkData, LightData};
use haste_protocol::haste_core::game::DimensionInfo;
use haste_protocol::haste_core::{BlockPos, BlockStateId, ChunkPos, EntityId, RegistryIndex};
use haste_protocol::packets::play::clientbound as cb;
use haste_protocol::{ClientboundPlay, SyncedRegistries};

use super::super::{new_entity_store, Session, WorldSink};
use super::use_experimental_movement;

/// A world that answers nothing: the entity packets do not read it.
struct EmptyWorld;
impl WorldSink for EmptyWorld {
    fn load_chunk(&mut self, _data: &ChunkData) -> anyhow::Result<()> {
        Ok(())
    }
    fn unload_chunk(&mut self, _pos: ChunkPos) {}
    fn set_block(&mut self, _pos: BlockPos, _state: BlockStateId) {}
    fn apply_light(&mut self, _pos: ChunkPos, _light: &LightData) {}
    fn set_block_entity(&mut self, _pos: BlockPos, _data: BlockEntityData) {}
    fn reset(&mut self, _dim: &DimensionInfo) {}
    fn chunk_count(&self) -> usize {
        0
    }
    fn non_empty_sections(&self) -> usize {
        0
    }
}

fn registries(features: &[&str]) -> SyncedRegistries {
    let mut regs = SyncedRegistries::default();
    regs.features = features.iter().map(|f| Identifier::minecraft(f)).collect();
    regs
}

#[test]
fn the_minecart_improvements_feature_picks_the_new_behaviour() {
    assert!(!use_experimental_movement(&registries(&[])));
    assert!(!use_experimental_movement(&registries(&["vanilla"])));
    assert!(use_experimental_movement(&registries(&[
        "vanilla",
        "minecart_improvements"
    ])));
    // Another namespace's flag of the same name is not vanilla's.
    let mut regs = SyncedRegistries::default();
    regs.features = vec![Identifier::new("mod", "minecart_improvements")];
    assert!(!use_experimental_movement(&regs));
}

/// The `minecraft:minecart` protocol id from the data-generator report; `None` without the
/// cache (the store cannot tell a minecart then, so the end-to-end test is skipped).
fn minecart_kind() -> Option<RegistryIndex> {
    haste_entity::registry::entity_type_names()?
        .iter()
        .find(|(_, name)| **name == "minecraft:minecart")
        .map(|(kind, _)| *kind)
}

/// Spawns a minecart (id 5) at (0.5, 64.1, 0.5) in a level with `features`, sends it one
/// window of track steps and ticks the store once; returns the cart's position.
fn cart_after_one_tick(kind: RegistryIndex, features: &[&str]) -> DVec3 {
    let regs = registries(features);
    let mut session = Session::new(
        EntityId(7),
        GameMode::Creative,
        DimensionInfo::overworld(),
        std::sync::Arc::new(Vec::new()),
    );
    let mut store: EntityStore = new_entity_store(256);
    let mut out = Vec::new();
    let start = DVec3::new(0.5, 64.1, 0.5);
    session.apply(
        ClientboundPlay::AddEntity(cb::AddEntity {
            entity_id: 5,
            uuid: Default::default(),
            kind: kind.0 as i32,
            pos: start,
            velocity: DVec3::ZERO,
            pitch: 0.0,
            yaw: 0.0,
            head_yaw: 0.0,
            data: 0,
        }),
        &mut EmptyWorld,
        &mut store,
        &regs,
        &mut out,
    );
    let step = |x: f64, weight: f32| cb::MinecartStep {
        pos: DVec3::new(x, 64.1, 0.5),
        velocity: DVec3::new(0.3, 0.0, 0.0),
        yaw: 180.0,
        pitch: 0.0,
        weight,
    };
    session.apply(
        ClientboundPlay::MoveMinecartAlongTrack(cb::MoveMinecartAlongTrack {
            entity_id: 5,
            steps: vec![step(0.8, 0.3), step(1.4, 0.6)],
        }),
        &mut EmptyWorld,
        &mut store,
        &regs,
        &mut out,
    );
    store.tick();
    store.get(EntityId(5)).unwrap().pos
}

#[test]
fn a_new_behaviour_cart_walks_the_packets_steps() {
    let Some(kind) = minecart_kind() else {
        eprintln!("skipped: no data-generator report in the cache");
        return;
    };
    // A third of the window's weight (0.9) is 0.3: the end of the first step.
    let pos = cart_after_one_tick(kind, &["vanilla", "minecart_improvements"]);
    assert!(
        (pos - DVec3::new(0.8, 64.1, 0.5)).length() < 1e-5,
        "{pos:?}"
    );
    // Without the feature the cart runs the old behaviour and ignores the steps.
    let pos = cart_after_one_tick(kind, &["vanilla"]);
    assert_eq!(pos, DVec3::new(0.5, 64.1, 0.5));
}
