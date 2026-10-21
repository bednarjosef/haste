//! The llama spit's pose as the generic model path draws it (`LlamaSpitRenderer.submit`: lift
//! 0.15, `rotY(yRot - 90)`, `rotZ(xRot)`), with vanilla's numbers.

use crate::llama_spit_model::{self as spit, LIFT};
use haste_core::model::BodyOp;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

#[test]
fn spit_lifts_by_a_sixth_of_a_block_before_its_rotations() {
    assert!(close(LIFT.x, 0.0));
    assert!(close(LIFT.y, 0.15));
    assert!(close(LIFT.z, 0.0));
    assert!(close(spit::body(0.0).pre_translate.y, 0.15));
}

#[test]
fn instance_yaw_turns_the_spit_by_y_rot_minus_90() {
    // The generic path turns by 180 - yaw, which must equal rotY(yRot - 90).
    for y_rot in [0.0f32, 30.0, -135.0] {
        let turn = 180.0 - spit::instance_yaw(y_rot);
        assert!(close(turn, y_rot - 90.0), "yRot {y_rot}");
    }
}

#[test]
fn body_pitches_the_spit_by_x_rot_and_cancels_the_flip() {
    // The generic flip is rotZ(pi); the body op's rotZ(xRot + pi) on top of it leaves rotZ(xRot).
    let b = spit::body(30.0);
    let first = b.iter().next().copied();
    match first {
        Some(BodyOp::RotateZ(r)) => {
            let turn = r - std::f32::consts::PI;
            assert!(close(turn, 30.0f32.to_radians()));
        }
        other => panic!("expected a z rotation, got {other:?}"),
    }
}
