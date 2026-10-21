//! The evoker fangs' bite as the generic model path draws it (`EvokerFangsModel.setupAnim`, and
//! `EvokerFangsRenderer.submit`'s lift), with vanilla's numbers.

use std::f32::consts::PI;

use crate::evoker_fangs_model::{self as fangs, FangsPose};
use crate::model_dump::ModelLibrary;
use glam::{Mat4, Vec3};
use haste_core::model::{BodyOp, CubeModel, ModelPart};
use haste_core::parts;

/// `EvokerFangsModel.createBodyLayer`'s rest zRot of the upper and lower jaw.
const UPPER_REST_Z: f32 = 2.042035;
const LOWER_REST_Z: f32 = 4.2411504;

/// The fangs' layer as the recorder dumps it (`EvokerFangsModel.createBodyLayer`).
const FANGS_DUMP: &str = r#"{"version": 1, "layers": [{"layer": "minecraft:evoker_fangs#main",
  "texture_width": 64, "texture_height": 32, "parts": [{"name": "base",
  "pivot": [-5.0, 24.0, -5.0], "rotation": [0.0, 0.0, 0.0], "scale": [1.0, 1.0, 1.0],
  "cubes": [{"origin": [0.0, 0.0, 0.0], "size": [10.0, 12.0, 10.0], "u": 0.0, "v": 0.0,
    "mirror": false, "inflate": [0.0, 0.0, 0.0]}],
  "children": [
    {"name": "lower_jaw", "pivot": [3.5, 0.0, 9.0], "rotation": [0.0, 3.1415927, 4.2411504],
     "scale": [1.0, 1.0, 1.0], "cubes": [{"origin": [0.0, 0.0, 0.0], "size": [4.0, 14.0, 8.0],
     "u": 40.0, "v": 0.0, "mirror": false, "inflate": [0.0, 0.0, 0.0]}], "children": []},
    {"name": "upper_jaw", "pivot": [6.5, 0.0, 1.0], "rotation": [0.0, 0.0, 2.042035],
     "scale": [1.0, 1.0, 1.0], "cubes": [{"origin": [0.0, 0.0, 0.0], "size": [4.0, 14.0, 8.0],
     "u": 40.0, "v": 0.0, "mirror": false, "inflate": [0.0, 0.0, 0.0]}], "children": []}
  ]}]}]}"#;

fn fangs_model() -> std::sync::Arc<CubeModel> {
    let lib = ModelLibrary::parse(FANGS_DUMP.as_bytes()).unwrap();
    lib.get("minecraft:evoker_fangs#main", 1.0).unwrap()
}

fn base_of(model: &CubeModel) -> &ModelPart {
    model
        .root
        .children
        .iter()
        .find(|p| p.name == "base")
        .unwrap()
}

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
fn overrides_hold_the_jaw_angles_relative_to_the_rest_pose_and_the_base_rise() {
    // Overrides add to the rest pose, vanilla assigns zRot outright: the layer's rest zRot
    // (2.042035 and 4.2411504 in `EvokerFangsModel.createBodyLayer`) is taken back out.
    let model = fangs_model();
    let p = fangs::pose(0.25);
    let o = fangs::overrides(p, &model);
    let upper = o.rotation_of(parts::intern("upper_jaw")).unwrap();
    let lower = o.rotation_of(parts::intern("lower_jaw")).unwrap();
    assert!(close(upper.z + UPPER_REST_Z, p.upper_jaw_z));
    assert!(close(lower.z + LOWER_REST_Z, p.lower_jaw_z));
    assert!(close(upper.x, 0.0) && close(upper.y, 0.0));
    assert!(
        close(lower.x, 0.0) && close(lower.y, 0.0),
        "lower jaw keeps its yRot = pi"
    );
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

#[test]
fn the_fangs_layer_keeps_vanillas_pivots() {
    // `EvokerFangsRenderer.submit` places the model itself (its -1.501 translate is in
    // `body`), so the loader must not also shift the layer down by the ground plane.
    let model = fangs_model();
    assert!(close(base_of(&model).pivot.y, 24.0));
}

/// The generic entity path's transform (haste-render `EntityRenderer.prepare`): the yaw
/// `180 - yaw`, the body ops, then the flip and the body scale over 16, then the part.
fn generic_path(yaw: f32, pose: FangsPose, model: &CubeModel) -> Mat4 {
    let mut m = Mat4::from_rotation_y((180.0 - yaw).to_radians());
    for op in fangs::body(pose).iter() {
        m *= match op {
            BodyOp::Translate(t) => Mat4::from_translation(*t),
            BodyOp::RotateX(a) => Mat4::from_rotation_x(*a),
            BodyOp::RotateY(a) => Mat4::from_rotation_y(*a),
            BodyOp::RotateZ(a) => Mat4::from_rotation_z(*a),
        };
    }
    let s = pose.pre_scale;
    let base = base_of(model);
    m * Mat4::from_scale(Vec3::new(-s, -s, s) / 16.0)
        * Mat4::from_translation(base.pivot + Vec3::new(0.0, pose.base_rise, 0.0))
}

/// `EvokerFangsRenderer.submit` then `EvokerFangsModel.setupAnim`'s root and base, in
/// blocks: `rotY(90 - yRot)`, `scale(-1, -1, 1)`, `translate(0, -1.501, 0)`, the root's
/// translation and scale, the base's pivot.
fn vanilla_path(y_rot: f32, pose: FangsPose) -> Mat4 {
    Mat4::from_rotation_y((90.0 - y_rot).to_radians())
        * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0))
        * Mat4::from_translation(Vec3::new(0.0, -1.501, 0.0))
        * Mat4::from_translation(Vec3::new(0.0, fangs::root_y(pose.pre_scale) / 16.0, 0.0))
        * Mat4::from_scale(Vec3::splat(pose.pre_scale))
        * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
        * Mat4::from_translation(Vec3::new(-5.0, 24.0 + pose.base_rise, -5.0))
}

#[test]
fn the_base_lands_where_vanilla_draws_it() {
    let model = fangs_model();
    for (progress, y_rot) in [(0.05f32, 0.0f32), (0.5, 37.0), (0.95, -120.0)] {
        let pose = fangs::pose(progress);
        let ours = generic_path(fangs::instance_yaw(y_rot), pose, &model);
        let theirs = vanilla_path(y_rot, pose);
        // The base cube's corners (pixels, relative to the base pivot).
        for corner in [
            Vec3::ZERO,
            Vec3::new(10.0, 12.0, 10.0),
            Vec3::new(10.0, 0.0, 0.0),
        ] {
            let a = ours.transform_point3(corner);
            let b = theirs.transform_point3(corner);
            assert!((a - b).length() < 1e-4, "progress {progress}: {a} vs {b}");
        }
    }
    // Mid-bite the base's top (its pivot) is above the ground, under one block.
    let top = vanilla_path(0.0, fangs::pose(0.5)).transform_point3(Vec3::ZERO);
    assert!(top.y > 0.0 && top.y < 1.0, "{top}");
}
