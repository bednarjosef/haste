use super::*;
/// `LivingEntityRenderer.setupRotations`: `fall = sqrt((deathTime - 1) / 20 * 1.6)` clamped to 1,
/// rotated about Z by `fall * 90`. deathTime is 0 until the first death tick, so the tilt is 0
/// then, and reaches the full 90 degrees once `fall` saturates.
#[test]
fn death_tilt_rotates_over_twenty_ticks() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.mark_dead(EntityId(1));
    // deathTime 0 (not yet ticked): no rotation at all.
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(crate::animation::body_transform(e, 0.0, false).len, 0);
    // deathTime 1: fall = 0 -> Z rotation of 0.
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    let b = crate::animation::body_transform(e, 0.0, false);
    assert_eq!(b.len, 1);
    assert!(matches!(b.ops[0], BodyOp::RotateZ(a) if a.abs() < 1e-6));
    // deathTime 21: fall = 1.6 -> sqrt -> clamp 1 -> 90 degrees.
    for _ in 0..20 {
        s.tick();
    }
    let e = s.get(EntityId(1)).unwrap();
    let b = crate::animation::body_transform(e, 1.0, false);
    match b.ops[0] {
        BodyOp::RotateZ(a) => {
            assert!((a - std::f32::consts::FRAC_PI_2).abs() < 1e-4, "tilt {a}");
        }
        other => panic!("expected RotateZ, got {other:?}"),
    }
}

/// `LivingEntityRenderer.setupRotations`: a mob named `Dinnerbone`/`Grumm` (custom name metadata
/// index 2) is lifted by `(boundingBoxHeight + 0.1)` and rolled 180 degrees; its look pitch and
/// head yaw are negated (`extractRenderState`).
#[test]
fn dinnerbone_is_upside_down() {
    let model = mob_model("entity/humanoid", &["head", "right_arm", "left_arm"]);
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: 2,
            value: MetadataValue::OptionalComponent(Some(haste_core::Component::text(
                "Dinnerbone",
            ))),
        }],
    );
    s.set_rotation(EntityId(1), 0.0, 30.0, true);
    let e = s.get(EntityId(1)).unwrap();
    let b = crate::animation::body_transform(e, 0.0, false);
    assert!(matches!(b.ops[0], BodyOp::Translate(t) if (t.y - 1.9).abs() < 1e-5));
    assert!(matches!(b.ops[1], BodyOp::RotateZ(a) if (a - std::f32::consts::PI).abs() < 1e-5));
    let pose = crate::animation::setup_anim(e, &model, true, 0.0, false);
    let rot: std::collections::HashMap<_, _> = pose.rotations.iter().cloned().collect();
    let head = rot.get("head").unwrap();
    assert!(
        (head.x - -30.0f32.to_radians()).abs() < 1e-5,
        "upside-down head pitch {} != -30 deg",
        head.x
    );
    // A name that is not Dinnerbone/Grumm leaves it upright.
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: 2,
            value: MetadataValue::OptionalComponent(Some(haste_core::Component::text("Steve"))),
        }],
    );
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(crate::animation::body_transform(e, 0.0, false).len, 0);
}

/// `LivingEntityRenderer.setupRotations`'s sleeping branch: the `180 - bodyRot` yaw is replaced
/// by `rotateY(angle); rotateZ(90); rotateY(270)` (the bed orientation is not tracked, so the
/// body yaw fallback is used, as vanilla does when it is null).
#[test]
fn sleeping_pose_replaces_the_body_yaw() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: 6,
            value: MetadataValue::Pose(Pose::Sleeping),
        }],
    );
    let e = s.get(EntityId(1)).unwrap();
    let b = crate::animation::body_transform(e, 0.0, false);
    assert!(b.suppress_yaw);
    assert_eq!(b.len, 3);
    assert!(
        matches!(b.ops[1], BodyOp::RotateZ(a) if (a - std::f32::consts::FRAC_PI_2).abs() < 1e-5)
    );
    assert!(matches!(b.ops[2], BodyOp::RotateY(a) if (a - 270.0f32.to_radians()).abs() < 1e-5));
}

/// `AvatarRenderer.setupRotations`: a fall-flying player is pitched by
/// `fallFlyingScale * (-90 - xRot)` about X; after 10 fall-flying ticks the scale is 1.
#[test]
fn fall_flying_player_pitch() {
    let mut s = store();
    let sp = spawn(1, DVec3::ZERO);
    let uuid = sp.uuid;
    s.player_info(&[PlayerInfo {
        uuid,
        name: "Flyer".into(),
        display_name: None,
        latency: 20,
        game_mode: Some(GameMode::Survival),
        listed: true,
        skin_texture_url: None,
    }]);
    s.spawn(&sp);
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: 0,
            value: MetadataValue::Byte(0x80u8 as i8),
        }],
    );
    s.set_rotation(EntityId(1), 0.0, 20.0, true);
    for _ in 0..12 {
        s.tick();
    }
    let e = s.get(EntityId(1)).unwrap();
    let b = crate::animation::body_transform(e, 0.0, false);
    match b.ops[0] {
        BodyOp::RotateX(a) => {
            let want = (-90.0f32 - 20.0).to_radians();
            assert!((a - want).abs() < 1e-4, "fly pitch {a} != {want}");
        }
        other => panic!("expected RotateX, got {other:?}"),
    }
}
