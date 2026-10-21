//! The evoker fangs' bite as the generic model path draws it (`EvokerFangsModel.setupAnim`, and
//! `EvokerFangsRenderer.submit`'s lift), with vanilla's numbers.

use std::f32::consts::PI;

use crate::evoker_fangs_model::{self as fangs, FangsPose};
use haste_core::model::BodyOp;
use haste_core::parts;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

#[test]
fn jaws_start_open_and_full_height_before_the_bite() {
    // bite = 1 - min(0, 1)^3 = 1: the jaws start at their fullest opening.
    let p = fangs::pose(0.0);
    assert!(close(p.upper_jaw_z, PI - 0.35 * PI));
    assert!(close(p.lower_jaw_z, PI + 0.35 * PI));
    assert!(close(p.base_rise, 0.0));
    assert!(close(p.pre_scale, 1.0));
}

#[test]
fn jaws_open_over_the_first_half_of_the_bite() {
    // bite = 1 - (min(2 * 0.25, 1))^3 = 1 - 0.125 = 0.875
    let p = fangs::pose(0.25);
    assert!(close(p.upper_jaw_z, PI - 0.875 * 0.35 * PI));
    assert!(close(p.lower_jaw_z, PI + 0.875 * 0.35 * PI));
}

#[test]
fn jaws_rest_closed_once_the_bite_is_halfway() {
    // min(2 * 0.5, 1) = 1, so the bite amount is 0 from here on.
    for progress in [0.5, 0.75, 1.0] {
        let p = fangs::pose(progress);
        assert!(close(p.upper_jaw_z, PI), "progress {progress}");
        assert!(close(p.lower_jaw_z, PI), "progress {progress}");
    }
}

#[test]
fn base_rises_by_progress_plus_a_sine_wobble() {
    let p = fangs::pose(0.5);
    assert!(close(p.base_rise, -(0.5 + (0.5f32 * 2.7).sin()) * 7.2));
    let end = fangs::pose(1.0);
    assert!(close(end.base_rise, -(1.0 + 2.7f32.sin()) * 7.2));
}

#[test]
fn fangs_shrink_over_the_last_tenth_of_the_bite() {
    assert!(close(fangs::pose(0.9).pre_scale, 1.0));
    assert!(close(fangs::pose(0.95).pre_scale, 0.5));
    assert!(close(fangs::pose(1.0).pre_scale, 0.0));
}

#[test]
fn root_height_is_24_minus_20_times_the_scale() {
    assert!(close(fangs::root_y(1.0), 4.0));
    assert!(close(fangs::root_y(0.0), 24.0));
    assert!(close(fangs::root_y(0.5), 14.0));
}

#[test]
fn overrides_hold_the_jaw_angles_and_the_base_rise() {
    let p = fangs::pose(0.25);
    let o = fangs::overrides(p);
    let upper = o.rotation_of(parts::intern("upper_jaw")).unwrap();
    let lower = o.rotation_of(parts::intern("lower_jaw")).unwrap();
    assert!(close(upper.z, p.upper_jaw_z));
    assert!(close(lower.z, p.lower_jaw_z));
    let base = o.offset_of(parts::intern("base")).unwrap();
    assert!(close(base.y, p.base_rise));
    assert!(close(base.x, 0.0) && close(base.z, 0.0));
}

#[test]
fn body_lifts_by_one_and_a_half_minus_the_root_height() {
    // 1.501 - rootY / 16: at full scale the root is at 4 px, so the net lift is 1.251 blocks.
    let full = fangs::body(FangsPose {
        pre_scale: 1.0,
        ..fangs::pose(0.0)
    });
    let first = full.iter().next().copied();
    match first {
        Some(BodyOp::Translate(t)) => assert!(close(t.y, 1.251)),
        other => panic!("expected a translate, got {other:?}"),
    }
    let gone = fangs::body(fangs::pose(1.0));
    let first = gone.iter().next().copied();
    match first {
        Some(BodyOp::Translate(t)) => assert!(close(t.y, 0.001)),
        other => panic!("expected a translate, got {other:?}"),
    }
}

#[test]
fn instance_yaw_turns_the_fangs_by_90_minus_y_rot() {
    // The generic path turns by 180 - yaw, which must equal rotY(90 - yRot).
    for y_rot in [0.0f32, 30.0, -135.0] {
        let turn = 180.0 - fangs::instance_yaw(y_rot);
        assert!(close(turn, 90.0 - y_rot), "yRot {y_rot}");
    }
}
