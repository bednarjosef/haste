use super::*;
#[test]
fn walking_animation_zero_at_rest_and_bounded() {
    let mut s = store();
    let sp = spawn(1, DVec3::ZERO);
    let uuid = sp.uuid;
    s.player_info(&[PlayerInfo {
        uuid,
        name: "Walker".into(),
        display_name: None,
        latency: 20,
        game_mode: Some(GameMode::Survival),
        listed: true,
        skin_texture_url: None,
    }]);
    s.spawn(&sp);
    // A few idle ticks: the walk swing (the arm/leg x rotation) stays zero; only the constant
    // leg yaw/z and the idle arm bob (`AnimationUtils.bobModelPart`, <= 0.05) remain.
    for _ in 0..5 {
        s.tick();
    }
    let mut out = Vec::new();
    s.render_instances(0.5, &mut out);
    let rot = rotations_by_name(&out[0].pose);
    for limb in ["right_leg", "left_leg"] {
        let r = rot.get(limb).expect(limb);
        assert!(r.x.abs() < 1e-3, "{limb} swings at rest: {}", r.x);
    }
    for limb in ["right_arm", "left_arm"] {
        let r = rot.get(limb).expect(limb);
        assert!(r.x.abs() <= 0.05, "{limb} swings at rest: {}", r.x);
    }
    // Walking: constant 0.2-block steps each tick; the swing amplitude is bounded < pi/2.
    for i in 1..=20 {
        if i == 1 || i == 4 {
            s.move_delta(EntityId(1), DVec3::new(0.6, 0.0, 0.0), true);
        }
        s.tick();
    }
    let mut out = Vec::new();
    s.render_instances(0.5, &mut out);
    let rot = rotations_by_name(&out[0].pose);
    let arm = rot.get("right_arm").expect("arm rotation while walking");
    assert!(
        arm.x.abs() > 1e-3,
        "arm should swing while walking, got {}",
        arm.x
    );
    assert!(
        arm.x.abs() <= std::f32::consts::FRAC_PI_2,
        "arm swing unbounded: {}",
        arm.x
    );
    let leg = rot.get("right_leg").expect("leg rotation while walking");
    assert!(leg.x.abs() > 1e-3);
    assert!(leg.x.abs() <= std::f32::consts::FRAC_PI_2);
    // Arms and legs on the same side are out of phase (opposite sign at some phase; here just
    // check they differ, which a standing wave would not).
    let left = rot.get("left_arm").unwrap().x;
    assert!(
        (arm.x - left).abs() > 1e-6 || arm.x.abs() < 1e-6,
        "arms in lockstep"
    );
}

/// A humanoid player walking at exactly 0.2 blocks/tick (a 0.6-block delta lands over the 3-tick
/// lerp window): after 5 ticks the walk position/speed are vanilla's, and the leg swing is
/// `cos(position * 0.6662) * 1.4 * speed` (`HumanoidModel.setupAnim`). The expected numbers come
/// from the vanilla recurrence `speed += (0.8 - speed) * 0.4; position += speed`.
#[test]
fn walk_leg_angle_after_five_ticks_at_point_two() {
    let mut s = store();
    let sp = spawn(1, DVec3::ZERO);
    let uuid = sp.uuid;
    s.player_info(&[PlayerInfo {
        uuid,
        name: "Walker".into(),
        display_name: None,
        latency: 20,
        game_mode: Some(GameMode::Survival),
        listed: true,
        skin_texture_url: None,
    }]);
    s.spawn(&sp);
    for i in 1..=5 {
        if i == 1 || i == 4 {
            s.move_delta(EntityId(1), DVec3::new(0.6, 0.0, 0.0), true);
        }
        s.tick();
    }
    let mut out = Vec::new();
    s.render_instances(1.0, &mut out);
    let rot = rotations_by_name(&out[0].pose);
    let rl = rot.get("right_leg").unwrap();
    let ll = rot.get("left_leg").unwrap();
    assert!(
        (rl.x - -0.360_702_3).abs() < 2e-3,
        "right leg {} != -0.36070",
        rl.x
    );
    assert!(
        (ll.x - 0.360_702_3).abs() < 2e-3,
        "left leg {} != 0.36070",
        ll.x
    );
}

/// A player mid-attack: a 6-tick swing (`SwingAnimation.DEFAULT`) advanced 4 ticks is at 0.5,
/// and `HumanoidModel.setupAttackAnimation` (WHACK, main hand right, pitch 0, age 5) leaves the
/// right arm at x -0.7427, y -0.5783, z -0.3050 (walk swing zero, idle bob included).
#[test]
fn attack_arm_angle_mid_swing() {
    let mut s = store();
    let sp = spawn(1, DVec3::ZERO);
    let uuid = sp.uuid;
    s.player_info(&[PlayerInfo {
        uuid,
        name: "Attacker".into(),
        display_name: None,
        latency: 20,
        game_mode: Some(GameMode::Survival),
        listed: true,
        skin_texture_url: None,
    }]);
    s.spawn(&sp);
    s.swing(EntityId(1), 0, 1, 6);
    for _ in 0..4 {
        s.tick();
    }
    let mut out = Vec::new();
    s.render_instances(1.0, &mut out);
    let rot = rotations_by_name(&out[0].pose);
    let ra = rot.get("right_arm").unwrap();
    assert!(
        (ra.x - -0.742_670).abs() < 2e-3,
        "arm x {} != -0.74267",
        ra.x
    );
    assert!(
        (ra.y - -0.578_342).abs() < 2e-3,
        "arm y {} != -0.57834",
        ra.y
    );
    assert!(
        (ra.z - -0.304_978).abs() < 2e-3,
        "arm z {} != -0.30498",
        ra.z
    );
}

/// `AnimationUtils.animateZombieArms`: a zombie holds its arms forward even at rest
/// (`-PI / 2.25`), overriding the humanoid walk swing. A `minecraft:zombie` model is recognised
/// from its texture; here the parts/texture are built directly.
#[test]
fn zombie_arms_are_forward_at_rest() {
    let model = mob_model(
        "minecraft:entity/zombie/zombie",
        &["head", "right_arm", "left_arm"],
    );
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    let e = s.get(EntityId(1)).unwrap();
    let pose = crate::animation::setup_anim(e, &model, true, 0.0, false);
    let rot = rotations_by_name(&pose);
    let ra = rot.get("right_arm").unwrap();
    assert!(
        (ra.x - -1.396_263_4).abs() < 1e-4,
        "zombie right arm {} != -1.39626",
        ra.x
    );
    assert!(
        (ra.y - -0.1).abs() < 1e-4,
        "zombie right arm y {} != -0.1",
        ra.y
    );
    // The idle arm bob (age 0) adds 0.1 to z on top of the zombie pose.
    assert!(
        (ra.z - 0.1).abs() < 1e-4,
        "zombie right arm z {} != 0.1",
        ra.z
    );
}

/// `Mob.DATA_MOB_FLAGS_ID` is metadata index 15 (Entity's 8 + LivingEntity's 7); bit 2 is
/// left-handed, bit 4 aggressive (`Mob.isLeftHanded`/`isAggressive`).
#[test]
fn mob_flags_byte_aggressive_and_left_handed() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: 15,
            value: MetadataValue::Byte(0x04 | 0x02),
        }],
    );
    let m = &s.get(EntityId(1)).unwrap().metadata;
    assert!(m.mob.aggressive);
    assert!(m.mob.left_handed);
    // Clearing the byte clears both bits.
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: 15,
            value: MetadataValue::Byte(0),
        }],
    );
    let m = &s.get(EntityId(1)).unwrap().metadata;
    assert!(!m.mob.aggressive);
    assert!(!m.mob.left_handed);
}

/// `AnimationUtils.animateZombieArms` with `aggressive = true`: `armDrop = -PI / 1.5`
/// (`-2.09440`), the adult raiseArms branch, at rest with age 0 (idle bob adds z 0.1).
#[test]
fn aggressive_zombie_arms_are_raised() {
    let model = mob_model(
        "minecraft:entity/zombie/zombie",
        &["head", "right_arm", "left_arm"],
    );
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: 15,
            value: MetadataValue::Byte(0x04),
        }],
    );
    let e = s.get(EntityId(1)).unwrap();
    let pose = crate::animation::setup_anim(e, &model, true, 0.0, false);
    let rot = rotations_by_name(&pose);
    let ra = rot.get("right_arm").unwrap();
    assert!(
        (ra.x - -2.094_395_2).abs() < 1e-4,
        "aggressive zombie right arm {} != -2.09440",
        ra.x
    );
    assert!((ra.y - -0.1).abs() < 1e-4);
    assert!((ra.z - 0.1).abs() < 1e-4);
}

/// `SkeletonModel.setupAnim`: an aggressive skeleton not holding a bow raises both arms to
/// `-PI/2` (with the swing offsets) and bobs them; this replaces the humanoid walk swing.
#[test]
fn aggressive_skeleton_arms_raised() {
    let model = mob_model(
        "minecraft:entity/skeleton/skeleton",
        &["head", "right_arm", "left_arm"],
    );
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: 15,
            value: MetadataValue::Byte(0x04),
        }],
    );
    let e = s.get(EntityId(1)).unwrap();
    let pose = crate::animation::setup_anim(e, &model, true, 0.0, false);
    let rot = rotations_by_name(&pose);
    let ra = rot.get("right_arm").unwrap();
    let la = rot.get("left_arm").unwrap();
    assert!(
        (ra.x - -std::f32::consts::FRAC_PI_2).abs() < 1e-4,
        "skeleton right arm {} != -PI/2",
        ra.x
    );
    assert!((ra.y - -0.1).abs() < 1e-4);
    assert!((ra.z - 0.1).abs() < 1e-4);
    assert!((la.y - 0.1).abs() < 1e-4);
    assert!((la.z - -0.1).abs() < 1e-4);
}

/// A quadruped model (pig/cow/sheep: `right_hind_leg`/`left_front_leg` parts) gets
/// `QuadrupedModel.setupAnim`: the hind and front legs on one side are out of phase.
#[test]
fn quadruped_legs_swing_out_of_phase() {
    let model = mob_model(
        "minecraft:entity/pig/pig_temperate",
        &[
            "head",
            "right_hind_leg",
            "left_hind_leg",
            "right_front_leg",
            "left_front_leg",
        ],
    );
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    for _ in 0..10 {
        s.move_delta(EntityId(1), DVec3::new(0.1, 0.0, 0.0), true);
        s.tick();
    }
    let e = s.get(EntityId(1)).unwrap();
    let pose = crate::animation::setup_anim(e, &model, true, 1.0, false);
    let rot = rotations_by_name(&pose);
    let rh = rot.get("right_hind_leg").unwrap();
    let rf = rot.get("right_front_leg").unwrap();
    let lh = rot.get("left_hind_leg").unwrap();
    assert!(rh.x.abs() > 1e-3, "hind leg not swinging");
    // Opposite phase: the right hind and right front legs have opposite sign here.
    assert!(rh.x * rf.x <= 0.0, "hind {} front {} in phase", rh.x, rf.x);
    assert!((lh.x + rh.x).abs() < 1e-4, "left/right hind not mirrored");
}

/// `HumanoidModel.setupAnim`'s crouch moves the pivots too: head down 4.2, body and arms down
/// 3.2, legs back 4 (model units), so the leaning body stays joined to the legs. Standing
/// leaves every pivot where the model puts it.
#[test]
fn crouching_humanoid_drops_head_body_arms_and_moves_legs_back() {
    use haste_core::parts;
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.tick();
    let model = crate::models::humanoid(false);
    let standing =
        crate::animation::setup_anim(s.get(EntityId(1)).unwrap(), &model, true, 1.0, false);
    for id in [
        parts::HEAD,
        parts::BODY,
        parts::RIGHT_ARM,
        parts::LEFT_ARM,
        parts::RIGHT_LEG,
    ] {
        assert_eq!(standing.offset_of(id), None);
    }
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: 0,
            value: MetadataValue::Byte(0x02),
        }],
    );
    s.tick();
    let pose = crate::animation::setup_anim(s.get(EntityId(1)).unwrap(), &model, true, 1.0, false);
    let y = |v: f32| Some(haste_core::math::Vec3::new(0.0, v, 0.0));
    let back = Some(haste_core::math::Vec3::new(0.0, 0.0, 4.0));
    assert_eq!(pose.offset_of(parts::HEAD), y(4.2));
    assert_eq!(pose.offset_of(parts::BODY), y(3.2));
    assert_eq!(pose.offset_of(parts::RIGHT_ARM), y(3.2));
    assert_eq!(pose.offset_of(parts::LEFT_ARM), y(3.2));
    assert_eq!(pose.offset_of(parts::RIGHT_LEG), back);
    assert_eq!(pose.offset_of(parts::LEFT_LEG), back);
}
