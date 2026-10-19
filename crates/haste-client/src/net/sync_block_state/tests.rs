//! `ClientLevel.syncBlockState` tests: the ack's restore sets the server's state only
//! when it differs, and snaps the player back only when that state collides with it
//! (`Entity.isColliding`) — the two halves vanilla gates with `oldState != state`.
//! Needs the game data's collision shapes from the cache.

use std::sync::{Arc, OnceLock};

use glam::DVec3;
use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId};
use haste_data::GameData;
use haste_physics::testing::TestWorld;
use haste_protocol::haste_core::chunk::{BlockEntityData, ChunkData, LightData};
use haste_protocol::haste_core::game::{DimensionInfo, GameMode};
use haste_protocol::haste_core::{ChunkPos, EntityId};
use haste_protocol::packets::play::clientbound::BlockChangedAck;
use haste_protocol::{ClientboundPlay, SyncedRegistries};

use super::super::sinks::WorldSink;
use super::super::Session;

/// The real game data (block states and collision shapes) once per process; `None`
/// without the cache (the collision half cannot be tested then).
fn game_data() -> Option<&'static GameData> {
    static D: OnceLock<Option<GameData>> = OnceLock::new();
    D.get_or_init(|| GameData::load().ok())
        .as_ref()
        .filter(|d| d.blocks.len() > 1000)
}

fn stone() -> BlockStateId {
    game_data()
        .and_then(|d| {
            d.blocks
                .default_state(&haste_core::Identifier::minecraft("stone"))
        })
        .expect("stone in the game data")
}

/// A world the driver can drive: a `TestWorld` behind the sink, with the game data for
/// the collision check, recording every `set_block` the sync runs.
struct SyncWorld {
    view: TestWorld,
    data: &'static GameData,
    sets: Vec<(BlockPos, BlockStateId)>,
}

impl WorldSink for SyncWorld {
    fn load_chunk(&mut self, _data: &ChunkData) -> anyhow::Result<()> {
        Ok(())
    }
    fn unload_chunk(&mut self, _pos: ChunkPos) {}
    fn set_block(&mut self, pos: BlockPos, state: BlockStateId) {
        self.sets.push((pos, state));
        self.view.set(pos.x, pos.y, pos.z, state);
    }
    fn apply_light(&mut self, _pos: ChunkPos, _light: &LightData) {}
    fn set_block_entity(&mut self, _pos: BlockPos, _data: BlockEntityData) {}
    fn block_state_at(&self, pos: BlockPos) -> BlockStateId {
        self.view.block_state(pos)
    }
    fn reset(&mut self, _dim: &DimensionInfo) {}
    fn chunk_count(&self) -> usize {
        1
    }
    fn non_empty_sections(&self) -> usize {
        1
    }
    fn physics(&self) -> Option<(&dyn BlockView, &GameData)> {
        Some((&self.view, self.data))
    }
}

/// One driven ack: a prediction is open for `pos` (the server's state there
/// `retained`, our predicted edit `predicted` already in the world, made while
/// standing at `predicted_at`), the player is at `now_at` when the ack arrives.
fn rig(
    pos: BlockPos,
    retained: BlockStateId,
    predicted: BlockStateId,
    predicted_at: DVec3,
    now_at: DVec3,
) -> Option<(Session, SyncWorld)> {
    let mut session = Session::new(
        EntityId(7),
        GameMode::Survival,
        DimensionInfo::overworld(),
        Arc::new(Vec::new()),
    );
    session
        .player
        .int
        .prediction
        .set_predicted(1, pos, retained, predicted_at, predicted);
    // The tick's own edit application took the predicted state into the world already
    // (the view below holds it); nothing of it is left to drain at the ack.
    let _ = session.player.take_world_edits().count();
    session.player.state.pos = now_at;
    session.player.prev_pos = now_at;
    let mut world = SyncWorld {
        view: TestWorld::new(),
        data: game_data()?,
        sets: Vec::new(),
    };
    world.view.set(pos.x, pos.y, pos.z, predicted);
    let mut entities = super::super::new_entity_store(0);
    let mut out = Vec::new();
    session.apply(
        ClientboundPlay::BlockChangedAck(BlockChangedAck { sequence: 1 }),
        &mut world,
        &mut entities,
        &SyncedRegistries::default(),
        &mut out,
    );
    Some((session, world))
}

/// The server's state differs and the player collides with it: the block is restored
/// and the player snaps back to the position the prediction retained (a rejected break
/// of the block under us must not leave us inside the floor the server still has).
#[test]
fn a_colliding_restore_snaps_the_player_back() {
    let Some((session, world)) = rig(
        BlockPos::new(0, 0, 0),
        stone(),                   // what the server has there
        BlockStateId::AIR,         // our break predicted it gone
        DVec3::new(0.5, 1.0, 0.5), // where we stood when breaking it
        DVec3::new(0.5, 0.2, 0.5), // we fell into the hole before the ack
    ) else {
        eprintln!("no game data in the cache: the snap-back cannot be tested");
        return;
    };
    assert_eq!(world.sets, [(BlockPos::new(0, 0, 0), stone())]);
    assert_eq!(
        session.player.state.pos,
        DVec3::new(0.5, 1.0, 0.5),
        "the snap-back to the retained position"
    );
    assert_eq!(session.player.prev_pos, session.player.state.pos);
}

/// The server's state differs but the player does not collide with it: the block is
/// restored, the player keeps its position.
#[test]
fn a_non_colliding_restore_sets_the_block_without_a_snap() {
    let Some((session, world)) = rig(
        BlockPos::new(0, 0, 0),
        stone(),
        BlockStateId::AIR,
        DVec3::new(0.5, 1.0, 0.5),
        DVec3::new(0.5, 5.0, 0.5), // clear of the cell
    ) else {
        eprintln!("no game data in the cache: the snap-back cannot be tested");
        return;
    };
    assert_eq!(world.sets, [(BlockPos::new(0, 0, 0), stone())]);
    assert_eq!(session.player.state.pos, DVec3::new(0.5, 5.0, 0.5));
}

/// The server's state matches what the world has (our prediction was right): nothing is
/// set and nothing snaps, even with the player inside the block — vanilla's
/// `oldState != state` gate.
#[test]
fn a_matching_restore_touches_neither_block_nor_player() {
    let Some((session, world)) = rig(
        BlockPos::new(0, 0, 0),
        stone(),
        stone(),
        DVec3::new(0.5, 1.0, 0.5),
        DVec3::new(0.5, 0.2, 0.5), // inside the cell, but the states agree
    ) else {
        eprintln!("no game data in the cache: the snap-back cannot be tested");
        return;
    };
    assert!(world.sets.is_empty(), "{:?}", world.sets);
    assert_eq!(session.player.state.pos, DVec3::new(0.5, 0.2, 0.5));
}
