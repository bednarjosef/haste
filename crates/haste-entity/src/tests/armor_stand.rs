//! The armour stand's model, pose and hit wobble against vanilla's constants
//! (`ArmorStandModel`, `ArmorStandArmorModel`, `ArmorStandRenderer`, 26.3 client).
use super::*;
use haste_core::model::BodyOp;

/// The synced entries for a stand with given flags and poses (indices per
/// `ArmorStand.defineSynchedData`: 15 the flags byte, 16..=21 the `Rotations`).
fn stand_metadata(flags: Option<i8>, poses: [Option<[f32; 3]>; 6]) -> Vec<MetadataEntry> {
    let mut e = Vec::new();
    if let Some(b) = flags {
        e.push(MetadataEntry {
            index: 15,
            value: MetadataValue::Byte(b),
        });
    }
    for (i, p) in poses.into_iter().enumerate() {
        if let Some(r) = p {
            e.push(MetadataEntry {
                index: 16 + i as u8,
                value: MetadataValue::Rotations(r),
            });
        }
    }
    e
}

fn armor_stand_entity(metadata: &[MetadataEntry]) -> EntityStore {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.set_metadata(EntityId(1), metadata);
    s
}

/// One stand part's pivot by name.
fn pivot_of(m: &CubeModel, name: &str) -> haste_core::Vec3 {
    find_part(&m.root, name).unwrap().pivot
}

#[test]
fn body_model_matches_the_vanilla_layer() {
    let m = crate::armor_stand::body_model(false);
    // Ten root children (the humanoid six, minus the hat, plus sticks and plate).
    assert_eq!(m.root.children.len(), 10);
    assert_eq!(m.tex_width, 64);
    assert_eq!(m.tex_height, 64);
    assert_eq!(m.texture.path(), "entity/armorstand/armorstand");
    // Vanilla `PartPose`s shifted to Haste space (ground y = 0): head (0,1,0), body at the
    // waist, legs pivoting at y=12, the plate at y=12.
    assert_eq!(pivot_of(&m, "head"), haste_core::Vec3::new(0.0, -23.0, 0.0));
    assert_eq!(pivot_of(&m, "body"), haste_core::Vec3::new(0.0, -24.0, 0.0));
    assert_eq!(
        pivot_of(&m, "right_arm"),
        haste_core::Vec3::new(-5.0, -22.0, 0.0)
    );
    assert_eq!(
        pivot_of(&m, "left_leg"),
        haste_core::Vec3::new(1.9, -12.0, 0.0)
    );
    assert_eq!(
        pivot_of(&m, "base_plate"),
        haste_core::Vec3::new(0.0, -12.0, 0.0)
    );
    // The vanilla cubes: the head a 2x7x2 post, the plate 12x1x12, the left arm mirrored.
    let head = find_part(&m.root, "head").unwrap();
    assert_eq!(head.cubes[0].size, haste_core::Vec3::new(2.0, 7.0, 2.0));
    assert_eq!(head.cubes[0].uv, [0, 0]);
    assert_eq!(
        find_part(&m.root, "base_plate").unwrap().cubes[0].size,
        haste_core::Vec3::new(12.0, 1.0, 12.0)
    );
    assert!(find_part(&m.root, "left_arm").unwrap().cubes[0].mirror);
    assert!(!find_part(&m.root, "right_arm").unwrap().cubes[0].mirror);
    // The hat vanilla hides is not carried at all.
    assert!(find_part(&m.root, "hat").is_none());
}

#[test]
fn small_body_model_is_the_baby_transform() {
    let big = crate::armor_stand::body_model(false);
    let small = crate::armor_stand::body_model(true);
    // `HumanoidModel.BABY_TRANSFORMER`: heads translate (0,16,0) and scale 3/4 around the
    // ground, the rest (0,24,0) and 1/2 — vanilla pivot (0,12.75,0) for the small head,
    // (0,12,0) for the plate's (0,12,0)+24 = 36 scaled to 18.
    assert_eq!(
        pivot_of(&small, "head"),
        haste_core::Vec3::new(0.0, -11.25, 0.0)
    );
    assert_eq!(
        pivot_of(&small, "base_plate"),
        haste_core::Vec3::new(0.0, -6.0, 0.0)
    );
    assert_eq!(
        pivot_of(&small, "body"),
        haste_core::Vec3::new(0.0, -12.0, 0.0)
    );
    // Cubes keep their original sizes and UV layout: vanilla's part scale shrinks the
    // geometry at render time (the renderer applies it about the pivot), not the UVs.
    assert_eq!(
        find_part(&small.root, "head").unwrap().cubes[0].size,
        haste_core::Vec3::new(2.0, 7.0, 2.0)
    );
    assert_eq!(
        find_part(&small.root, "shoulder_stick").unwrap().cubes[0].size,
        haste_core::Vec3::new(8.0, 2.0, 2.0)
    );
    // Texture and layout are shared.
    assert_eq!(small.texture, big.texture);
    assert_eq!(small.root.children.len(), big.root.children.len());
}

#[test]
fn default_pose_is_vanillas_rest_pose() {
    let s = armor_stand_entity(&[]);
    let e = s.get(EntityId(1)).unwrap();
    let r = |d: f32| d.to_radians();
    // `ArmorStand.DEFAULT_*_POSE`: straight head and body, arms and legs slightly splayed.
    let pose = crate::armor_stand::pose(e, 0.0);
    let by_name = rotations_by_name(&pose);
    assert_eq!(by_name["head"], haste_core::Vec3::ZERO);
    assert_eq!(by_name["body"], haste_core::Vec3::ZERO);
    assert_eq!(
        by_name["left_arm"],
        haste_core::Vec3::new(r(-10.0), 0.0, r(-10.0))
    );
    assert_eq!(
        by_name["right_arm"],
        haste_core::Vec3::new(r(-15.0), 0.0, r(10.0))
    );
    assert_eq!(
        by_name["left_leg"],
        haste_core::Vec3::new(r(-1.0), 0.0, r(-1.0))
    );
    assert_eq!(
        by_name["right_leg"],
        haste_core::Vec3::new(r(1.0), 0.0, r(1.0))
    );
    // The sticks carry the body pose; the default body pose is zero, so they are too.
    assert_eq!(by_name["right_body_stick"], haste_core::Vec3::ZERO);
    assert_eq!(by_name["shoulder_stick"], haste_core::Vec3::ZERO);
    // Without ShowArms the arms are hidden (the flag defaults to 0); the plate shows.
    assert!(pose.is_hidden(haste_core::parts::RIGHT_ARM));
    assert!(pose.is_hidden(haste_core::parts::LEFT_ARM));
    assert!(!pose.is_hidden(haste_core::parts::intern("base_plate")));
}

#[test]
fn synced_pose_and_flags_drive_the_overrides() {
    // Small + ShowArms + NoBasePlate (bits 0x01|0x04|0x08) and a custom head/body pose.
    let s = armor_stand_entity(&stand_metadata(
        Some(0x01 | 0x04 | 0x08),
        [
            Some([30.0, -45.0, 0.0]),
            Some([10.0, 0.0, 5.0]),
            None,
            None,
            None,
            None,
        ],
    ));
    let e = s.get(EntityId(1)).unwrap();
    assert!(crate::armor_stand::is_small(&e.metadata));
    let pose = crate::armor_stand::pose(e, 90.0);
    let by_name = rotations_by_name(&pose);
    assert_eq!(
        by_name["head"],
        haste_core::Vec3::new(30.0f32.to_radians(), (-45.0f32).to_radians(), 0.0)
    );
    // The sticks follow the body pose.
    assert_eq!(by_name["left_body_stick"], by_name["body"]);
    // The base plate counter-rotates the entity's yaw by the vanilla formula
    // `(PI/180) * -yRot` — a yaw of 90 degrees turns the plate by -90 degrees.
    assert_eq!(
        by_name["base_plate"],
        haste_core::Vec3::new(0.0, (-90.0f32).to_radians(), 0.0)
    );
    // ShowArms keeps the arms; NoBasePlate hides the plate.
    assert!(!pose.is_hidden(haste_core::parts::RIGHT_ARM));
    assert!(pose.is_hidden(haste_core::parts::intern("base_plate")));
}

#[test]
fn hit_wobble_is_sin_over_one_and_a_half_pi_times_three_degrees() {
    let mut s = armor_stand_entity(&[]);
    // Never hit: no wobble (vanilla's `lastHit = 0` against a large game time).
    assert_eq!(
        crate::armor_stand::body_transform(s.get(EntityId(1)).unwrap(), 0.0).len,
        0
    );
    s.mark_hit(EntityId(1));
    // `wiggle = gameTime - lastHit + partial`: sin(wiggle / 1.5 * PI) * 3 degrees of yaw
    // while under 5. Sample the wobble across the first ticks and partials.
    let expected = |wiggle: f32| (wiggle / 1.5 * std::f32::consts::PI).sin() * 3.0f32.to_radians();
    let wobble_at = |s: &EntityStore, partial: f32| match crate::armor_stand::body_transform(
        s.get(EntityId(1)).unwrap(),
        partial,
    )
    .ops
    .first()
    {
        Some(BodyOp::RotateY(a)) => *a,
        other => panic!("no wobble rotation, got {other:?}"),
    };
    assert!((wobble_at(&s, 0.0) - expected(0.0)).abs() < 1e-6);
    assert!((wobble_at(&s, 0.5) - expected(0.5)).abs() < 1e-6);
    s.tick();
    assert!((wobble_at(&s, 0.25) - expected(1.25)).abs() < 1e-6);
    s.tick();
    assert!((wobble_at(&s, 0.0) - expected(2.0)).abs() < 1e-6);
    s.tick();
    s.tick();
    assert!((wobble_at(&s, 0.9) - expected(4.9)).abs() < 1e-6);
    // ...and stops once `wiggle` reaches 5.
    s.tick();
    assert_eq!(
        crate::armor_stand::body_transform(s.get(EntityId(1)).unwrap(), 0.0).len,
        0
    );
}

/// The store draws the stand from its own model pair (needs the entity-type names from the
/// datagen reports to resolve `minecraft:armor_stand`).
#[test]
#[ignore = "needs the datagen reports (HASTE_CACHE overrides)"]
fn instances_use_the_stand_model_and_pose() {
    if crate::registry::entity_type_names().is_none() {
        eprintln!("skipped: no datagen reports in the cache");
        return;
    }
    let kind = crate::registry::entity_type_names()
        .unwrap()
        .iter()
        .find(|(_, n)| **n == "minecraft:armor_stand")
        .map(|(k, _)| *k)
        .unwrap();
    let mut sp = spawn(1, DVec3::ZERO);
    sp.kind = kind;
    let mut s = store();
    s.spawn(&sp);
    s.set_metadata(EntityId(1), &stand_metadata(Some(0x01), [None; 6]));
    let mut out = Vec::new();
    s.render_instances(0.0, &mut out);
    let inst = &out[0];
    // The small body model, with the synced pose (a small stand hides no arms by default but
    // its plate shows; the defaults hold).
    assert_eq!(
        inst.model.root.children.len(),
        crate::armor_stand::body_model(true).root.children.len()
    );
    assert_eq!(
        inst.model.root.children[9].name, "base_plate",
        "the plate is the last root child"
    );
    // No shadow (ArmorStandRenderer passes 0.0).
    assert_eq!(inst.shadow_radius, 0.0);
    // The wobble replaced the living setupRotations: no death tilt on a living stand.
    assert_eq!(inst.body.len, 0);
}
