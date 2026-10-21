use glam::Vec3;

use super::item_activation_pose;

/// The pose's origin, which is where the item's translation lands.
fn origin(pose: glam::Mat4) -> Vec3 {
    pose.transform_point3(Vec3::ZERO)
}

#[test]
fn first_tick_starts_at_the_near_plane_offset() {
    // At 40 ticks left nothing has moved yet: scale 0, so the sines are 0 and z is -10.
    let p = origin(item_activation_pose(40, 0.0, 1.0, -1.0, 1.0));
    assert!(p.abs_diff_eq(Vec3::new(0.0, 0.0, -10.0), 1e-5), "{p}");
}

#[test]
fn midpoint_pushes_the_item_to_the_camera() {
    // 20 ticks in with no partial tick: the curve's smooth scale is about 0.498, so the z
    // sine is about 1 and the item sits 9 units further toward the camera (z about -1).
    let p = origin(item_activation_pose(20, 0.0, 0.0, 0.0, 1.0));
    assert!((p.z - -1.0).abs() < 1e-3, "{p}");
    assert!(p.x.abs() < 1e-5 && p.y.abs() < 1e-5, "{p}");
}

#[test]
fn partial_tick_moves_the_curve_between_ticks() {
    // Half a tick past 30 ticks in lies between the poses at 30 and 29 ticks left.
    let at_30 = origin(item_activation_pose(30, 0.0, 0.0, 0.0, 1.0)).z;
    let at_29 = origin(item_activation_pose(29, 0.0, 0.0, 0.0, 1.0)).z;
    let half = origin(item_activation_pose(30, 0.5, 0.0, 0.0, 1.0)).z;
    assert!(half > at_30.min(at_29) && half < at_30.max(at_29), "{half}");
}

#[test]
fn the_horizontal_drift_scales_with_the_aspect_ratio() {
    // At 30 ticks left the wobble is non-zero (0.3 * |sin| is about 0.095), so the drift
    // doubles with a 2:1 window.
    let square = origin(item_activation_pose(30, 0.0, 1.0, 0.0, 1.0)).x;
    let wide = origin(item_activation_pose(30, 0.0, 1.0, 0.0, 2.0)).x;
    assert!(square.abs() > 0.05, "{square}");
    assert!((wide - 2.0 * square).abs() < 1e-4, "{wide} vs {square}");
}

#[test]
fn the_vertical_drift_ignores_the_aspect_ratio() {
    let square = origin(item_activation_pose(30, 0.0, 0.0, 1.0, 1.0)).y;
    let wide = origin(item_activation_pose(30, 0.0, 0.0, 1.0, 2.0)).y;
    assert!(square.abs() > 0.05, "{square}");
    assert!((wide - square).abs() < 1e-5, "{wide} vs {square}");
}

#[test]
fn the_pose_is_finished_and_still_in_front_at_the_last_tick() {
    // One tick left: scale near 1, the item settles back to the camera's near offset.
    let p = origin(item_activation_pose(1, 1.0, 0.0, 0.0, 1.0));
    assert!(p.z.is_finite() && p.z < 0.0, "{p}");
}
