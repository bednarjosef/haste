//! Tridents: the pose `ThrownTridentRenderer.submit` gives the model, checked by the tip's
//! position after the transform the renderer builds (the body ops, then the generic path's
//! `scale(-1, -1, 1)` flip, which the body transform must cancel).

use glam::{Mat4, Vec3};
use haste_core::model::{BodyOp, BodyTransform};

use crate::trident::{body_transform, is_trident};

/// The model's tip: the spike at the top of the pole, `ModelPart` space (y down, sixteenths).
const TIP: Vec3 = Vec3::new(0.0, -4.0, 0.0);

/// The whole-model matrix the generic path builds for a body transform at scale 1: the ops in
/// order (the first outermost), then the flip.
fn full_matrix(b: &BodyTransform) -> Mat4 {
    let mut m = Mat4::IDENTITY;
    for op in b.iter() {
        m *= match op {
            BodyOp::RotateX(a) => Mat4::from_rotation_x(*a),
            BodyOp::RotateY(a) => Mat4::from_rotation_y(*a),
            BodyOp::RotateZ(a) => Mat4::from_rotation_z(*a),
            BodyOp::Translate(t) => Mat4::from_translation(*t),
        };
    }
    m * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0))
}

/// The tip's position for a yaw and pitch in degrees, as the renderer draws it.
fn tip(yaw: f32, pitch: f32) -> Vec3 {
    full_matrix(&body_transform(yaw, pitch)).transform_point3(TIP)
}

fn assert_near(actual: Vec3, expected: Vec3) {
    assert!(
        actual.abs_diff_eq(expected, 1e-4),
        "tip {actual:?} != {expected:?}"
    );
}

#[test]
fn body_transform_is_the_vanilla_rotations_without_the_yaw() {
    let b = body_transform(30.0, -15.0);
    assert!(b.suppress_yaw);
    assert_eq!(b.len, 2);
    let ops: Vec<BodyOp> = b.iter().copied().collect();
    // `rotateDegrees(YP, yRot - 90)` then `rotateDegrees(ZP, xRot + 90)`, with the half turn
    // that undoes the flip on the Z rotation.
    let BodyOp::RotateY(y) = ops[0] else {
        panic!("first op must be RotateY, got {:?}", ops[0]);
    };
    let BodyOp::RotateZ(z) = ops[1] else {
        panic!("second op must be RotateZ, got {:?}", ops[1]);
    };
    assert!((y - (-60.0f32).to_radians()).abs() < 1e-6);
    assert!((z - (75.0f32.to_radians() + std::f32::consts::PI)).abs() < 1e-6);
}

#[test]
fn tip_points_along_the_flight_at_level_pitch() {
    // Yaw 0: the tip points to +Z; yaw 90: to +X; yaw 180: to -Z.
    assert_near(tip(0.0, 0.0), Vec3::new(0.0, 0.0, 4.0));
    assert_near(tip(90.0, 0.0), Vec3::new(4.0, 0.0, 0.0));
    assert_near(tip(180.0, 0.0), Vec3::new(0.0, 0.0, -4.0));
}

#[test]
fn tip_follows_the_pitch() {
    // Pitch -90 keeps the tip up (-Y); pitch 90 turns it down (+Y).
    assert_near(tip(0.0, -90.0), Vec3::new(0.0, -4.0, 0.0));
    assert_near(tip(0.0, 90.0), Vec3::new(0.0, 4.0, 0.0));
}

#[test]
fn tip_at_mixed_yaw_and_pitch() {
    // Pitch 30 tips the up tip about Z to (4 sin 120, -4 cos 120, 0) = (3.464, 2, 0); yaw 45 then
    // turns it about Y by -45 degrees, which gives x = z = 3.464 / sqrt 2.
    let expected = Vec3::new(2.449_49, 2.0, 2.449_49);
    assert_near(tip(45.0, 30.0), expected);
}

#[test]
fn the_flip_is_cancelled_so_the_tip_keeps_its_length() {
    for (yaw, pitch) in [(0.0, 0.0), (33.0, -70.0), (-120.0, 45.0), (270.0, 89.0)] {
        assert!((tip(yaw, pitch).length() - 4.0).abs() < 1e-4);
    }
}

#[test]
fn trident_kind_resolves_only_for_the_trident() {
    let Some(names) = crate::registry::entity_type_names() else {
        return;
    };
    for (k, v) in names.iter() {
        assert_eq!(is_trident(*k), *v == "minecraft:trident");
    }
}
