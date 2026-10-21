//! The fishing bobber's pure math (`FishingHookRenderer`, `FishingRodCast`) against vanilla's
//! numbers: the line's sag and 16 segments, the hand offsets, the arm choice, the cast model
//! and the line width.
use haste_core::{DVec3, Vec3};

use crate::fishing_hook::{
    appropriate_line_width, cast_model_selected, fraction, hand_pos_first_person,
    hand_pos_third_person, holding_arm_is_right, line_origin_offset, string_point,
    string_segment_vertices, string_vertex, swing_factor, NearPlane, STRING_SEGMENTS,
};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-4
}

fn close_v(a: DVec3, b: DVec3) -> bool {
    close(a.x, b.x) && close(a.y, b.y) && close(a.z, b.z)
}

#[test]
fn line_sags_from_hook_to_hand() {
    // The end at the bobber sits 0.25 above the hook's origin; the hand end lands at the line.
    let line = DVec3::new(2.0, 4.0, 0.0);
    assert_eq!(string_point(line, 0.0), Vec3::new(0.0, 0.25, 0.0));
    assert_eq!(string_point(line, 1.0), Vec3::new(2.0, 4.25, 0.0));
    // Halfway: y = 4 * (0.25 + 0.5) / 2 + 0.25 = 1.75 (the parabola, not a straight line).
    assert_eq!(string_point(line, 0.5), Vec3::new(1.0, 1.75, 0.0));
}

#[test]
fn sixteen_segments_two_vertices_each() {
    assert_eq!(STRING_SEGMENTS, 16);
    assert_eq!(fraction(0, 16), 0.0);
    assert_eq!(fraction(16, 16), 1.0);
    assert_eq!(fraction(4, 16), 0.25);
    let line = DVec3::new(1.0, 2.0, 3.0);
    for i in 0..STRING_SEGMENTS {
        let [first, second] = string_segment_vertices(line, i);
        // The two endpoints of one segment are the same two points in swapped order.
        assert_eq!(first.0, string_point(line, fraction(i, 16)));
        assert_eq!(second.0, string_point(line, fraction(i + 1, 16)));
        // The normals point from one endpoint to the other, in opposite directions.
        assert!((first.1 + second.1).length() < 1e-5);
        assert!((first.1.length() - 1.0).abs() < 1e-5);
    }
}

#[test]
fn degenerate_segment_has_zero_normal_not_nan() {
    let (p, n) = string_vertex(DVec3::ZERO, 0.5, 0.5);
    assert_eq!(p, string_point(DVec3::ZERO, 0.5));
    assert_eq!(n, Vec3::ZERO);
}

#[test]
fn holding_arm_follows_the_rod() {
    // A rod in the main hand: the main arm holds it.
    assert!(holding_arm_is_right(true, true));
    assert!(!holding_arm_is_right(false, true));
    // Anything else in the main hand: the off arm holds the rod.
    assert!(!holding_arm_is_right(true, false));
    assert!(holding_arm_is_right(false, false));
}

#[test]
fn cast_model_needs_a_hook_and_the_rod_in_that_arm() {
    assert!(cast_model_selected(true, true));
    assert!(!cast_model_selected(false, true));
    assert!(!cast_model_selected(true, false));
    assert!(!cast_model_selected(false, false));
}

#[test]
fn line_width_is_at_least_two_and_a_half_pixels() {
    assert_eq!(appropriate_line_width(800), 2.5);
    assert_eq!(appropriate_line_width(1920), 2.5);
    assert_eq!(appropriate_line_width(3840), 5.0);
}

#[test]
fn swing_lifts_the_hand_on_a_sine_of_its_root() {
    assert!(swing_factor(0.0).abs() < 1e-6);
    // sqrt(0.25) * pi = pi / 2: the peak.
    assert!((swing_factor(0.25) - 1.0).abs() < 1e-6);
    // sqrt(1) * pi = pi: back to zero.
    assert!(swing_factor(1.0).abs() < 1e-6);
}

#[test]
fn third_person_hand_at_zero_yaw() {
    let eye = DVec3::new(10.0, 64.0, 10.0);
    // Yaw 0: the right hand is 0.35 to the right (-x), 0.8 forward (+z), 0.45 below the eye.
    let right = hand_pos_third_person(eye, 0.0, 1.0, true, false);
    assert!(close_v(right - eye, DVec3::new(-0.35, -0.45, 0.8)));
    // The left hand mirrors across the body.
    let left = hand_pos_third_person(eye, 0.0, 1.0, false, false);
    assert!(close_v(left - eye, DVec3::new(0.35, -0.45, 0.8)));
    // Crouching drops the hand by a further 0.1875.
    let crouch = hand_pos_third_person(eye, 0.0, 1.0, true, true);
    assert!(close_v(crouch - eye, DVec3::new(-0.35, -0.6375, 0.8)));
}

#[test]
fn third_person_hand_follows_body_yaw_and_scale() {
    let eye = DVec3::ZERO;
    // Yaw 90 degrees: forward becomes -x for the body's sin/cos; the right offset follows.
    let hand = hand_pos_third_person(eye, 90.0, 1.0, true, false);
    assert!(close_v(hand, DVec3::new(-0.8, -0.45, -0.35)));
    // A 0.5 scale halves every offset.
    let small = hand_pos_third_person(eye, 0.0, 0.5, true, false);
    assert!(close_v(small, DVec3::new(-0.175, -0.225, 0.4)));
}

#[test]
fn near_plane_point_and_first_person_hand() {
    // A 90 degree fov at z-near 0.05: the plane is 0.05 tall and 0.05 * aspect wide.
    let near = NearPlane::new(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 1.0, 0.0),
        DVec3::new(1.0, 0.0, 0.0),
        90.0,
        1.0,
        0.05,
    );
    assert!(close_v(
        near.point_on_plane(0.0, 0.0),
        DVec3::new(0.0, 0.0, 0.05)
    ));
    // Right hand (invert +1): x = 0.525 moves the point to the right (-left), y = -0.1 down.
    let point = near.point_on_plane(0.525, -0.1);
    assert!(close(point.x, -0.05 * 0.525));
    assert!(close(point.y, -0.05 * 0.1));
    // With no swing the hand is the plane point scaled by 960 / fov, from the eye.
    let eye = DVec3::new(1.0, 2.0, 3.0);
    let hand = hand_pos_first_person(eye, &near, 90.0, true, 0.0);
    let expected = eye + point * (960.0 / 90.0);
    assert!(close_v(hand, expected));
}

#[test]
fn line_starts_at_hand_and_is_empty_without_owner() {
    let hook = DVec3::new(0.0, 0.0, 0.0);
    let offset = line_origin_offset(Some(DVec3::new(1.0, 2.0, 3.0)), hook);
    assert!(close_v(offset, DVec3::new(1.0, 1.75, 3.0)));
    assert_eq!(line_origin_offset(None, hook), DVec3::ZERO);
}
