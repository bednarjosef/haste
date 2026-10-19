//! Prediction tests (vanilla `BlockStatePredictionHandler`): a break and a place are set
//! in the client world at once, server updates for the predicted position are held back
//! until the `block_changed_ack`, and the ack releases the server's state for the net
//! driver's `ClientLevel.syncBlockState` (the restore and the snap-back, which needs the
//! world, live in `net/sync_block_state.rs` — its tests cover that half).

use haste_core::game::GameMode;
use haste_core::input::{InputState, InteractInput};
use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId};
use haste_protocol::packets::play::clientbound::BlockChangedAck;
use haste_protocol::{ClientboundPlay, ServerboundPlay};

use super::{data, item, Rig, ATTACK, STONE, USE};
use crate::player::interact::InteractEnv;

/// `minecraft:dirt` in the test registry (see `tests::data`): a block item.
const DIRT_ITEM: u32 = 12;
const DIRT: BlockStateId = BlockStateId(2);
/// The cell a pitch-45 placement puts the block in (see `click_results`: the ray from
/// the eye at (0.5, 1.62, 0.5) crosses y = 0 at z = 2.12, beside the player).
const BESIDE: BlockPos = BlockPos::new(0, 0, 2);

/// Whether placement prediction can run here: `minecraft:dirt` must resolve to plain
/// class `Block` in the `block_classes.json` dump (the cache's oracle dir). Without the
/// dump nothing is predicted and these tests have nothing to check.
fn dirt_is_covered() -> bool {
    crate::player::interact::block_classes::placement_class("minecraft:dirt")
        == Some(crate::player::interact::block_classes::PlacementClass::Block)
}

/// One rig tick with the crosshair at pitch 45 (the placement goes beside the player),
/// keeping the movement packets: the packet-order test reads the whole tick's out.
fn tick_pitch_45(r: &mut Rig, i: InteractInput) -> Vec<ServerboundPlay> {
    r.p.set_interact(i);
    let mut out = Vec::new();
    let env = InteractEnv {
        data: data(),
        entities: &r.entities,
        entity_types: &r.types,
        border: r.border.as_ref(),
        warm_blocks: None,
        ui: None,
    };
    let input = InputState {
        pitch: 45.0 - r.p.look_offset().1,
        ..Default::default()
    };
    r.p.tick_interact(&input, &r.w, &env, None, &mut out);
    for (pos, s) in r.p.take_world_edits().collect::<Vec<_>>() {
        r.w.set(pos.x, pos.y, pos.z, s);
    }
    out
}

/// The syncs an ack released, as (pos, server state, snap-back position offered).
fn ack(r: &mut Rig, sequence: i32) -> Vec<(BlockPos, BlockStateId, bool)> {
    let mut out = Vec::new();
    r.p.handle(
        &ClientboundPlay::BlockChangedAck(BlockChangedAck { sequence }),
        &mut out,
    );
    r.p.take_prediction_syncs()
        .map(|s| (s.pos, s.server_state, s.player_pos.is_some()))
        .collect()
}

/// A break's prediction: server updates for the broken block are held back until the
/// ack, and the ack releases the server's state (the last update it sent, or the state
/// before our prediction when it said nothing).
#[test]
fn a_breaks_server_updates_wait_for_the_ack() {
    let mut r = Rig::new(GameMode::Creative);
    r.tick(ATTACK);
    let pos = BlockPos::new(0, -1, 0);
    assert_eq!(r.w.block_state(pos), BlockStateId::AIR);
    // A stale server update for the broken block arrives before the ack: held back,
    // only recorded.
    assert!(r.p.defer_block_update(pos, STONE));
    assert!(r.p.take_world_edits().next().is_none());
    assert_eq!(ack(&mut r, 1), [(pos, STONE, true)]);
    // After the ack, updates go straight to the world again.
    assert!(!r.p.defer_block_update(pos, BlockStateId::AIR));
}

/// A break the server rejects with no block update: the ack still restores what the
/// server has (the state before our prediction), so our world does not keep a block the
/// server never lost.
#[test]
fn a_silently_rejected_break_is_reverted_on_the_ack() {
    let mut r = Rig::new(GameMode::Creative);
    r.tick(ATTACK);
    assert_eq!(ack(&mut r, 1), [(BlockPos::new(0, -1, 0), STONE, true)]);
}

/// A place's prediction: the block is in the client world the same tick, and the ack
/// releases the state the cell had before it. The sequence is the `use_item_on`
/// packet's, so the server's ack covers exactly our prediction.
#[test]
fn a_place_predicts_the_block_until_the_ack() {
    if !dirt_is_covered() {
        eprintln!("no block_classes.json in the oracle dir: placement is not predicted");
        return;
    }
    let mut r = Rig::new(GameMode::Survival);
    r.p.int.hotbar[0] = item(DIRT_ITEM);
    let out = tick_pitch_45(&mut r, USE);
    let seq = out
        .iter()
        .find_map(|p| match p {
            ServerboundPlay::UseItemOn(u) => Some(u.sequence),
            _ => None,
        })
        .expect("the use sent use_item_on");
    // The block is there for physics and the renderer at once.
    assert_eq!(r.w.block_state(BESIDE), DIRT);
    // A server update for the predicted cell is held back, not applied.
    assert!(r.p.defer_block_update(BESIDE, STONE));
    assert!(r.p.take_world_edits().next().is_none());
    // The ack releases the server's state and offers the snap-back position (we stood
    // at the rig's spawn when the prediction was made).
    let syncs = ack(&mut r, seq);
    assert_eq!(syncs.len(), 1);
    assert_eq!(syncs[0].0, BESIDE);
    assert_eq!(
        syncs[0].1, STONE,
        "the held-back update is the state restored"
    );
    assert!(syncs[0].2, "no teleport intervened");
}

/// A prediction whose ack arrives after a `player_position` teleport never offers the
/// snap-back: the retained position is stale through the teleport
/// (`BlockStatePredictionHandler.onTeleport`).
#[test]
fn a_teleport_between_prediction_and_ack_drops_the_snap_back() {
    if !dirt_is_covered() {
        eprintln!("no block_classes.json in the oracle dir: placement is not predicted");
        return;
    }
    let mut r = Rig::new(GameMode::Survival);
    r.p.int.hotbar[0] = item(DIRT_ITEM);
    let out = tick_pitch_45(&mut r, USE);
    let seq = out
        .iter()
        .find_map(|p| match p {
            ServerboundPlay::UseItemOn(u) => Some(u.sequence),
            _ => None,
        })
        .expect("the use sent use_item_on");
    // The server corrects us before the ack.
    let mut teleport_out = Vec::new();
    r.p.handle(
        &ClientboundPlay::PlayerPosition(
            haste_protocol::packets::play::clientbound::PlayerPosition {
                teleport_id: 9,
                pos: glam::DVec3::new(30.0, 80.0, 30.0),
                velocity: glam::DVec3::ZERO,
                yaw: 0.0,
                pitch: 45.0,
                flags: 0,
            },
        ),
        &mut teleport_out,
    );
    let syncs = ack(&mut r, seq);
    assert_eq!(syncs.len(), 1);
    assert!(!syncs[0].2, "the teleport made the retained position stale");
}

/// Packet order within a tick: the use click's `use_item_on` leaves before this tick's
/// movement packets (vanilla `Minecraft.tick` runs `handleKeybinds` before the player
/// tick's `sendPosition`), so the server places the block before moving the player —
/// without it every placement while moving would rubber-band.
#[test]
fn the_use_item_on_precedes_the_movements_of_its_tick() {
    if !dirt_is_covered() {
        eprintln!("no block_classes.json in the oracle dir: placement is not predicted");
        return;
    }
    let mut r = Rig::new(GameMode::Survival);
    r.p.int.hotbar[0] = item(DIRT_ITEM);
    let out = tick_pitch_45(&mut r, USE);
    let use_at = out
        .iter()
        .position(|p| matches!(p, ServerboundPlay::UseItemOn(_)))
        .expect("the use sent use_item_on");
    let move_at = out
        .iter()
        .position(|p| {
            matches!(
                p,
                ServerboundPlay::MovePlayerPos(_)
                    | ServerboundPlay::MovePlayerPosRot(_)
                    | ServerboundPlay::MovePlayerRot(_)
                    | ServerboundPlay::MovePlayerStatusOnly(_)
            )
        })
        .expect("the tick sent a movement packet");
    assert!(use_at < move_at, "{out:?}");
}
