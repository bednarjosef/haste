//! Climbing (ladders, vines, scaffolding, an open trapdoor over a ladder): vanilla
//! `LivingEntity.onClimbable`, `handleOnClimbable` and the climb boost in
//! `handleRelativeFrictionAndCalculateMovement` / `travelInFluid`.
//!
//! Which blocks are climbable is data (`#minecraft:climbable`, synced by the server with the
//! other block tags), so the tick asks a [`ClimbableBlocks`] for it. The game wires its tag table
//! in; [`NoClimbables`] stands in where nothing is known (then nothing climbs, like before).
use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId, DVec3};

use crate::PlayerState;

/// Horizontal speed cap and the fall speed floor on a climbable (vanilla
/// `LivingEntity.handleOnClimbable`: `0.15F`, widened to double where vanilla clamps doubles).
pub const CLIMB_SPEED_LIMIT: f64 = 0.15f32 as f64;
/// Upward speed set after the move when colliding horizontally or holding jump on a climbable
/// (`LivingEntity.handleRelativeFrictionAndCalculateMovement`, `travelInFluid`).
pub const CLIMB_UP_SPEED: f64 = 0.2;

/// Block identity for climbing, answered from the game's tag table and block states.
pub trait ClimbableBlocks {
    /// `BlockTags.CLIMBABLE` (`#minecraft:climbable`).
    fn is_climbable(&self, state: BlockStateId) -> bool;
    /// `Blocks.SCAFFOLDING` (`minecraft:scaffolding`): sneaking slides down it.
    fn is_scaffolding(&self, state: BlockStateId) -> bool {
        let _ = state;
        false
    }
    /// `LivingEntity.trapdoorUsableAsLadder`: `state` is a trapdoor (`TrapDoorBlock`) that is
    /// open, and `below` is `minecraft:ladder` with the same `facing`.
    fn trapdoor_usable_as_ladder(&self, state: BlockStateId, below: BlockStateId) -> bool {
        let _ = (state, below);
        false
    }
    /// `BlockTags.CAN_GLIDE_THROUGH` (`#minecraft:can_glide_through`): a glider passes through
    /// these climbables instead of being stopped.
    fn can_glide_through(&self, state: BlockStateId) -> bool {
        let _ = state;
        false
    }
}

/// Nothing is climbable (no tag table available).
pub struct NoClimbables;
impl ClimbableBlocks for NoClimbables {
    fn is_climbable(&self, _state: BlockStateId) -> bool {
        false
    }
}

/// The block the player is in (vanilla `Entity.getInBlockState`: the state at `blockPosition()`,
/// the floor of the feet position).
pub(crate) fn in_block_pos(state: &PlayerState) -> BlockPos {
    BlockPos::from_dvec3(state.pos)
}

/// Vanilla `LivingEntity.onClimbable`: never for a spectator; a glider is not stopped by a
/// climbable it can glide through; else the in-block state is in `#climbable` or is an open
/// trapdoor over a ladder facing the same way.
pub fn on_climbable(
    state: &PlayerState,
    world: &dyn BlockView,
    climb: &dyn ClimbableBlocks,
) -> bool {
    if state.spectating {
        return false;
    }
    let pos = in_block_pos(state);
    let in_block = world.block_state(pos);
    if state.fall_flying && climb.can_glide_through(in_block) {
        return false;
    }
    climb.is_climbable(in_block)
        || climb.trapdoor_usable_as_ladder(in_block, world.block_state(pos.add(0, -1, 0)))
}

/// Vanilla `LivingEntity.handleOnClimbable`, applied to the velocity between the acceleration
/// and the move: on a climbable the fall distance resets, x and z are clamped to the climb speed
/// limit, the fall is floored at it, and a player holding shift (and not flying:
/// `LocalPlayer.isSuppressingSlidingDownLadder`) on anything but scaffolding does not slide.
pub(crate) fn handle_on_climbable(
    state: &mut PlayerState,
    world: &dyn BlockView,
    climb: &dyn ClimbableBlocks,
) {
    if !on_climbable(state, world, climb) {
        return;
    }
    state.fall_distance = 0.0;
    let v = state.velocity;
    let mut y = v.y.max(-CLIMB_SPEED_LIMIT);
    if y < 0.0
        && !climb.is_scaffolding(world.block_state(in_block_pos(state)))
        && state.sneaking
        && !state.flying
    {
        y = 0.0;
    }
    state.velocity = DVec3::new(
        v.x.clamp(-CLIMB_SPEED_LIMIT, CLIMB_SPEED_LIMIT),
        y,
        v.z.clamp(-CLIMB_SPEED_LIMIT, CLIMB_SPEED_LIMIT),
    );
}

/// The climb boost after the move (vanilla `LivingEntity.handleRelativeFrictionAndCalculateMovement`
/// in air with `jumping` = the jump key, `travelInFluid` in water without it): colliding
/// horizontally with a climbable (or holding jump on one) sets the upward speed to 0.2, before
/// gravity and drag. The powder-snow-with-leather-boots case of the same rule is not modelled.
pub(crate) fn climb_after_move(
    state: &mut PlayerState,
    jumping: bool,
    world: &dyn BlockView,
    climb: &dyn ClimbableBlocks,
) {
    if (state.horizontal_collision || jumping) && on_climbable(state, world, climb) {
        state.velocity.y = CLIMB_UP_SPEED;
    }
}
