//! The llama spit's pose as the generic model path draws it (`LlamaSpitRenderer.submit`: lift
//! 0.15, `rotY(yRot - 90)`, `rotZ(xRot)`), with vanilla's numbers.

use crate::llama_spit_model::{self as spit, LIFT};
use crate::model_dump::ModelLibrary;
use haste_core::model::BodyOp;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

#[test]
fn spit_lifts_by_0_15_blocks_before_its_rotations() {
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

#[test]
fn the_spit_layer_keeps_its_pivot_at_the_origin() {
    // `LlamaSpitRenderer.submit` draws the layer about the entity's origin (no ground-plane
    // offset), so the loader must not shift it down by 24 pixels.
    let dump = r#"{"version": 1, "layers": [{"layer": "minecraft:llama_spit#main",
      "texture_width": 64, "texture_height": 32, "parts": [{"name": "main",
      "pivot": [0.0, 0.0, 0.0], "rotation": [0.0, 0.0, 0.0], "scale": [1.0, 1.0, 1.0],
      "cubes": [{"origin": [0.0, 0.0, 0.0], "size": [2.0, 2.0, 2.0], "u": 0.0, "v": 0.0,
        "mirror": false, "inflate": [0.0, 0.0, 0.0]}], "children": []}]}]}"#;
    let lib = ModelLibrary::parse(dump.as_bytes()).unwrap();
    let model = lib.get("minecraft:llama_spit#main", 1.0).unwrap();
    assert!(close(model.root.children[0].pivot.y, 0.0));
}
