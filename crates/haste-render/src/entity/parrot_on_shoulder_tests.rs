use glam::Vec3;
use haste_core::model::{CubeModel, ModelPart, PoseOverrides};
use haste_core::parts::intern;
use haste_core::Identifier;

use super::parrot_on_shoulder::*;

fn part(name: &str, rotation: Vec3, children: Vec<ModelPart>) -> ModelPart {
    ModelPart {
        name: name.to_string(),
        id: intern(name),
        pivot: Vec3::ZERO,
        rotation,
        visible: true,
        cubes: Vec::new(),
        children,
    }
}

/// A parrot with the parts the shoulder pose moves, all at rest (zero base rotation).
fn parrot() -> CubeModel {
    let children = ["head", "left_wing", "right_wing", "tail"]
        .iter()
        .map(|n| part(n, Vec3::ZERO, Vec::new()))
        .collect();
    CubeModel {
        texture: Identifier::minecraft("entity/parrot/parrot_red_blue"),
        tex_width: 32,
        tex_height: 32,
        root: part("root", Vec3::ZERO, children),
    }
}

fn rotation_of(pose: &PoseOverrides, name: &str) -> Vec3 {
    pose.rotation_of(intern(name)).unwrap_or(Vec3::ZERO)
}

#[test]
fn shoulder_offsets_match_vanilla_in_model_units() {
    // 0.4 blocks to the side, 1.5 down (1.3 crouching), 1/16 block per model unit.
    assert_eq!(shoulder_offset(true, false), Vec3::new(6.4, -24.0, 0.0));
    assert_eq!(shoulder_offset(false, false), Vec3::new(-6.4, -24.0, 0.0));
    assert_eq!(shoulder_offset(true, true), Vec3::new(6.4, -20.8, 0.0));
    assert_eq!(shoulder_offset(false, true), Vec3::new(-6.4, -20.8, 0.0));
}

#[test]
fn wings_are_folded_to_the_vanilla_roll() {
    let model = parrot();
    let pose = shoulder_pose(&model, ShoulderPose::default());
    assert!((rotation_of(&pose, "left_wing").z + 0.0873).abs() < 1e-6);
    assert!((rotation_of(&pose, "right_wing").z - 0.0873).abs() < 1e-6);
}

#[test]
fn head_follows_the_players_head_and_the_tail_sways_with_the_walk() {
    let model = parrot();
    let pose = shoulder_pose(
        &model,
        ShoulderPose {
            head_pitch: 0.3,
            head_yaw: -0.5,
            walk_pos: 0.0,
            walk_speed: 1.0,
        },
    );
    let head = rotation_of(&pose, "head");
    assert!((head.x - 0.3).abs() < 1e-6);
    assert!((head.y + 0.5).abs() < 1e-6);
    // cos(0) * 0.3 * speed 1.0.
    assert!((rotation_of(&pose, "tail").x - 0.3).abs() < 1e-6);
    // The shoulder pose never flaps: no roll on the head or tail.
    assert_eq!(head.z, 0.0);
}

#[test]
fn a_standing_parrot_on_a_still_player_has_no_tail_sway() {
    let model = parrot();
    let pose = shoulder_pose(&model, ShoulderPose::default());
    assert_eq!(rotation_of(&pose, "tail"), Vec3::ZERO);
    assert_eq!(rotation_of(&pose, "head"), Vec3::ZERO);
}
