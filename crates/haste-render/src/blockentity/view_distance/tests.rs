use super::*;
use haste_core::BlockPos;

fn camera(x: f64, y: f64, z: f64) -> glam::DVec3 {
    glam::DVec3::new(x, y, z)
}

/// Vanilla `BlockEntityRenderer.shouldRender`: 64 blocks from the block's centre, and the
/// boundary itself is out (`closerThan` is strict).
#[test]
fn default_sixty_four_blocks_from_the_centre() {
    let kind = BlockEntityKind::Bell;
    // The block at the origin: its centre is (0.5, 0.5, 0.5). 63.9 blocks away is in...
    assert!(within_view_distance(
        &kind,
        BlockPos::new(0, 0, 0),
        camera(63.9 + 0.5, 0.5, 0.5),
        512.0
    ));
    // ...exactly 64 is out, ...
    assert!(!within_view_distance(
        &kind,
        BlockPos::new(0, 0, 0),
        camera(64.5, 0.5, 0.5),
        512.0
    ));
    // ...and the distance is 3D: a block 63 blocks below and 10 across is over 64 out.
    assert!(!within_view_distance(
        &kind,
        BlockPos::new(0, 0, 0),
        camera(10.5, 63.5 + 0.5, 0.5),
        512.0
    ));
}

/// `PistonHeadRenderer.getViewDistance` is 68, not the default 64.
#[test]
fn piston_sixty_eight_blocks() {
    let kind = BlockEntityKind::MovingPiston;
    assert!(within_view_distance(
        &kind,
        BlockPos::new(0, 0, 0),
        camera(67.5, 0.5, 0.5),
        512.0
    ));
    assert!(!within_view_distance(
        &kind,
        BlockPos::new(0, 0, 0),
        camera(68.5, 0.5, 0.5),
        512.0
    ));
}

/// `BeaconRenderer.shouldRender` measures horizontally (both y zeroed) out to the effective
/// render distance in blocks, so a beacon far below or above still draws.
#[test]
fn beacon_horizontal_to_the_render_distance() {
    let kind = BlockEntityKind::Beacon;
    // 300 blocks below and 100 across, render distance 512 blocks: in.
    assert!(within_view_distance(
        &kind,
        BlockPos::new(0, 0, 0),
        camera(100.5, 300.5, 0.5),
        512.0
    ));
    // The same beacon with the camera 600 blocks out horizontally: out.
    assert!(!within_view_distance(
        &kind,
        BlockPos::new(0, 0, 0),
        camera(600.5, 0.5, 0.5),
        512.0
    ));
}
