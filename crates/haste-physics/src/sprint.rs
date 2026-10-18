//! Sprinting and sprint-swimming (vanilla `LocalPlayer.aiStep`'s sprint rules and
//! `Player.updateSwimming` / `Entity.updateSwimming`).
use haste_core::input::InputState;
use haste_core::view::BlockView;
use haste_core::BlockPos;

use crate::{BlockPhysics, FluidKind, PlayerState, Pose};

/// Vanilla `Player.updateSwimming` (from `Entity.baseTick`, before `aiStep` reads the keys):
/// never while flying; an established swim lasts while sprinting in water, a new one needs the
/// sprint under water (`isUnderWater`: the eyes in water and in water) with water at the feet
/// block. It reads the sprint flag of the previous tick, as vanilla does.
pub(crate) fn update_swimming(
    state: &mut PlayerState,
    under_water: bool,
    world: &dyn BlockView,
    blocks: &dyn BlockPhysics,
) {
    state.swimming = if state.flying {
        false
    } else if state.swimming {
        state.sprinting && state.in_water
    } else {
        state.sprinting
            && under_water
            && blocks
                .fluid(world.block_state(BlockPos::from_dvec3(state.pos)))
                .is_some_and(|f| f.kind == FluidKind::Water)
    };
}

/// Vanilla `LocalPlayer.aiStep`'s sprint step, after the crouch flag: `canStartSprinting` with
/// the sprint key starts it (the double tap of forward reaches here as `input.sprint` too), then
/// a sprinting player stops on `shouldStopSwimSprinting` while swimming and on
/// `shouldStopRunSprinting` otherwise.
pub(crate) fn update_sprinting(state: &mut PlayerState, input: &InputState, under_water: bool) {
    let forward = input.forward && !input.back;
    if input.sprint && can_start_sprinting(state, forward, under_water) {
        state.sprinting = true;
    }
    if state.sprinting {
        let stop = if state.swimming {
            !sprinting_possible(state, under_water, true)
                || !state.in_water
                || (!forward && !state.on_ground && !input.sneak)
        } else {
            !sprinting_possible(state, under_water, state.flying)
                || !forward
                || (state.horizontal_collision && !state.minor_horizontal_collision)
        };
        if stop {
            state.sprinting = false;
        }
    }
}

/// Vanilla `LocalPlayer.canStartSprinting`: not sprinting, a forward impulse, sprinting
/// possible, not slowed by the item in use, not gliding and not moving slowly (`isMovingSlowly`:
/// crouching, or the swimming pose out of water) unless under water (`isUnderWater`: the eyes
/// in water and in water). The client's double tap of forward asks the same question.
pub fn can_start_sprinting(state: &PlayerState, forward: bool, under_water: bool) -> bool {
    let moving_slowly = state.crouching || (state.pose == Pose::Swimming && !state.in_water);
    !state.sprinting
        && forward
        && sprinting_possible(state, under_water, state.flying)
        && !state.slow_using_item
        && (!state.fall_flying || under_water)
        && (!moving_slowly || under_water)
}

/// Vanilla `LocalPlayer.isSprintingPossible`: not blind and fed (or may fly; the caller folds
/// both into `sprint_blocked`), and out of shallow water (`isInShallowWater`: in water with
/// the eyes out) unless `allowed_in_shallow_water`.
fn sprinting_possible(
    state: &PlayerState,
    under_water: bool,
    allowed_in_shallow_water: bool,
) -> bool {
    let in_shallow_water = state.in_water && !under_water;
    !state.sprint_blocked && (allowed_in_shallow_water || !in_shallow_water)
}

#[cfg(test)]
mod tests;
