//! Boats: the client-side `AbstractBoat` state and `AbstractBoatRenderer`'s extraction,
//! against vanilla's own numbers (`Mth`, `BoatModel`, `AbstractBoatModel`,
//! `AbstractBoatRenderer`).

use super::*;
use crate::boat::{
    animate_paddle, body_transform, entry_for_name, is_boat_family, model, rowing_time, setup_anim,
    tick, water_patch_model, water_patch_overlay, BoatState, PADDLE_SPEED,
};
use haste_core::entity::{MetadataEntry, MetadataValue};

/// A boat entity with `metadata` applied, fresh from its spawn packet.
fn boat_entity(metadata: &[MetadataEntry]) -> Entity {
    let mut e = Entity::new(
        &spawn(1, DVec3::new(0.5, 64.0, 0.5)),
        &EntityDimensions::default(),
    );
    e.is_boat = true;
    e.metadata.apply(metadata);
    e
}

fn meta(index: u8, value: MetadataValue) -> MetadataEntry {
    MetadataEntry { index, value }
}

/// `Mth.lerp(value, min, max)` with the factor clamped to 0..1 (`Mth.clampedLerp`).
fn clamped_lerp(factor: f32, min: f32, max: f32) -> f32 {
    min + (max - min) * factor.clamp(0.0, 1.0)
}

/// `AbstractBoat.getRowingTime(side, partialTicks)`: 0 while the paddle is up, the sweep
/// interpolated through the tick while it is in the water (`Mth.clampedLerp(partial,
/// position - PADDLE_SPEED, position)`).
#[test]
fn rowing_time_follows_get_rowing_time() {
    assert_eq!(rowing_time(false, 3.0, 0.5), 0.0);
    // Half-way through a tick whose sweep ended at 3 radians.
    let expected = clamped_lerp(0.5, 3.0 - PADDLE_SPEED, 3.0);
    assert_eq!(rowing_time(true, 3.0, 0.5), expected);
    assert!((rowing_time(true, 3.0, 0.5) - (3.0 - PADDLE_SPEED / 2.0)).abs() < 1.0e-6);
    // The ends of the tick.
    assert_eq!(rowing_time(true, 3.0, 0.0), 3.0 - PADDLE_SPEED);
    assert_eq!(rowing_time(true, 3.0, 1.0), 3.0);
}

/// `AbstractBoatModel.animatePaddle`: both sines swing the factor over 0..1, so the x
/// rotation stays within `-PI/3 .. -PI/12` and the y within `-PI/4 .. PI/4`; the right
/// paddle's y mirrors around PI. The exact ends happen where each sine hits -1 and 1.
#[test]
fn paddle_animation_follows_animate_paddle() {
    let quarter_pi = std::f32::consts::FRAC_PI_4;
    // x's sine `sin(-time)`: -1 at time = PI/2 (factor 0), 1 at time = -PI/2 (factor 1).
    let (x_low, _) = animate_paddle(std::f32::consts::FRAC_PI_2, false);
    let (x_high, _) = animate_paddle(-std::f32::consts::FRAC_PI_2, false);
    assert!((x_low - (-std::f32::consts::FRAC_PI_3)).abs() < 1.0e-6);
    assert!((x_high + std::f32::consts::PI / 12.0).abs() < 1.0e-6);
    // y's sine `sin(-time + 1)`: -1 at time = 1 + PI/2 (factor 0), 1 at 1 - PI/2 (factor 1).
    let (_, y_low) = animate_paddle(1.0 + std::f32::consts::FRAC_PI_2, false);
    let (_, y_high) = animate_paddle(1.0 - std::f32::consts::FRAC_PI_2, false);
    assert!((y_low + quarter_pi).abs() < 1.0e-6);
    assert!((y_high - quarter_pi).abs() < 1.0e-6);
    // The right paddle's y rotation mirrors (`PI - yRot`), its x does not.
    let (x, y) = animate_paddle(0.3, true);
    let (x_left, y_left) = animate_paddle(0.3, false);
    assert_eq!(x, x_left);
    assert!((y - (std::f32::consts::PI - y_left)).abs() < 1.0e-6);
    // A sweep never leaves vanilla's bounds.
    for t in -20i32..20 {
        let right = t % 2 == 0;
        let (x, y) = animate_paddle(t as f32 * 0.25, right);
        assert!(
            (-std::f32::consts::FRAC_PI_3 - 1.0e-5..=std::f32::consts::PI / 12.0 + 1.0e-5)
                .contains(&x)
        );
        let plain_y = if right { std::f32::consts::PI - y } else { y };
        assert!((-quarter_pi - 1.0e-5..=quarter_pi + 1.0e-5).contains(&plain_y));
    }
}

/// `AbstractBoat.tick`'s client half (`tickBubbleColumn` and the paddle sweeps): the bubble
/// multiplier eases in at 0.05 and out at 0.1 per tick, the rocking is
/// `10 * sin(tickCount / 2) * multiplier`, and each paddle advances `PADDLE_SPEED` while its
/// flag is set and resets when it is not.
#[test]
fn tick_advances_the_paddles_and_the_bubble_rocking() {
    let world = crate::item_entity::NoWorld;
    let mut e = boat_entity(&[
        meta(11, MetadataValue::Boolean(true)),
        meta(12, MetadataValue::Boolean(false)),
        meta(13, MetadataValue::VarInt(60)),
    ]);
    // The store's tick loop advances `tickCount` first (`Entity.tick` before
    // `tickBubbleColumn`); the direct calls here mirror that order.
    let tick_boat = |e: &mut Entity| {
        e.tick_count += 1;
        tick(e, &world, 0.5625);
    };
    tick_boat(&mut e);
    assert_eq!(e.boat.paddle_positions[0], PADDLE_SPEED);
    assert_eq!(e.boat.paddle_positions[1], 0.0);
    assert!((e.boat.bubble_multiplier - 0.05).abs() < 1.0e-6);
    // `bubbleAngleO` is the previous tick's angle (0 before the first).
    assert_eq!(e.boat.bubble_angle_o, 0.0);
    assert!(
        (e.boat.bubble_angle - 10.0 * (0.5_f32).sin() * 0.05).abs() < 1.0e-6,
        "{}",
        e.boat.bubble_angle
    );
    // The NoWorld view holds no water: a floating boat is not under it.
    assert!(!e.boat.underwater);
    // Twenty ticks in a bubble column saturate the multiplier at 1.
    for _ in 0..19 {
        tick_boat(&mut e);
    }
    assert!((e.boat.bubble_multiplier - 1.0).abs() < 1.0e-6);
    assert!((e.boat.paddle_positions[0] - 20.0 * PADDLE_SPEED).abs() < 1.0e-5);
    // The column stops: the multiplier eases back out at twice the speed, the paddle resets.
    e.metadata.apply(&[
        meta(13, MetadataValue::VarInt(0)),
        meta(11, MetadataValue::Boolean(false)),
    ]);
    tick_boat(&mut e);
    assert!((e.boat.bubble_multiplier - 0.9).abs() < 1.0e-6);
    assert_eq!(e.boat.paddle_positions[0], 0.0);
    // Ten ticks out it rests at 0.
    for _ in 0..9 {
        tick_boat(&mut e);
    }
    assert_eq!(e.boat.bubble_multiplier, 0.0);
    assert_eq!(e.boat.bubble_angle, 0.0);
}

/// `AbstractBoatRenderer.extractRenderState`/`submit`: the hurt wobble is
/// `sin(hurtTime) * hurtTime * damage / 10 * hurtDir` degrees... radians about x, with the
/// hurt time and damage both counted down by the partial tick.
#[test]
fn hurt_wobble_follows_the_synced_hurt_fields() {
    let e = boat_entity(&[
        meta(8, MetadataValue::VarInt(10)),
        meta(9, MetadataValue::VarInt(-1)),
        meta(10, MetadataValue::Float(40.0)),
    ]);
    let b = body_transform(&e, 0.0);
    // The lift before the yaw (`poseStack.translate(0, 0.375, 0)`).
    assert_eq!(b.pre_translate, glam::Vec3::new(0.0, 0.375, 0.0));
    let expected = -(10.0_f32.sin() * 10.0 * 40.0 / 10.0);
    match b.iter().next() {
        Some(haste_core::model::BodyOp::RotateX(a)) => assert!((a - expected).abs() < 1.0e-4),
        other => panic!("expected the hurt wobble, got {other:?}"),
    }
    // Half a tick in, both counters have counted down.
    let b = body_transform(&e, 0.5);
    let expected = -(9.5_f32.sin() * 9.5 * 39.5 / 10.0);
    match b.iter().next() {
        Some(haste_core::model::BodyOp::RotateX(a)) => assert!((a - expected).abs() < 1.0e-4),
        other => panic!("expected the hurt wobble, got {other:?}"),
    }
    // No hurt time, no wobble: the transform is only the lift.
    let e = boat_entity(&[]);
    let b = body_transform(&e, 0.5);
    assert_eq!(b.len, 0);
}

/// `AbstractBoatRenderer.submit`'s bubble-column rocking: a rotation by `bubbleAngle` degrees
/// about the (1, 0, 1) axis, expressed as the three body rotations
/// `Ry(-PI/4) * Rx(angle) * Ry(PI/4)`, and only while the boat is not under water.
#[test]
fn bubble_rocking_is_the_diagonal_axis_rotation() {
    let mut e = boat_entity(&[]);
    e.boat.bubble_angle_o = 30.0;
    e.boat.bubble_angle = 22.5;
    let b = body_transform(&e, 0.5);
    // `Mth.lerp(0.5, 30, 22.5)`.
    let angle = 26.25_f32.to_radians();
    let ops: Vec<_> = b.iter().cloned().collect();
    match ops.as_slice() {
        [haste_core::model::BodyOp::RotateY(a), haste_core::model::BodyOp::RotateX(x), haste_core::model::BodyOp::RotateY(c)] =>
        {
            let quarter = std::f32::consts::FRAC_PI_4;
            assert!((a + quarter).abs() < 1.0e-5 && (c - quarter).abs() < 1.0e-5);
            assert!((x - angle).abs() < 1.0e-5);
            // The composed rotation equals the axis-angle form about the diagonal.
            let m = glam::Mat4::from_rotation_y(*a)
                * glam::Mat4::from_rotation_x(*x)
                * glam::Mat4::from_rotation_y(*c);
            let axis = glam::Vec3::new(1.0, 0.0, 1.0).normalize();
            let expected = glam::Mat4::from_axis_angle(axis, angle);
            for (got, want) in m.to_cols_array().iter().zip(expected.to_cols_array()) {
                assert!((got - want).abs() < 1.0e-5, "{got} vs {want}");
            }
        }
        other => panic!("expected the bubble rocking, got {other:?}"),
    }
    // Under water (or a zero angle) there is no rocking at all.
    let mut e = boat_entity(&[]);
    e.boat.underwater = true;
    e.boat.bubble_angle = 30.0;
    assert_eq!(body_transform(&e, 0.0).len, 0);
}

/// `AbstractBoatModel.setupAnim`: the paddles' rotations are the animatePaddle pose on top of
/// their rest tilt (x/y from the animation, the rest z of PI/16 kept, the right paddle's
/// rest PI of y subtracted), keyed by the model's own part names.
#[test]
fn setup_anim_poses_the_paddles_by_name() {
    let mut e = boat_entity(&[meta(11, MetadataValue::Boolean(true))]);
    // Two ticks of sweeping the left paddle.
    tick(&mut e, &crate::item_entity::NoWorld, 0.0);
    tick(&mut e, &crate::item_entity::NoWorld, 0.0);
    let model = crate::models::humanoid(false); // any tree; parts are looked up by name
    let mut model = (*model).clone();
    model.root.children = vec![
        paddle_part("left_paddle", 0.0),
        paddle_part("right_paddle", std::f32::consts::PI),
    ];
    let pose = setup_anim(&e, &model, 0.5);
    let rotations = rotations_by_name(&pose);
    let time = rowing_time(true, e.boat.paddle_positions[0], 0.5);
    let (x, y) = animate_paddle(time, false);
    let left = rotations["left_paddle"];
    assert!((left.x - x).abs() < 1.0e-6 && (left.y - y).abs() < 1.0e-6 && left.z == 0.0);
    // The right paddle is up (its flag is clear), so its rowing time is 0; its y override is
    // the animatePaddle pose with the rest PI subtracted.
    let (x, y) = animate_paddle(0.0, true);
    let right = rotations["right_paddle"];
    assert!((right.x - x).abs() < 1.0e-6);
    assert!((right.y - (y - std::f32::consts::PI)).abs() < 1.0e-6);
    // A model without paddle parts poses nothing.
    let humanoid = crate::models::humanoid(false);
    let pose = setup_anim(&e, &humanoid, 0.5);
    assert_eq!(rotations_by_name(&pose).len(), 0);
}

/// `BoatModel`'s paddle parts as vanilla defines them (rest z PI/16, the right one's rest y
/// PI), built as children of any root.
fn paddle_part(name: &str, rest_y: f32) -> haste_core::model::ModelPart {
    haste_core::model::ModelPart {
        name: name.to_owned(),
        id: haste_core::parts::intern(name),
        pivot: haste_core::Vec3::ZERO,
        rotation: glam::Vec3::new(0.0, rest_y, std::f32::consts::PI / 16.0),
        visible: true,
        cubes: Vec::new(),
        children: Vec::new(),
    }
}

/// `BoatRenderer`'s table: every boat family type has a renderer (so a layer and a texture),
/// the chest boats use the `chest_boat` layers, and only bamboo is a raft.
#[test]
fn the_boat_table_matches_entity_renderers() {
    assert_eq!(
        entry_for_name("minecraft:oak_boat").unwrap().layer,
        "minecraft:boat/oak#main"
    );
    assert_eq!(
        entry_for_name("minecraft:oak_boat").unwrap().texture,
        "entity/boat/oak"
    );
    assert!(!entry_for_name("minecraft:oak_boat").unwrap().chest);
    assert!(!entry_for_name("minecraft:oak_boat").unwrap().raft);
    assert_eq!(
        entry_for_name("minecraft:cherry_chest_boat").unwrap().layer,
        "minecraft:chest_boat/cherry#main"
    );
    assert_eq!(
        entry_for_name("minecraft:cherry_chest_boat")
            .unwrap()
            .texture,
        "entity/chest_boat/cherry"
    );
    assert!(entry_for_name("minecraft:cherry_chest_boat").unwrap().chest);
    // Bamboo is the raft family (`RaftRenderer`).
    let raft = entry_for_name("minecraft:bamboo_raft").unwrap();
    assert!(raft.raft && !raft.chest);
    assert_eq!(raft.layer, "minecraft:boat/bamboo#main");
    assert_eq!(
        entry_for_name("minecraft:bamboo_chest_raft").unwrap().layer,
        "minecraft:chest_boat/bamboo#main"
    );
    // Every vanilla boat type is covered, and unknown types are not boats here.
    for name in [
        "minecraft:acacia_boat",
        "minecraft:acacia_chest_boat",
        "minecraft:bamboo_raft",
        "minecraft:bamboo_chest_raft",
        "minecraft:birch_boat",
        "minecraft:birch_chest_boat",
        "minecraft:cherry_boat",
        "minecraft:cherry_chest_boat",
        "minecraft:dark_oak_boat",
        "minecraft:dark_oak_chest_boat",
        "minecraft:jungle_boat",
        "minecraft:jungle_chest_boat",
        "minecraft:mangrove_boat",
        "minecraft:mangrove_chest_boat",
        "minecraft:oak_boat",
        "minecraft:oak_chest_boat",
        "minecraft:pale_oak_boat",
        "minecraft:pale_oak_chest_boat",
        "minecraft:poplar_boat",
        "minecraft:poplar_chest_boat",
        "minecraft:spruce_boat",
        "minecraft:spruce_chest_boat",
    ] {
        assert!(
            entry_for_name(name).is_some(),
            "{name} missing from the boat table"
        );
    }
    assert!(entry_for_name("minecraft:pig").is_none());
    assert!(entry_for_name("minecraft:speedboat_of_aven").is_none());
    // The family check the client steers by takes mods' boats too.
    assert!(is_boat_family("oak_boat") && is_boat_family("modded_chest_raft"));
    assert!(!is_boat_family("oak"));
}

/// `BoatRenderer.submitTypeAdditions`: the water patch overlay only for a floating boat
/// (never a raft's - `RaftRenderer` has none).
#[test]
fn the_water_patch_follows_the_raft_and_underwater_gates() {
    let floating = BoatState::default();
    let oak = entry_for_name("minecraft:oak_boat").unwrap();
    assert!(water_patch_overlay(oak, &floating).is_some());
    let underwater = BoatState {
        underwater: true,
        ..Default::default()
    };
    assert!(water_patch_overlay(oak, &underwater).is_none());
    let raft = entry_for_name("minecraft:bamboo_raft").unwrap();
    assert!(water_patch_overlay(raft, &floating).is_none());
    assert!(water_patch_overlay(raft, &underwater).is_none());
}

/// `BoatModel.createWaterPatch`: the patch plate's geometry (the hull's bottom box, pivoted
/// 3 units above it and laid flat).
#[test]
fn the_water_patch_is_the_hull_sized_plate() {
    let patch = water_patch_model();
    let part = find_part(&patch.root, "water_patch").unwrap();
    assert_eq!(part.pivot, glam::Vec3::new(0.0, -3.0, 1.0));
    assert!((part.rotation.x - std::f32::consts::FRAC_PI_2).abs() < 1.0e-6);
    let cube = &part.cubes[0];
    assert_eq!(cube.origin, glam::Vec3::new(-14.0, -9.0, -3.0));
    assert_eq!(cube.size, glam::Vec3::new(28.0, 16.0, 3.0));
}

/// The boat models from the recorder dump: vanilla's `BoatModel`/`RaftModel` geometry
/// (pivots as baked - `AbstractBoatRenderer` renders from its own origin, so the dump's
/// ground shift must not apply), the trailing 90-degree yaw baked into the root, and the
/// textures the renderers build from the layer paths.
#[test]
#[ignore = "needs cache/oracle/26.3/entity_models.json (recorder mod dump; HASTE_CACHE overrides)"]
fn boat_models_from_the_dump() {
    let Some(oak) = model(entry_for_name("minecraft:oak_boat").unwrap()) else {
        eprintln!("skipped: no entity model dump in the cache");
        return;
    };
    assert_eq!(oak.texture.path(), "entity/boat/oak");
    // `AbstractBoatRenderer.submit`'s trailing `rotateDegrees(Axis.YP, 90)`.
    assert!((oak.root.rotation.y - std::f32::consts::FRAC_PI_2).abs() < 1.0e-6);
    // `BoatModel.createBoatModel`: five hull parts and the two paddles, at vanilla's pivots
    // (model units, unshifted - the boat renders from its own origin, not the ground plane).
    let bottom = find_part(&oak.root, "bottom").unwrap();
    assert_eq!(bottom.pivot, glam::Vec3::new(0.0, 3.0, 1.0));
    assert!((bottom.rotation.x - std::f32::consts::FRAC_PI_2).abs() < 1.0e-6);
    assert_eq!(bottom.cubes[0].origin, glam::Vec3::new(-14.0, -9.0, -3.0));
    assert_eq!(bottom.cubes[0].size, glam::Vec3::new(28.0, 16.0, 3.0));
    assert_eq!(
        find_part(&oak.root, "back").unwrap().pivot,
        glam::Vec3::new(-15.0, 4.0, 4.0)
    );
    assert_eq!(
        find_part(&oak.root, "front").unwrap().pivot,
        glam::Vec3::new(15.0, 4.0, 0.0)
    );
    assert_eq!(
        find_part(&oak.root, "right").unwrap().pivot,
        glam::Vec3::new(0.0, 4.0, -9.0)
    );
    assert_eq!(
        find_part(&oak.root, "left").unwrap().pivot,
        glam::Vec3::new(0.0, 4.0, 9.0)
    );
    let left = find_part(&oak.root, "left_paddle").unwrap();
    assert_eq!(left.pivot, glam::Vec3::new(3.0, -5.0, 9.0));
    assert!((left.rotation.z - std::f32::consts::PI / 16.0).abs() < 1.0e-6);
    let right = find_part(&oak.root, "right_paddle").unwrap();
    assert_eq!(right.pivot, glam::Vec3::new(3.0, -5.0, -9.0));
    assert!((right.rotation.y - std::f32::consts::PI).abs() < 1.0e-6);
    // The layer's 128x64 texture layout.
    assert_eq!((oak.tex_width, oak.tex_height), (128, 64));

    // `BoatModel.createChestBoatModel`: the chest parts on top, a 128x128 layout.
    let chest = model(entry_for_name("minecraft:oak_chest_boat").unwrap()).unwrap();
    assert_eq!(chest.texture.path(), "entity/chest_boat/oak");
    assert_eq!((chest.tex_width, chest.tex_height), (128, 128));
    for name in ["chest_bottom", "chest_lid", "chest_lock"] {
        assert!(find_part(&chest.root, name).is_some(), "{name} missing");
    }
    assert_eq!(
        find_part(&chest.root, "chest_bottom").unwrap().pivot,
        glam::Vec3::new(-2.0, -5.0, -6.0)
    );

    // `RaftModel.createRaftModel`: bamboo's flat hull (two stacked boxes) with its own
    // paddle pivots.
    let raft = model(entry_for_name("minecraft:bamboo_raft").unwrap()).unwrap();
    assert_eq!(raft.texture.path(), "entity/boat/bamboo");
    let raft_bottom = find_part(&raft.root, "bottom").unwrap();
    assert_eq!(raft_bottom.pivot, glam::Vec3::new(0.0, -2.1, 1.0));
    assert_eq!(raft_bottom.cubes.len(), 2);
    let left = find_part(&raft.root, "left_paddle").unwrap();
    assert_eq!(left.pivot, glam::Vec3::new(3.0, -4.0, 9.0));
}

/// Every boat texture exists in the client jar as `assets/minecraft/textures/<path>.png`
/// (the renderers build the path from the layer's model path).
#[test]
#[ignore = "needs the client jar and assets index (HASTE_CACHE; from `haste setup`)"]
fn boat_textures_exist_in_jar() {
    let jar = haste_core::paths::client_jar();
    let file = std::fs::File::open(&jar).unwrap_or_else(|e| {
        panic!(
            "opening {}: {e} (run `haste setup` or set HASTE_CACHE)",
            jar.display()
        )
    });
    let zip = zip::ZipArchive::new(file).expect("reading client jar zip");
    let names: std::collections::HashSet<String> =
        zip.file_names().map(|s| s.to_string()).collect();
    for (_, entry) in crate::boat::all_entries() {
        let key = format!("assets/minecraft/textures/{}.png", entry.texture);
        assert!(names.contains(&key), "{key} missing from the jar");
    }
}

/// The store's instance for a boat: the dumped model with the water patch overlay, the
/// paddle pose, the lift and the wobble in the body transform, and no head aim. Needs the
/// dump and the registries (the kind resolves through the real entity-type names).
#[test]
#[ignore = "needs cache/oracle/26.3 (recorder mod dump + registries.json; HASTE_CACHE overrides)"]
fn render_instances_builds_a_boat_instance() {
    let Some(names) = crate::registry::entity_type_names() else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    let Some((kind, _)) = names.iter().find(|(_, n)| **n == "minecraft:oak_boat") else {
        eprintln!("skipped: minecraft:oak_boat not in the registry");
        return;
    };
    let mut s = store();
    let mut sp = spawn(7, DVec3::new(0.5, 64.0, 0.5));
    sp.kind = *kind;
    sp.yaw = 30.0;
    s.spawn(&sp);
    s.set_metadata(
        EntityId(7),
        &[
            meta(8, MetadataValue::VarInt(5)),
            meta(9, MetadataValue::VarInt(1)),
            meta(10, MetadataValue::Float(20.0)),
            meta(11, MetadataValue::Boolean(true)),
        ],
    );
    let mut out = Vec::new();
    s.tick();
    s.render_instances(0.5, &mut out);
    if out.is_empty() || out[0].model.texture.path() != "entity/boat/oak" {
        eprintln!("skipped: no entity model dump in the cache");
        return;
    }
    let inst = &out[0];
    // The water patch rides as the first overlay.
    let patch = inst.overlays[0].as_ref().expect("the water patch overlay");
    assert_eq!(patch.model.texture.path(), "boat/water_patch");
    // The whole-model transform: the lift, then the hurt wobble.
    assert_eq!(inst.body.pre_translate, glam::Vec3::new(0.0, 0.375, 0.0));
    let expected = 4.5_f32.sin() * 4.5 * 19.5 / 10.0;
    assert!(inst.body.len >= 1);
    match inst.body.iter().next() {
        Some(haste_core::model::BodyOp::RotateX(a)) => {
            assert!((a - expected).abs() < 1.0e-4, "{a} vs {expected}")
        }
        other => panic!("expected the hurt wobble, got {other:?}"),
    }
    // Both paddles are posed (vanilla's animatePaddle runs for each side; the left one with
    // the swept rowing time, the right one - its flag is clear - with time 0).
    let rotations = rotations_by_name(&inst.pose);
    assert!(rotations.contains_key("left_paddle"));
    assert!(rotations.contains_key("right_paddle"));
    // The hull follows the entity yaw, not a body yaw (no head part).
    assert!((inst.yaw - 30.0).abs() < 1.0e-6);
}
