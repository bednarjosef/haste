//! `ClientLevel.syncBlockState` (net driver): the restore half of a block-state
//! prediction, run when `block_changed_ack` releases it
//! (`BlockStatePredictionHandler.endPredictionsUpTo`). The world lives here, not in the
//! player, so the player's `handle_interact_packet` only ends the predictions
//! (`interact/prediction.rs`) and this module applies what they released:
//!
//! - the server's state wins, set only when it really differs from what the client
//!   world has (a matching state keeps the prediction's own local edit, remesh and all);
//! - the player is snapped back to the position it made the prediction from, but only
//!   when the server's state collides with it (`Entity.isColliding`) — a rejected
//!   placement would otherwise leave the player inside a block the server says is not
//!   there — and never when a `player_position` teleport intervened (the prediction
//!   module already drops the retained position then).

use haste_core::view::{BlockView, CollisionShapes};
use haste_core::{Aabb, BlockPos, BlockStateId};
use haste_data::GameData;

use super::{Session, WorldSink};
use crate::player::interact::prediction::Sync;

impl Session {
    /// Applies the syncs the last `block_changed_ack` released (the caller drains them
    /// off the player right after its own packet handling, where the ack ends them).
    pub(super) fn sync_block_state<W: WorldSink + ?Sized>(&mut self, world: &mut W) {
        // Only the ack fills it, never per tick; the loop body borrows `self`.
        let syncs: Vec<Sync> = self.player.take_prediction_syncs().collect();
        for sync in syncs {
            self.sync_one(world, sync);
        }
    }

    fn sync_one<W: WorldSink + ?Sized>(&mut self, world: &mut W, sync: Sync) {
        if world.block_state_at(sync.pos) == sync.server_state {
            return;
        }
        self.set_block_with_block_entities(world, sync.pos, sync.server_state);
        let Some(from) = sync.player_pos else {
            return;
        };
        // `player.isColliding(pos, state)` after the setBlock, as vanilla runs it. The
        // test fakes carry no physics tables: without them the check cannot run, and the
        // snap is skipped rather than guessed.
        let Some((view, data)) = world.physics() else {
            return;
        };
        if player_collides(&self.player, view, data, sync.pos, sync.server_state) {
            self.player.abs_snap_to(from);
        }
    }
}

/// `Entity.isColliding(pos, state)`: the entity's box intersects the state's collision
/// shape at `pos`.
fn player_collides(
    player: &crate::player::LocalPlayer,
    view: &dyn BlockView,
    data: &GameData,
    pos: BlockPos,
    state: BlockStateId,
) -> bool {
    let boxes = data.shapes.collision_boxes(state);
    if boxes.is_empty() {
        return false;
    }
    let _ = view;
    let box_ = Aabb::entity(
        player.state.pos,
        player.attrs.box_width(),
        player.attrs.height_for(player.state.pose),
    );
    boxes.iter().any(|s| {
        box_.intersects(&Aabb {
            min: s.min + pos.as_dvec3(),
            max: s.max + pos.as_dvec3(),
        })
    })
}

#[cfg(test)]
mod tests;
