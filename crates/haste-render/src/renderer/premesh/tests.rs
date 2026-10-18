//! Tests for the pre-mesh turn prediction: the smoothed velocities, the widened cone
//! and the online predictor. No GPU needed — the state machine is driven by hand.

use std::time::{Duration, Instant};

use super::*;

fn camera(yaw: f32) -> Camera {
    Camera {
        pos: glam::DVec3::ZERO,
        yaw,
        pitch: 0.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: ViewBob::default(),
    }
}

/// Feeds `frames` 16 ms samples turning at `deg_per_second` (and moving at
/// `blocks_per_second` along +X), returning the last camera. The aspect is 1 (a
/// square view: the horizontal fov is the vertical one, 70 degrees).
fn drive(
    premesh: &mut Premesh,
    deg_per_second: f32,
    blocks_per_second: f64,
    frames: usize,
) -> Camera {
    let mut t = Instant::now();
    premesh.update(&camera(0.0), 1.0, t);
    let mut last = camera(0.0);
    for i in 1..=frames {
        t += Duration::from_millis(16);
        let mut cam = camera(deg_per_second * (i as f32 * 0.016));
        cam.pos = glam::DVec3::new(blocks_per_second * i as f64 * 0.016, 0.0, 0.0);
        premesh.update(&cam, 1.0, t);
        last = cam;
    }
    last
}

#[test]
fn yaw_differences_wrap_into_minus_180_to_180() {
    assert_eq!(wrap_deg(10.0), 10.0);
    assert_eq!(wrap_deg(190.0), -170.0);
    assert_eq!(wrap_deg(-190.0), 170.0);
    assert_eq!(wrap_deg(360.0), 0.0);
    assert_eq!(wrap_deg(-720.0), 0.0);
}

#[test]
fn a_still_camera_pre_meshes_nothing_even_in_cone_mode() {
    let mut premesh = Premesh::new();
    premesh.mode = PremeshMode::Cone;
    drive(&mut premesh, 0.0, 0.0, 30);
    assert!(premesh.frustum.is_none(), "no turn, no widened cone");
}

#[test]
fn off_mode_never_builds_a_cone() {
    let mut premesh = Premesh::new();
    drive(&mut premesh, 180.0, 0.0, 30);
    assert!(premesh.frustum.is_none());
    assert_eq!(premesh.mode, PremeshMode::Off);
}

#[test]
fn cone_widens_toward_the_turn_and_covers_the_side() {
    let mut premesh = Premesh::new();
    premesh.mode = PremeshMode::Cone;
    // A 40 deg/s turn for half a second: the smoothed velocity settles there and the
    // cone opens about 20 degrees past the camera's own look.
    let last = drive(&mut premesh, 40.0, 0.0, 31);
    let (planes, origin) = premesh.frustum.expect("a turning camera widens the cone");
    assert_eq!(origin, glam::DVec3::ZERO, "no movement, no shifted origin");
    // The camera has turned to ~20 degrees (yaw + is toward -X); the cone looks ~20
    // degrees further. A section cube at bearing ~75, 40 blocks out, is inside the
    // widened cone (which spans about 5..75 degrees) but outside the plain view
    // (about -15..55 degrees with the square 70-degree fov).
    let lo = [-46.6, -8.0, 2.4];
    let hi = [-30.6, 8.0, 18.4];
    assert!(
        Renderer::aabb_visible(&planes, lo, hi),
        "the widened cone covers the side the camera turns toward"
    );
    let plain = Renderer::frustum_planes(&Renderer::view_projection(&last, 1.0, None));
    assert!(
        !Renderer::aabb_visible(&plain, lo, hi),
        "the plain view does not reach the side section yet"
    );
    // The widening is capped: a section behind the camera (-Z) stays out of the cone.
    let lo = [-8.0, -8.0, -24.0];
    let hi = [8.0, 8.0, -8.0];
    assert!(
        !Renderer::aabb_visible(&planes, lo, hi),
        "the cone does not reach behind the camera"
    );
}

#[test]
fn fast_movement_shifts_the_cone_origin_forward() {
    let mut premesh = Premesh::new();
    premesh.mode = PremeshMode::Cone;
    // 20 blocks/s (flight): the origin sits HORIZON seconds ahead of the camera.
    let last = drive(&mut premesh, 0.0, 20.0, 31);
    let (_, origin) = premesh
        .frustum
        .expect("fast movement widens the cone forward");
    let want = last.pos + glam::DVec3::new(20.0 * HORIZON as f64, 0.0, 0.0);
    assert!(
        (origin - want).length() < 2.0,
        "origin {origin:?} should be about {want:?}"
    );
}

#[test]
fn predict_mode_learns_a_constant_turn_it_underestimates() {
    let mut premesh = Premesh::new();
    premesh.mode = PremeshMode::Predict;
    // A wrong gain (predicting a quarter of the realized 60-degree deltas of a
    // 120 deg/s turn): training pulls the velocity gain up toward cone's rule (50).
    premesh.weights = [0.0, 20.0, 0.0];
    drive(&mut premesh, 120.0, 0.0, 300);
    // The model's prediction for the turn (bias + gain * the 1.2 feature of 120 deg/s)
    // converges toward the realized 60-degree delta.
    let predicted = premesh.weights[0] + premesh.weights[1] * 1.2;
    assert!(
        predicted > 50.0,
        "the prediction {predicted} (weights {:?}) should converge upward",
        premesh.weights
    );
    assert!(premesh.weights[1] <= WEIGHT_VEL_CLAMP, "the clamp holds");
    assert!(premesh.frustum.is_some(), "a turning camera still widens");
}

#[test]
fn predict_mode_survives_a_wild_ramp() {
    let mut premesh = Premesh::new();
    premesh.mode = PremeshMode::Predict;
    let mut t = Instant::now();
    premesh.update(&camera(0.0), 1.0, t);
    let mut yaw = 0.0;
    let mut vel = 0.0f32;
    for _ in 1..=250 {
        t += Duration::from_millis(16);
        vel += 6.0; // +6 deg/s per frame: 0 to 1500 deg/s over 4 s
        yaw += vel * 0.016;
        premesh.update(&camera(yaw), 1.0, t);
    }
    // However far the ramp drove the weights, they stay inside their clamps and the
    // cone still opens (the widening itself is clamped to 90 degrees).
    assert!(premesh.weights[0].abs() <= WEIGHT_BIAS_CLAMP);
    assert!((0.0..=WEIGHT_VEL_CLAMP).contains(&premesh.weights[1]));
    assert!(premesh.weights[2].abs() <= WEIGHT_ACCEL_CLAMP);
    assert!(premesh.frustum.is_some());
}

#[test]
fn premesh_verdicts_count_hits_and_waste() {
    let mut premesh = Premesh::new();
    let near = SectionPos::new(0, 0, 0);
    let far = SectionPos::new(100, 0, 0);
    premesh.note_premeshed(near);
    premesh.note_premeshed(far);
    assert_eq!(premesh.summary(), (2, 0, 0));
    // The near one becomes visible next frame: a hit. The far one never does; aging it
    // past the window counts it as waste. The survivors keep waiting.
    let mut pending = std::mem::take(&mut premesh.premeshed);
    let mut keep = std::mem::take(&mut premesh.keep);
    let (hits, wasted) = Premesh::age_premeshed(1, &|p| p == near, &mut pending, &mut keep);
    assert_eq!((hits, wasted), (1, 0));
    assert_eq!(keep, vec![(far, 0)], "the far one keeps waiting");
    premesh.hits += hits;
    premesh.wasted += wasted;
    premesh.premeshed = keep;
    premesh.keep = pending;
    assert_eq!(premesh.summary(), (2, 1, 0));
    let mut pending = std::mem::take(&mut premesh.premeshed);
    let mut keep = std::mem::take(&mut premesh.keep);
    let (hits, wasted) =
        Premesh::age_premeshed(1 + HIT_FRAMES, &|_| false, &mut pending, &mut keep);
    assert_eq!((hits, wasted), (0, 1));
    premesh.hits += hits;
    premesh.wasted += wasted;
    premesh.premeshed = keep;
    premesh.keep = pending;
    assert_eq!(premesh.summary(), (2, 1, 1));
    assert!(premesh.premeshed.is_empty());
}
