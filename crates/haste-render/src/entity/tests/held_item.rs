//! The held-item and worn-head transforms as `emit_equipment` composes them: the entity
//! root through the arm or head part, across the pixel-units bridge, into vanilla's layer
//! transforms — the item drawn at block scale at the hand, not the model's pixel scale.
use super::*;

/// `CustomHeadLayer.translateToHead`: translate (0, -0.25, 0), rotate Y 180, scale
/// (0.625, -0.625, -0.625).
#[test]
fn the_custom_head_transform_is_vanillas() {
    let deg = |d: f32| d.to_radians();
    let want = Mat4::from_translation(Vec3::new(0.0, -0.25, 0.0))
        * Mat4::from_rotation_y(deg(180.0))
        * Mat4::from_scale(Vec3::new(0.625, -0.625, -0.625));
    assert!(close(custom_head_layer_matrix(), want, 1e-6));
}

/// The composed third-person item root is vanilla's `translateToHand`: the entity's flip
/// and scale, the arm's pivot in block units, then `ItemInHandLayer.submitArmWithItem`'s
/// transform. The translation to the entity's position cancels out of the comparison.
#[test]
fn the_third_person_item_root_is_vanillas_translate_to_hand() {
    let model = haste_entity_test_humanoid();
    let pose = PoseOverrides::default();
    let arm_m = part_matrix(&model.root, &pose, Mat4::IDENTITY, parts::RIGHT_ARM).unwrap();
    // The entity root with yaw 0 and no body transform: the flip and the pixel scale
    // (`LivingEntityRenderer.submit`'s scale(-1, -1, 1) and the model's /16).
    let root = Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0) / 16.0);
    let haste = root * arm_m * equipment::pixel_units() * held_item_layer_matrix(false);
    // Vanilla: the flip at block scale, the arm pivot / 16 (`ModelPart.translateAndRotate`),
    // then `ItemInHandLayer.submitArmWithItem`'s rotations and hand offset.
    let deg = |d: f32| d.to_radians();
    let mut want = Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0));
    want *= Mat4::from_translation(Vec3::new(-5.0, -22.0, 0.0) / 16.0);
    want *= Mat4::from_rotation_x(deg(-90.0));
    want *= Mat4::from_rotation_y(deg(180.0));
    want *= Mat4::from_translation(Vec3::new(1.0 / 16.0, 2.0 / 16.0, -10.0 / 16.0));
    assert!(close(haste, want, 1e-5), "haste {haste:?} want {want:?}");
}

/// The composed custom-head item root is vanilla's `CustomHeadLayer.translateToHead`: the
/// head part's pivot in block units, the (0, -0.25, 0) offset, the Y turn and the
/// (0.625, -0.625, -0.625) scale, all at block scale.
#[test]
fn the_custom_head_item_root_is_vanillas_translate_to_head() {
    // The test humanoid's right arm stands in for a head part (the composition under
    // test is the same); its pivot is (-5, -22, 0).
    let model = haste_entity_test_humanoid();
    let pose = PoseOverrides::default();
    let Some(head_m) = part_matrix(&model.root, &pose, Mat4::IDENTITY, parts::RIGHT_ARM) else {
        panic!("the test humanoid has no arm part");
    };
    let root = Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0) / 16.0);
    let haste = root * head_m * equipment::pixel_units() * custom_head_layer_matrix();
    let deg = |d: f32| d.to_radians();
    let mut want = Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0));
    want *= Mat4::from_translation(Vec3::new(-5.0, -22.0, 0.0) / 16.0);
    want *= Mat4::from_translation(Vec3::new(0.0, -0.25, 0.0));
    want *= Mat4::from_rotation_y(deg(180.0));
    want *= Mat4::from_scale(Vec3::new(0.625, -0.625, -0.625));
    assert!(close(haste, want, 1e-5), "haste {haste:?} want {want:?}");
}
