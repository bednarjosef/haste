use super::{BorderStatus, WorldBorder};
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

/// `set_border_lerp_size` interpolates the diameter over the given ticks.
#[test]
fn the_size_lerps_over_time() {
    let mut b = WorldBorder::default();
    // 0 ticks: the size is already the target.
    b.set_lerp_size(10.0, 20.0, 0);
    assert!(b.is_within_bounds(BlockPos::new(9, 64, 0)));
    // Shrinking back to 10 over a long time (about a month of ticks): the bounds stay near 20.
    b.set_lerp_size(20.0, 10.0, 60_000_000);
    assert!(b.is_within_bounds(BlockPos::new(9, 64, 0)));
    assert!(!b.is_within_bounds(BlockPos::new(10, 64, 0)));
}

/// Vanilla `WorldBorder.Settings.DEFAULT`: the warning comes 15 ticks ahead, 5 blocks out.
#[test]
fn the_default_warning_settings_are_vanillas() {
    let b = WorldBorder::default();
    assert_eq!(b.warning_distance(), 5.0);
    assert_eq!(b.status(), BorderStatus::Stationary);
}

/// `getDistanceToBorder` is the nearest edge's distance, negative outside.
#[test]
fn distance_is_to_the_nearest_edge() {
    let mut b = WorldBorder::default();
    b.initialize(0.0, 0.0, 20.0, 20.0, 0);
    assert_eq!(b.distance_to_border(0.0, 0.0), 10.0);
    assert_eq!(b.distance_to_border(7.0, 0.0), 3.0);
    assert_eq!(b.distance_to_border(0.0, -9.0), 1.0);
    assert_eq!(b.distance_to_border(11.0, 0.0), -1.0);
}

/// The vignette: the warning strength rises from 0 at the warning distance to 1 at the edge.
#[test]
fn the_warning_strength_rises_towards_the_edge() {
    let mut b = WorldBorder::default();
    b.initialize(0.0, 0.0, 20.0, 20.0, 0);
    // The warning distance is 5 blocks (the default warning) and the border is still.
    assert_eq!(b.warning_strength(0.0, 0.0), 0.0);
    assert_eq!(b.warning_strength(5.0, 0.0), 0.0);
    // The east edge is at x = 10: x = 8 is 2 blocks from it, 0.6 of the way into the warning.
    let s = b.warning_strength(8.0, 0.0);
    assert!((s - 0.6).abs() < 1e-6, "{s}");
    assert_eq!(b.warning_strength(10.0, 0.0), 1.0);
}

/// `WorldBorder.warning_distance` grows with a moving border: the lerp speed times the
/// warning time, capped by what is left to move (vanilla `Hud.extractVignette`).
#[test]
fn a_moving_border_warns_further_out() {
    let mut b = WorldBorder::default();
    // Shrinking 100 blocks over 1000 ticks: 0.1 blocks a tick, 15 ticks ahead is 1.5 blocks,
    // less than the 5-block warning.
    b.set_lerp_size(200.0, 100.0, 1000);
    assert_eq!(b.status(), BorderStatus::Shrinking);
    assert!((b.lerp_speed() - 0.1).abs() < 1e-9);
    assert_eq!(b.warning_distance(), 5.0);
    // Shrinking 1000 blocks over 1000 ticks: 1 block a tick, 15 ticks ahead is 15 blocks.
    b.set_lerp_size(2000.0, 1000.0, 1000);
    assert!((b.warning_distance() - 15.0).abs() < 1e-9);
    // Growing: the status and the colour follow the direction.
    b.set_lerp_size(100.0, 200.0, 1000);
    assert_eq!(b.status(), BorderStatus::Growing);
    assert_eq!(BorderStatus::Growing.color(), 4_259_712);
    assert_eq!(BorderStatus::Shrinking.color(), 16_724_016);
    assert_eq!(BorderStatus::Stationary.color(), 2_138_367);
}

/// The warning settings packets set the warning distance and time.
#[test]
fn the_warning_packets_set_the_settings() {
    let mut b = WorldBorder::default();
    b.set_warning_blocks(12);
    assert_eq!(b.warning_distance(), 12.0);
    b.set_warning_time(30);
    assert_eq!(b.warning_distance(), 12.0);
}
