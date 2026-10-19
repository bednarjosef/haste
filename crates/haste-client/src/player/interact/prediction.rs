//! Block-state prediction (player workstream): vanilla `BlockStatePredictionHandler` —
//! what our own `use_item_on` places and our destroy `player_action`s break is set in the
//! client world at once, and the server's state of each predicted block is retained until
//! its `block_changed_ack` releases it.
//!
//! Vanilla runs every `useItemOn` / `useItem` / destroy action inside `startPrediction`
//! (`MultiPlayerGameMode`): the packet carries a sequence number, and any `ClientLevel
//! .setBlock` under it (a placed block, a broken block) goes through
//! `retainKnownServerState`, which remembers the state the server had there and where the
//! player stood. Server `block_update`s for a retained position only update the retained
//! state (`ClientLevel.setServerVerifiedBlockState` -> `updateKnownServerState`); the ack
//! (`endPredictionsUpTo`) then restores it with `ClientLevel.syncBlockState`: the block is
//! set only when it really changed, and the player is snapped back to the retained
//! position — but only when the server's state collides with the player and no teleport
//! (`onTeleport`) has intervened since the prediction.
//!
//! The snap is what makes vanilla bridging seamless: a placement the server rejects leaves
//! the client standing where it was, on a block the server says is not there, instead of
//! inside it. The snap-back application (the `ClientLevel.syncBlockState` half) runs in the
//! net driver, which owns the world (`Session::apply`); this module only retains and
//! releases.

use glam::DVec3;
use haste_core::{BlockPos, BlockStateId};

use super::super::LocalPlayer;

/// One retained server-verified state (vanilla `BlockStatePredictionHandler
/// .ServerVerifiedState`): the server's state of a predicted block and where the player
/// stood when the first prediction for it was made.
#[derive(Clone, Copy, Debug)]
pub(crate) struct VerifiedState {
    /// The sequence of the last prediction for the block (bumped by a new one).
    pub(crate) sequence: i32,
    /// The server's state there: the state before our first prediction, then the latest
    /// block update the server sent while the prediction was open.
    pub(crate) server_state: BlockStateId,
    /// The player's position when the first prediction for the block was made; the
    /// snap-back target (`ClientLevel.syncBlockState`'s `playerPos`).
    pub(crate) player_pos: DVec3,
}

/// One acked prediction's restore, for the driver to apply (vanilla `ClientLevel
/// .syncBlockState`'s inputs).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sync {
    pub(crate) pos: BlockPos,
    /// The server's state to restore.
    pub(crate) server_state: BlockStateId,
    /// Where the player that made the prediction stood, when it may still be snapped back
    /// there (no teleport since the prediction; vanilla's `lastTeleportSequence` check).
    pub(crate) player_pos: Option<DVec3>,
}

/// Vanilla `BlockStatePredictionHandler`: the client-side block predictions and the
/// server states they hold back. One retained state per predicted position (vanilla keys
/// its map by `BlockPos.asLong`); a new prediction for the same block only bumps its
/// sequence, like vanilla's `retainKnownServerState`.
#[derive(Default)]
pub struct BlockStatePrediction {
    /// The last sequence handed out (`currentSequenceNr`); 0 before any prediction.
    sequence: i32,
    /// The sequence at the last `player_position` teleport (`lastTeleportSequence`); -1
    /// means none arrived yet. Predictions from before a teleport never snap the player
    /// back: the retained position is stale through the teleport.
    last_teleport_sequence: i32,
    /// The retained server states, one per predicted position. Only touched when a
    /// prediction is made, a server update arrives for one, or an ack releases some —
    /// nothing here runs per tick.
    retained: Vec<(BlockPos, VerifiedState)>,
    /// Predicted blocks to apply to the client world now (the local half of a
    /// prediction; vanilla's `ClientLevel.setBlock` under `startPredicting`). Drained by
    /// the driver each tick (`take_world_edits`).
    world_edits: Vec<(BlockPos, BlockStateId)>,
    /// Syncs released by the last `block_changed_ack` and not yet applied.
    pending_syncs: Vec<Sync>,
}

impl BlockStatePrediction {
    /// `BlockStatePredictionHandler.startPredicting`'s sequence half: the next sequence
    /// number, counting up once per predicted action (vanilla increments before every
    /// use/destroy packet, predicting or not).
    pub(crate) fn start_predicting(&mut self) -> i32 {
        self.sequence += 1;
        self.sequence
    }

    /// `retainKnownServerState`: remember the server's state of `pos` (the state before
    /// our edit, or the last one the server told us) and where the player stood. An
    /// earlier prediction for the same block keeps its retained state and position and
    /// only takes the new sequence, like vanilla's `compute`.
    pub(crate) fn retain(
        &mut self,
        seq: i32,
        pos: BlockPos,
        server_state: BlockStateId,
        player_pos: DVec3,
    ) {
        match self.retained.iter_mut().find(|(p, _)| *p == pos) {
            Some((_, v)) => v.sequence = seq,
            None => self.retained.push((
                pos,
                VerifiedState {
                    sequence: seq,
                    server_state,
                    player_pos,
                },
            )),
        }
    }

    /// The server state retained for `pos`, when a prediction is open for it.
    pub(crate) fn retained_server_state(&self, pos: BlockPos) -> Option<BlockStateId> {
        self.retained
            .iter()
            .find(|(p, _)| *p == pos)
            .map(|(_, v)| v.server_state)
    }

    /// The local half of a prediction: `pos` becomes `predicted` in the client world at
    /// once (physics and the renderer see it this tick), and the server's state there is
    /// retained until the ack. `server_state` is what the world has there now, before the
    /// edit. Vanilla: `ClientLevel.setBlock` inside `startPredicting` ->
    /// `retainKnownServerState` + the block change.
    pub(crate) fn set_predicted(
        &mut self,
        seq: i32,
        pos: BlockPos,
        server_state: BlockStateId,
        player_pos: DVec3,
        predicted: BlockStateId,
    ) {
        self.retain(seq, pos, server_state, player_pos);
        self.world_edits.push((pos, predicted));
    }

    /// `updateKnownServerState`: a server block update for a predicted position only
    /// updates the retained state (it is applied on the ack); `true` when it was held
    /// back. `false` lets the driver apply it at once.
    pub(crate) fn update_known_server_state(&mut self, pos: BlockPos, state: BlockStateId) -> bool {
        match self.retained.iter_mut().find(|(p, _)| *p == pos) {
            Some((_, v)) => {
                v.server_state = state;
                true
            }
            None => false,
        }
    }

    /// Whether a prediction is still open for `pos` (a broken block is gone for us until
    /// its ack even before the world edit applies).
    pub(crate) fn is_predicted(&self, pos: BlockPos) -> bool {
        self.retained.iter().any(|(p, _)| *p == pos)
    }

    /// `endPredictionsUpTo` (the `block_changed_ack`): every prediction the ack covers
    /// releases its server state; the rest (made after the ack's sequence) stay held.
    /// The player position is only offered to the sync when no teleport intervened
    /// (`lastTeleportSequence < sequence`).
    pub(crate) fn end_predictions_up_to(&mut self, ack: i32) {
        let mut i = 0;
        while i < self.retained.len() {
            if self.retained[i].1.sequence > ack {
                i += 1;
                continue;
            }
            let (pos, v) = self.retained.remove(i);
            self.pending_syncs.push(Sync {
                pos,
                server_state: v.server_state,
                player_pos: (self.last_teleport_sequence < v.sequence).then_some(v.player_pos),
            });
        }
    }

    /// `onTeleport` (a `player_position` arrived): the player position retained by
    /// predictions from before this point is stale — those never snap the player back.
    pub(crate) fn on_teleport(&mut self) {
        self.last_teleport_sequence = self.sequence;
    }

    /// The predicted blocks to apply to the world now (drained by the driver).
    pub fn take_world_edits(&mut self) -> std::vec::Drain<'_, (BlockPos, BlockStateId)> {
        self.world_edits.drain(..)
    }

    /// The syncs the last `block_changed_ack` released, for the driver's
    /// `ClientLevel.syncBlockState`.
    pub(crate) fn take_syncs(&mut self) -> std::vec::Drain<'_, Sync> {
        self.pending_syncs.drain(..)
    }

    /// A respawn: vanilla builds a new `ClientLevel` (and with it a new handler), so no
    /// prediction survives one.
    pub(crate) fn clear(&mut self) {
        self.retained.clear();
        self.world_edits.clear();
        self.pending_syncs.clear();
    }
}

impl LocalPlayer {
    /// `Entity.absSnapTo`: position without interpolation (the previous position pinned to
    /// the new one, so nothing lerps through the gap), clamped to the world like vanilla.
    /// The snap-back of `ClientLevel.syncBlockState` runs through the net driver, which
    /// owns the world (`net/sync_block_state.rs`).
    pub(crate) fn abs_snap_to(&mut self, pos: DVec3) {
        const LIMIT: f64 = 3.0e7;
        let pos = DVec3::new(
            pos.x.clamp(-LIMIT, LIMIT),
            pos.y,
            pos.z.clamp(-LIMIT, LIMIT),
        );
        self.prev_pos = pos;
        self.state.pos = pos;
    }
}
