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

/// The left hand draws through the model's own `thirdperson_lefthand` entry with the fix
/// on top (`ItemStackRenderState` uses `ItemTransforms.getTransform(THIRD_PERSON_LEFT_HAND)`
/// and then `ItemTransform.apply(true, ...)`): `item/handheld`'s left entry [0, 90, -55]
/// cancels the fix back to the right entry's [0, -90, 55] — not the mirrored-right
/// [0, 90, -55] a plain fix of the right entry would give (that held the sword pointing up).
#[test]
fn the_left_hand_uses_the_models_left_entry() {
    let part = haste_assets::ItemModelPart {
        geometry: haste_assets::ItemGeometry::None,
        tints: vec![],
        gui: haste_assets::DisplayTransform::IDENTITY,
        firstperson: haste_assets::DisplayTransform::IDENTITY,
        thirdperson: haste_assets::DisplayTransform {
            rotation: [0.0, -90.0, 55.0],
            translation: [0.0, 0.25, 0.5 / 16.0],
            scale: [0.85; 3],
        },
        thirdperson_left: haste_assets::DisplayTransform {
            rotation: [0.0, 90.0, -55.0],
            translation: [0.0, 0.25, 0.5 / 16.0],
            scale: [0.85; 3],
        },
        head: haste_assets::DisplayTransform::IDENTITY,
        ground: haste_assets::DisplayTransform::IDENTITY,
        fixed: haste_assets::DisplayTransform::IDENTITY,
        on_shelf: haste_assets::DisplayTransform::IDENTITY,
        gui_light_front: false,
        transformation: None,
    };
    // Vanilla: apply(true) on the left entry — the fix negates y and z of [0, 90, -55]
    // back to the right entry's angle [0, -90, 55].
    let got = item_display_matrix(&part, &part.thirdperson_left, true);
    let want = crate::gui::display_matrix(&haste_assets::DisplayTransform {
        rotation: [0.0, -90.0, 55.0],
        translation: [0.0, 0.25, 0.5 / 16.0],
        scale: [0.85; 3],
    }) * Mat4::from_translation(Vec3::splat(-0.5));
    assert!(close(got, want, 1e-6), "got {got:?} want {want:?}");
    // The mirrored-right mistake: the fix of the right entry alone.
    let wrong = crate::gui::display_matrix(&haste_assets::DisplayTransform {
        rotation: [0.0, 90.0, -55.0],
        translation: [0.0, 0.25, 0.5 / 16.0],
        scale: [0.85; 3],
    }) * Mat4::from_translation(Vec3::splat(-0.5));
    assert!(
        !close(got, wrong, 1e-3),
        "left hand mirrors the right entry"
    );
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
