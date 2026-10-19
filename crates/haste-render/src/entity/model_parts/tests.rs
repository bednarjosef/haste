use super::*;

/// A pose pivot offset (vanilla setupAnim's `part.y += ...`) moves the part, and with it
/// every child and the item held at it, before the part's rotation.
#[test]
fn a_pose_offset_moves_the_part_pivot() {
    let arm = ModelPart {
        name: "right_arm".into(),
        id: parts::RIGHT_ARM,
        pivot: Vec3::new(-5.0, -22.0, 0.0),
        rotation: Vec3::ZERO,
        visible: true,
        cubes: Vec::new(),
        children: Vec::new(),
    };
    let root = ModelPart {
        name: "root".into(),
        id: parts::ROOT,
        pivot: Vec3::ZERO,
        rotation: Vec3::ZERO,
        visible: true,
        cubes: Vec::new(),
        children: vec![arm],
    };
    let mut pose = PoseOverrides::default();
    pose.set_offset(parts::RIGHT_ARM, Vec3::new(0.0, 3.2, 0.0));
    pose.set_rotation(
        parts::RIGHT_ARM,
        Vec3::new(0.0, 0.0, std::f32::consts::FRAC_PI_2),
    );
    let m = part_matrix(&root, &pose, Mat4::IDENTITY, parts::RIGHT_ARM).unwrap();
    assert_eq!(m.transform_point3(Vec3::ZERO), Vec3::new(-5.0, -18.8, 0.0));
    // The rotation turns about the moved pivot.
    let p = m.transform_point3(Vec3::new(0.0, 10.0, 0.0));
    assert!(
        (p.x - -15.0).abs() < 1e-4 && (p.y - -18.8).abs() < 1e-4,
        "{p:?}"
    );
}

/// The armour pose keeps the rotations and pivot offsets of the parts the body model names
/// and drops every hidden part: vanilla's armour layers draw their own models, so a part the
/// body model hides (a player's hidden skin layer, an armour stand's hidden arms) still
/// draws its armour piece.
#[test]
fn pose_without_hidden_keeps_rotations_and_offsets() {
    let arm = ModelPart {
        name: "right_arm".into(),
        id: parts::RIGHT_ARM,
        pivot: Vec3::new(-5.0, -22.0, 0.0),
        rotation: Vec3::ZERO,
        visible: true,
        cubes: Vec::new(),
        children: Vec::new(),
    };
    let model = CubeModel {
        texture: Identifier::new("haste", "entity/humanoid"),
        tex_width: 64,
        tex_height: 64,
        root: ModelPart {
            name: "root".into(),
            id: parts::ROOT,
            pivot: Vec3::ZERO,
            rotation: Vec3::ZERO,
            visible: true,
            cubes: Vec::new(),
            children: vec![arm],
        },
    };
    let mut pose = PoseOverrides::default();
    pose.set_rotation(parts::RIGHT_ARM, Vec3::new(0.1, 0.2, 0.3));
    pose.set_offset(parts::RIGHT_ARM, Vec3::new(0.0, 3.2, 0.0));
    pose.hide(parts::RIGHT_ARM);
    let armor = pose_without_hidden(&model, &pose);
    assert_eq!(
        armor.rotation_of(parts::RIGHT_ARM),
        Some(Vec3::new(0.1, 0.2, 0.3))
    );
    assert_eq!(
        armor.offset_of(parts::RIGHT_ARM),
        Some(Vec3::new(0.0, 3.2, 0.0))
    );
    assert!(!armor.is_hidden(parts::RIGHT_ARM));
}
