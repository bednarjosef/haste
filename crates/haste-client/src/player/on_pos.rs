//! Vanilla `Entity.getOnPosLegacy`: the block the player stands on, for step and fall sounds.

use glam::DVec3;
use haste_core::view::BlockView;
use haste_core::BlockPos;

/// The block supporting the player at `pos` (vanilla `Entity.getOnPosLegacy`:
/// the block at feet y minus 0.2), for step and fall sounds.
pub(super) fn supporting_block_state(
    pos: DVec3,
    world: &dyn BlockView,
) -> Option<haste_core::BlockStateId> {
    let p = BlockPos::new(
        pos.x.floor() as i32,
        (pos.y - 0.2).floor() as i32,
        pos.z.floor() as i32,
    );
    let state = world.block_state(p);
    (state != haste_core::BlockStateId::AIR).then_some(state)
}
