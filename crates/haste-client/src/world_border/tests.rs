use super::WorldBorder;
use haste_core::BlockPos;

/// The default border (vanilla `WorldBorder.Settings.DEFAULT`) crosses nothing.
#[test]
fn the_default_border_is_effectively_unlimited() {
    let b = WorldBorder::default();
    assert!(b.is_within_bounds(BlockPos::new(0, 64, 0)));
    assert!(b.is_within_bounds(BlockPos::new(-29_000_000, 64, 29_000_000)));
}

/// `initialize_border` / `set_border_*` move the square, and the size lerps over time.
#[test]
fn centre_and_size_move_the_bounds() {
    let mut b = WorldBorder::default();
    b.initialize(10.0, -20.0, 8.0, 8.0, 0);
    // A diameter-8 border around (10, -20): x in [6, 14), z in [-24, -16).
    assert!(b.is_within_bounds(BlockPos::new(6, 64, -24)));
    assert!(!b.is_within_bounds(BlockPos::new(14, 64, -16)));
    assert!(!b.is_within_bounds(BlockPos::new(5, 64, -20)));
    // The centre moves.
    b.set_center(0.0, 0.0);
    assert!(b.is_within_bounds(BlockPos::new(-4, 64, 3)));
    assert!(!b.is_within_bounds(BlockPos::new(4, 64, 0)));
    // The size jumps with set_border_size.
    b.set_size(4.0);
    assert!(b.is_within_bounds(BlockPos::new(-2, 64, 1)));
    assert!(!b.is_within_bounds(BlockPos::new(2, 64, 0)));
}

/// `set_border_lerp_size` interpolates the diameter over the given milliseconds.
#[test]
fn the_size_lerps_over_time() {
    let mut b = WorldBorder::default();
    // 0 ms: the size is already the target.
    b.set_lerp_size(10.0, 20.0, 0);
    assert!(b.is_within_bounds(BlockPos::new(9, 64, 0)));
    // Shrinking back to 10 over a long time: the bounds grow with it.
    b.set_lerp_size(20.0, 10.0, 60_000_000); // ~69 days
                                             // A tenth of the way: diameter ~19, half ~9.5.
    assert!(b.is_within_bounds(BlockPos::new(9, 64, 0)));
    assert!(!b.is_within_bounds(BlockPos::new(10, 64, 0)));
}
