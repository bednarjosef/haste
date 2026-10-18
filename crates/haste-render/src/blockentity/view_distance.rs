//! Vanilla `BlockEntityRenderer.shouldRender`: the per-renderer view distance that bounds
//! which block entities draw at all.

use super::BlockEntityKind;
use haste_core::BlockPos;

/// A block entity only draws within its renderer's view distance of its block's centre
/// (`Vec3.atCenterOf`, +0.5 on every axis; `closerThan` is strict). 64 blocks by default
/// (`BlockEntityRenderer.getViewDistance`), 68 for the moving piston
/// (`PistonHeadRenderer.getViewDistance`); the beacon is measured horizontally out to the
/// effective render distance in blocks (`BeaconRenderer.shouldRender` zeroes both y,
/// `getViewDistance` is `getEffectiveRenderDistance() * 16`).
pub fn within_view_distance(
    kind: &BlockEntityKind,
    pos: BlockPos,
    camera: glam::DVec3,
    render_distance_blocks: f64,
) -> bool {
    let centre = pos.as_dvec3() + glam::DVec3::new(0.5, 0.5, 0.5);
    match kind {
        BlockEntityKind::Beacon => {
            let dx = centre.x - camera.x;
            let dz = centre.z - camera.z;
            dx * dx + dz * dz < render_distance_blocks * render_distance_blocks
        }
        BlockEntityKind::MovingPiston => centre.distance_squared(camera) < 68.0 * 68.0,
        _ => centre.distance_squared(camera) < 64.0 * 64.0,
    }
}

#[cfg(test)]
mod tests;
