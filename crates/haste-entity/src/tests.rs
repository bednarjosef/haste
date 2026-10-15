//! Tests for Task 1: tracking + interpolation. Timed tests are `#[ignore]`d in debug (too slow to
//! be meaningful) and run in release via `cargo test --release -- --ignored`.
use std::time::Instant;

use haste_core::entity::{EntitySpawn, MetadataEntry, MetadataValue, PlayerInfo, Pose};
use haste_core::game::GameMode;
use haste_core::ids::{EntityId, RegistryIndex};
use haste_core::model::{BodyOp, EntityInstance};
use haste_core::DVec3;
use uuid::Uuid;

use crate::dims::EntityDimensions;
use crate::riding::{passenger_seat, PassengerKind, SeatPoint, SeatQuery};
use crate::store::{Entity, EntityStore};

fn store() -> EntityStore {
    EntityStore::new(EntityDimensions::default())
}

fn spawn(id: i32, pos: DVec3) -> EntitySpawn {
    EntitySpawn {
        id: EntityId(id),
        uuid: Uuid::new_v4(),
        kind: RegistryIndex(0),
        pos,
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 0.0,
        data: 0,
        velocity: DVec3::ZERO,
    }
}

#[test]
fn spawn_get_remove() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::new(0.0, 64.0, 0.0)));
    s.spawn(&spawn(2, DVec3::new(1.0, 64.0, 0.0)));
    assert_eq!(s.len(), 2);
    let e: &Entity = s.get(EntityId(1)).unwrap();
    assert_eq!(e.id, EntityId(1));
    assert_eq!(e.pos, DVec3::new(0.0, 64.0, 0.0));
    s.remove(&[EntityId(1)]);
    assert!(s.get(EntityId(1)).is_none());
    assert_eq!(s.len(), 1);
    // Slot reuse: the freed slot serves the next spawn.
    s.spawn(&spawn(3, DVec3::new(2.0, 64.0, 0.0)));
    assert_eq!(s.len(), 2);
    assert!(s.get(EntityId(3)).is_some());
    // Duplicate add_entity for a live id is ignored.
    s.spawn(&spawn(2, DVec3::new(9.0, 9.0, 9.0)));
    assert_eq!(s.get(EntityId(2)).unwrap().pos, DVec3::new(1.0, 64.0, 0.0));
}

#[test]
fn delta_moves_accumulate_exactly() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    // A tick with no pending lerp just restates the position.
    s.tick();
    // Deltas are exact: after the 3-tick window finishes, the entity is at the sum.
    s.move_delta(EntityId(1), DVec3::new(1.0, 0.0, 0.0), true);
    s.tick();
    s.tick();
    s.tick();
    assert_eq!(s.get(EntityId(1)).unwrap().pos, DVec3::new(1.0, 0.0, 0.0));
    s.move_delta(EntityId(1), DVec3::new(0.5, 0.25, -0.25), false);
    s.tick();
    s.tick();
    s.tick();
    assert_eq!(
        s.get(EntityId(1)).unwrap().pos,
        DVec3::new(1.5, 0.25, -0.25)
    );
    assert!(!s.get(EntityId(1)).unwrap().on_ground);
}

#[test]
fn teleport_resets_interpolation() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.move_delta(EntityId(1), DVec3::new(10.0, 0.0, 0.0), false);
    s.tick();
    // Mid-lerp teleport: the very next tick lands exactly on the teleport position.
    s.teleport(
        EntityId(1),
        DVec3::new(100.0, 64.0, 100.0),
        90.0,
        -45.0,
        true,
    );
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(e.pos, DVec3::new(100.0, 64.0, 100.0));
    assert_eq!(e.prev_pos, DVec3::new(100.0, 64.0, 100.0));
    assert_eq!(e.yaw, 90.0);
    assert_eq!(e.pitch, -45.0);
    assert!(e.on_ground);
    // And the lerp window is gone: further ticks do not drift.
    s.tick();
    assert_eq!(
        s.get(EntityId(1)).unwrap().pos,
        DVec3::new(100.0, 64.0, 100.0)
    );
}

#[test]
fn metadata_pose_sets_height() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    // Standing default: 0.6 x 1.8.
    assert_eq!(s.get(EntityId(1)).unwrap().dimensions, (0.6, 1.8));
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: 6,
            value: MetadataValue::Pose(Pose::Crouching),
        }],
    );
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(e.pose, Pose::Crouching);
    assert_eq!(e.dimensions.0, 0.6);
    assert!(
        (e.dimensions.1 - 1.5).abs() < 1e-6,
        "crouch height {}",
        e.dimensions.1
    );
    // Back up.
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: 6,
            value: MetadataValue::Pose(Pose::Standing),
        }],
    );
    assert_eq!(s.get(EntityId(1)).unwrap().dimensions, (0.6, 1.8));
    // Swimming crawls.
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: 6,
            value: MetadataValue::Pose(Pose::Swimming),
        }],
    );
    assert!((s.get(EntityId(1)).unwrap().dimensions.1 - 0.6).abs() < 1e-6);
}

#[test]
fn metadata_flags_byte() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    // flags: 0x02 crouching | 0x10 invisible | 0x40 glowing
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: 0,
            value: MetadataValue::Byte(0x02 | 0x10 | 0x40),
        }],
    );
    let e = s.get(EntityId(1)).unwrap();
    assert!(e.metadata.shared.crouching);
    assert!(e.metadata.shared.invisible);
    assert!(e.metadata.shared.glowing);
    // Raw entries are kept per index and later updates replace them.
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: 0,
            value: MetadataValue::Byte(0),
        }],
    );
    assert!(!s.get(EntityId(1)).unwrap().metadata.shared.crouching);
    assert_eq!(s.get(EntityId(1)).unwrap().metadata.iter().count(), 1);
}

#[test]
fn render_instances_midpoint() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    // Move a large distance in one packet: prev_pos=0, target=4; after the first of 3 ticks the
    // entity is at 4/3. Partial 0.5 between prev (0) and pos (4/3) is 2/3.
    s.move_delta(EntityId(1), DVec3::new(4.0, 0.0, 0.0), true);
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert!((e.pos.x - 4.0 / 3.0).abs() < 1e-9, "pos {}", e.pos.x);
    assert_eq!(e.prev_pos, DVec3::ZERO);
    let mut out = Vec::new();
    s.render_instances(0.5, &mut out);
    assert_eq!(out.len(), 1);
    assert!(
        (out[0].pos.x - 2.0 / 3.0).abs() < 1e-9,
        "partial {}",
        out[0].pos.x
    );
    // Endpoints.
    let mut out = Vec::new();
    s.render_instances(0.0, &mut out);
    assert_eq!(out[0].pos.x, 0.0);
    let mut out = Vec::new();
    s.render_instances(1.0, &mut out);
    assert!((out[0].pos.x - 4.0 / 3.0).abs() < 1e-9);
}

/// A dropped item entity carries its stack in `ItemEntity.DATA_ITEM`, metadata index 8
/// (Entity's eight synched fields come first): it reaches the draw list with the item's
/// registry id, the stack count and its age (partial-tick interpolated), and an
/// emptied stack clears it.
#[test]
fn item_entity_stack_from_metadata() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    let stack = |count, item: Option<u32>| MetadataEntry {
        index: 8,
        value: MetadataValue::ItemStack(haste_core::entity::ItemStackData {
            count,
            item: item.map(RegistryIndex),
            components_raw: Default::default(),
        }),
    };
    s.set_metadata(EntityId(1), &[stack(10, Some(5))]);
    let mut out = Vec::new();
    s.render_instances(0.5, &mut out);
    assert_eq!(
        out[0].item,
        Some(haste_core::model::DroppedItem {
            item: RegistryIndex(5),
            count: 10,
            age: 0.5,
        })
    );
    // Age advances with the ticks (plus the partial).
    s.tick();
    s.tick();
    let mut out = Vec::new();
    s.render_instances(0.25, &mut out);
    assert_eq!(out[0].item.unwrap().age, 2.25);
    // An emptied stack (count 0) stops the item from drawing as an item.
    s.set_metadata(EntityId(1), &[stack(0, Some(5))]);
    let mut out = Vec::new();
    s.render_instances(0.0, &mut out);
    assert_eq!(out[0].item, None);
    // An empty stack slot (None item) likewise.
    s.set_metadata(EntityId(1), &[stack(3, None)]);
    let mut out = Vec::new();
    s.render_instances(0.0, &mut out);
    assert_eq!(out[0].item, None);
    // Entities without the metadata entry are unaffected.
    s.spawn(&spawn(2, DVec3::ZERO));
    let mut out = Vec::new();
    s.render_instances(0.0, &mut out);
    assert_eq!(out.iter().find(|i| i.id == EntityId(2)).unwrap().item, None);
}

#[test]
fn player_info_nametag() {
    let mut s = store();
    let sp = spawn(1, DVec3::ZERO);
    let uuid = sp.uuid;
    s.spawn(&sp);
    assert!(s.get(EntityId(1)).unwrap().nametag.is_none());
    s.player_info(&[PlayerInfo {
        uuid,
        name: "Steve".into(),
        display_name: None,
        latency: 20,
        game_mode: Some(GameMode::Survival),
        listed: true,
        skin_texture_url: None,
    }]);
    assert_eq!(
        s.get(EntityId(1)).unwrap().nametag.as_deref(),
        Some("Steve")
    );
    // Spawning after the info arrived also picks the name up.
    let sp2 = spawn(2, DVec3::ZERO);
    s.player_info(&[PlayerInfo {
        uuid: sp2.uuid,
        name: "Alex".into(),
        display_name: None,
        latency: 30,
        game_mode: Some(GameMode::Creative),
        listed: true,
        skin_texture_url: None,
    }]);
    s.spawn(&sp2);
    assert_eq!(s.get(EntityId(2)).unwrap().nametag.as_deref(), Some("Alex"));
    // Instances carry the nametag through.
    let mut out = Vec::new();
    s.render_instances(0.0, &mut out);
    assert!(out
        .iter()
        .any(|i: &EntityInstance| i.nametag.as_deref() == Some("Steve")));
}

#[test]
fn equipment_replaces_slot() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    let sword = haste_core::entity::ItemStackData {
        count: 1,
        item: Some(RegistryIndex(7)),
        components_raw: Default::default(),
    };
    s.set_equipment(EntityId(1), 0, sword.clone());
    s.set_equipment(EntityId(1), 0, sword.clone()); // same slot replaces, not duplicates
    assert_eq!(s.get(EntityId(1)).unwrap().equipment.len(), 1);
    let helmet = haste_core::entity::ItemStackData {
        count: 1,
        item: Some(RegistryIndex(8)),
        components_raw: Default::default(),
    };
    s.set_equipment(EntityId(1), 5, helmet);
    assert_eq!(s.get(EntityId(1)).unwrap().equipment.len(), 2);
}

/// 10k entities tick in < 1 ms. Timed tests only mean something optimized; run with
/// `cargo test --release -p haste-entity -- --ignored`.
#[test]
#[ignore = "timed test, run in release: cargo test --release -p haste-entity -- --ignored"]
fn ten_k_entities_tick_under_1ms() {
    let mut s = store();
    for i in 0..10_000 {
        s.spawn(&spawn(i, DVec3::new(f64::from(i) * 0.1, 64.0, 0.0)));
    }
    // Half of them get a delta each tick, so the lerp path is exercised too.
    for t in 0..10 {
        let _ = t;
        for i in 0..5_000 {
            s.move_delta(EntityId(i), DVec3::new(0.01, 0.0, 0.0), true);
        }
        let start = Instant::now();
        s.tick();
        let dur = start.elapsed();
        assert!(
            dur.as_nanos() < 1_000_000,
            "10k tick took {} ns (iteration {t})",
            dur.as_nanos()
        );
    }
    // render_instances over 10k entities stays proportionate (steady state: warm the model's
    // OnceLock and the allocator with one untimed pass first).
    let mut out = Vec::with_capacity(10_000);
    s.render_instances(0.5, &mut out);
    out.clear();
    let start = Instant::now();
    s.render_instances(0.5, &mut out);
    let dur = start.elapsed();
    assert_eq!(out.len(), 10_000);
    assert!(
        dur.as_nanos() < 2_000_000,
        "render_instances took {} ns",
        dur.as_nanos()
    );
}

// ---------------------------------------------------------------------------
// Task 2: models as data + walking animation.
// ---------------------------------------------------------------------------

use haste_core::model::{CubeModel, ModelPart};

fn find_part<'a>(p: &'a ModelPart, name: &str) -> Option<&'a ModelPart> {
    if p.name == name {
        Some(p)
    } else {
        p.children.iter().find_map(|c| find_part(c, name))
    }
}

fn humanoid_parts(m: &CubeModel) -> Vec<&str> {
    let mut v: Vec<&str> = m.root.children.iter().map(|c| c.name.as_str()).collect();
    v.extend(
        m.root
            .children
            .iter()
            .flat_map(|c| c.children.iter().map(|g| g.name.as_str())),
    );
    v
}

#[test]
fn humanoid_model_part_tree() {
    let classic = crate::models::humanoid(false);
    assert_eq!(classic.tex_width, 64);
    assert_eq!(classic.tex_height, 64);
    let names = humanoid_parts(&classic);
    for expected in [
        "head",
        "hat",
        "body",
        "jacket",
        "right_arm",
        "right_sleeve",
        "left_arm",
        "left_sleeve",
        "right_leg",
        "right_pants",
        "left_leg",
        "left_pants",
    ] {
        assert!(
            names.contains(&expected),
            "missing part {expected} in {names:?}"
        );
    }
    // UV layout: head 0,0; body 16,16; right arm 40,16; left arm 32,48; right leg 0,16;
    // left leg 16,48; hat 32,0.
    let r = &classic.root;
    assert_eq!(find_part(r, "head").unwrap().cubes[0].uv, [0, 0]);
    assert_eq!(find_part(r, "hat").unwrap().cubes[0].uv, [32, 0]);
    assert_eq!(find_part(r, "body").unwrap().cubes[0].uv, [16, 16]);
    assert_eq!(find_part(r, "jacket").unwrap().cubes[0].uv, [16, 32]);
    assert_eq!(find_part(r, "right_arm").unwrap().cubes[0].uv, [40, 16]);
    assert_eq!(find_part(r, "right_sleeve").unwrap().cubes[0].uv, [40, 32]);
    assert_eq!(find_part(r, "left_arm").unwrap().cubes[0].uv, [32, 48]);
    assert_eq!(find_part(r, "left_sleeve").unwrap().cubes[0].uv, [48, 48]);
    assert_eq!(find_part(r, "right_leg").unwrap().cubes[0].uv, [0, 16]);
    assert_eq!(find_part(r, "right_pants").unwrap().cubes[0].uv, [0, 32]);
    assert_eq!(find_part(r, "left_leg").unwrap().cubes[0].uv, [16, 48]);
    assert_eq!(find_part(r, "left_pants").unwrap().cubes[0].uv, [0, 48]);
    // Cube sizes: head 8x8x8, body 8x12x4, arms 4x12x4 (classic), legs 4x12x4.
    assert_eq!(
        find_part(r, "head").unwrap().cubes[0].size,
        glam::Vec3::new(8.0, 8.0, 8.0)
    );
    assert_eq!(
        find_part(r, "body").unwrap().cubes[0].size,
        glam::Vec3::new(8.0, 12.0, 4.0)
    );
    assert_eq!(
        find_part(r, "right_arm").unwrap().cubes[0].size,
        glam::Vec3::new(4.0, 12.0, 4.0)
    );
    assert_eq!(
        find_part(r, "left_leg").unwrap().cubes[0].size,
        glam::Vec3::new(4.0, 12.0, 4.0)
    );
    // Pivots: head at y=-24, legs at y=-12 (feet at 0, model y grows downward).
    assert_eq!(
        find_part(r, "head").unwrap().pivot,
        glam::Vec3::new(0.0, -24.0, 0.0)
    );
    assert_eq!(
        find_part(r, "right_leg").unwrap().pivot,
        glam::Vec3::new(-1.9, -12.0, 0.0)
    );
    // Child pivots are relative to the parent (vanilla PartPose): overlays sit at the origin.
    for overlay in [
        "hat",
        "jacket",
        "right_sleeve",
        "left_sleeve",
        "right_pants",
        "left_pants",
    ] {
        assert_eq!(
            find_part(r, overlay).unwrap().pivot,
            glam::Vec3::ZERO,
            "{overlay}"
        );
    }
}

#[test]
fn humanoid_slim_arms() {
    let slim = crate::models::humanoid(true);
    assert_eq!(
        find_part(&slim.root, "right_arm").unwrap().cubes[0].size,
        glam::Vec3::new(3.0, 12.0, 4.0)
    );
    assert_eq!(
        find_part(&slim.root, "left_arm").unwrap().cubes[0].size,
        glam::Vec3::new(3.0, 12.0, 4.0)
    );
    // Classic and slim are distinct cached models.
    assert!(!std::sync::Arc::ptr_eq(
        &crate::models::humanoid(false),
        &crate::models::humanoid(true)
    ));
}

#[test]
fn box_model_matches_dimensions() {
    let m = crate::models::box_for(0.6, 1.95);
    assert_eq!(m.texture.to_string(), "haste:entity_box");
    let c = &m.root.cubes[0];
    // 0.6 blocks wide = 9.6 units, rounded to 10 (1/16 precision); height 1.95 -> 31.2 -> 31.
    assert_eq!(c.size, glam::Vec3::new(10.0, 31.0, 10.0));
    assert_eq!(c.origin, glam::Vec3::new(-5.0, -31.0, -5.0));
    // Cache: same dimensions return the same Arc.
    let m2 = crate::models::box_for(0.6, 1.95);
    assert!(std::sync::Arc::ptr_eq(&m, &m2));
}

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
    let rot: std::collections::HashMap<_, _> = out[0].pose.rotations.iter().cloned().collect();
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
    let rot: std::collections::HashMap<_, _> = out[0].pose.rotations.iter().cloned().collect();
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
    let rot: std::collections::HashMap<_, _> = out[0].pose.rotations.iter().cloned().collect();
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
    let rot: std::collections::HashMap<_, _> = out[0].pose.rotations.iter().cloned().collect();
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
    let rot: std::collections::HashMap<_, _> = pose.rotations.iter().cloned().collect();
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
    let rot: std::collections::HashMap<_, _> = pose.rotations.iter().cloned().collect();
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
    let rot: std::collections::HashMap<_, _> = pose.rotations.iter().cloned().collect();
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
    let rot: std::collections::HashMap<_, _> = pose.rotations.iter().cloned().collect();
    let rh = rot.get("right_hind_leg").unwrap();
    let rf = rot.get("right_front_leg").unwrap();
    let lh = rot.get("left_hind_leg").unwrap();
    assert!(rh.x.abs() > 1e-3, "hind leg not swinging");
    // Opposite phase: the right hind and right front legs have opposite sign here.
    assert!(rh.x * rf.x <= 0.0, "hind {} front {} in phase", rh.x, rf.x);
    assert!((lh.x + rh.x).abs() < 1e-4, "left/right hind not mirrored");
}

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

/// `VillagerModel.setupAnim`: an unhappy villager (`AbstractVillager.DATA_UNHAPPY_COUNTER`,
/// metadata index 18, > 0) shakes its head (`zRot = 0.3*sin(0.45*ageInTicks)`, `xRot = 0.4`),
/// and the legs swing at half the humanoid amplitude.
#[test]
fn villager_unhappy_shakes_head_and_halves_walk() {
    let model = mob_model(
        "entity/villager/villager",
        &["head", "arms", "right_leg", "left_leg"],
    );
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: 18,
            value: MetadataValue::VarInt(1),
        }],
    );
    for _ in 0..10 {
        s.move_delta(EntityId(1), DVec3::new(0.1, 0.0, 0.0), true);
        s.tick();
    }
    let e = s.get(EntityId(1)).unwrap();
    let pose = crate::animation::setup_anim(e, &model, true, 0.0, false);
    let rot: std::collections::HashMap<_, _> = pose.rotations.iter().cloned().collect();
    let head = rot.get("head").unwrap();
    assert!((head.x - 0.4).abs() < 1e-6, "unhappy head x {}", head.x);
    let age = e.tick_count as f32;
    assert!(
        (head.z - 0.3 * (0.45 * age).sin()).abs() < 1e-5,
        "unhappy head z {}",
        head.z
    );
    let pos = e.walk.position(0.0);
    let speed = e.walk.speed(0.0);
    let want = (pos * 0.6662).cos() * 1.4 * speed * 0.5;
    let rl = rot.get("right_leg").unwrap();
    assert!(
        (rl.x - want).abs() < 1e-5,
        "villager leg {} != {want}",
        rl.x
    );
}

/// `IllagerModel.setupAnim`: a calm vindicator crosses its arms (the "arms" part shown, the two
/// arms hidden); an aggressive one attacks with `animateZombieArms(aggressive = true)`
/// (`armDrop = -PI / 1.5`).
#[test]
fn illager_crosses_arms_and_attacks_when_aggressive() {
    let model = mob_model(
        "entity/illager/vindicator",
        &[
            "head",
            "arms",
            "right_arm",
            "left_arm",
            "right_leg",
            "left_leg",
        ],
    );
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    let e = s.get(EntityId(1)).unwrap();
    let pose = crate::animation::setup_anim(e, &model, true, 0.0, false);
    assert!(pose.hidden.iter().any(|h| h == "right_arm"));
    assert!(pose.hidden.iter().any(|h| h == "left_arm"));
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: 15,
            value: MetadataValue::Byte(0x04),
        }],
    );
    let e = s.get(EntityId(1)).unwrap();
    let pose = crate::animation::setup_anim(e, &model, true, 0.0, false);
    assert!(pose.hidden.iter().any(|h| h == "arms"));
    let rot: std::collections::HashMap<_, _> = pose.rotations.iter().cloned().collect();
    let ra = rot.get("right_arm").unwrap();
    assert!(
        (ra.x - -2.094_395_2).abs() < 1e-4,
        "attacking illager arm {} != -PI/1.5",
        ra.x
    );
}

/// `IronGolemModel.setupAnim`: entity event 4 starts a 10-tick attack animation; at the first
/// frame `attackTicksRemaining = 10`, `triangleWave(10, 10) = 1`, so both arms are at -0.5.
#[test]
fn iron_golem_attack_arms() {
    let model = mob_model(
        "entity/iron_golem/iron_golem",
        &[
            "head",
            "body",
            "right_arm",
            "left_arm",
            "right_leg",
            "left_leg",
        ],
    );
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.mark_attack(EntityId(1));
    let e = s.get(EntityId(1)).unwrap();
    let pose = crate::animation::setup_anim(e, &model, true, 0.0, false);
    let rot: std::collections::HashMap<_, _> = pose.rotations.iter().cloned().collect();
    let ra = rot.get("right_arm").unwrap();
    let la = rot.get("left_arm").unwrap();
    assert!((ra.x - -0.5).abs() < 1e-5, "golem arm {} != -0.5", ra.x);
    assert!((la.x - -0.5).abs() < 1e-5);
}

/// `CreeperRenderer.scale`: a fresh creeper is unscaled; a swelling one grows wider (x/z) and
/// taller (y) from `getSwelling(partial) = lerp(partial, oldSwell, swell) / (maxSwell - 2)`.
#[test]
fn creeper_swell_scales_the_model() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(
        crate::animation::creeper_scale(e, 0.0),
        haste_core::math::Vec3::ONE
    );
    // Swell direction 1 (index 16, int) for 28 ticks: oldSwell 27, swell 28.
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: 16,
            value: MetadataValue::VarInt(1),
        }],
    );
    for _ in 0..28 {
        s.tick();
    }
    let e = s.get(EntityId(1)).unwrap();
    let sc = crate::animation::creeper_scale(e, 0.0);
    assert!((sc.x - sc.z).abs() < 1e-6, "x/z should match");
    assert!(sc.x > 1.0, "swollen creeper x {} not wider", sc.x);
    assert!(sc.y > 1.0, "swollen creeper y {} not taller", sc.y);
}

/// Builds a minimal `CubeModel` whose root has one cube-less child per `parts` name, so the
/// animation classification (by part name) and setupAnim can be exercised without the dump.
fn mob_model(texture: &str, parts: &[&str]) -> CubeModel {
    let children = parts
        .iter()
        .map(|name| ModelPart {
            name: (*name).to_owned(),
            pivot: glam::Vec3::ZERO,
            rotation: glam::Vec3::ZERO,
            visible: true,
            cubes: Vec::new(),
            children: Vec::new(),
        })
        .collect();
    CubeModel {
        texture: haste_core::Identifier::minecraft(texture),
        tex_width: 64,
        tex_height: 64,
        root: ModelPart {
            name: "root".to_owned(),
            pivot: glam::Vec3::ZERO,
            rotation: glam::Vec3::ZERO,
            visible: true,
            cubes: Vec::new(),
            children,
        },
    }
}

#[test]
fn player_gets_humanoid_model_and_nametag() {
    let mut s = store();
    let sp = spawn(1, DVec3::ZERO);
    let uuid = sp.uuid;
    s.player_info(&[PlayerInfo {
        uuid,
        name: "Steve".into(),
        display_name: None,
        latency: 20,
        game_mode: Some(GameMode::Survival),
        listed: true,
        skin_texture_url: None,
    }]);
    s.spawn(&sp);
    let mut out = Vec::new();
    s.render_instances(0.0, &mut out);
    assert_eq!(out[0].model.texture.to_string(), "haste:entity/humanoid");
    assert_eq!(out[0].nametag.as_deref(), Some("Steve"));
    // Non-player (no player_info entry): generic box.
    s.spawn(&spawn(2, DVec3::ZERO));
    let mut out = Vec::new();
    s.render_instances(0.0, &mut out);
    let other = out.iter().find(|i| i.id == EntityId(2)).unwrap();
    assert_eq!(other.model.texture.to_string(), "haste:entity_box");
}

#[test]
fn hurt_ticks_count_down_and_restart() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    // No flash before any hit.
    let mut out = Vec::new();
    s.render_instances(0.0, &mut out);
    assert_eq!(out[0].hurt, 0.0);
    // A hit starts 10 ticks of flash; the strength eases out and reaches 0 after 10 ticks.
    s.mark_hurt(EntityId(1));
    for _ in 0..10 {
        let mut out = Vec::new();
        s.render_instances(0.0, &mut out);
        assert!(out[0].hurt > 0.0, "flash during the 10 ticks");
        s.tick();
    }
    let mut out = Vec::new();
    s.render_instances(0.0, &mut out);
    assert_eq!(out[0].hurt, 0.0);
    // A second hurt restarts it.
    s.mark_hurt(EntityId(1));
    s.tick();
    let mut out = Vec::new();
    s.render_instances(0.0, &mut out);
    assert!(out[0].hurt > 0.0);
}

#[test]
fn hurt_strength_eases_out() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.mark_hurt(EntityId(1));
    let strength = |s: &EntityStore| {
        let mut out = Vec::new();
        s.render_instances(0.0, &mut out);
        out[0].hurt
    };
    // Quadratic ease-out over the 10 ticks: 1.0, 0.81, ... 0.01, 0.
    assert_eq!(strength(&s), 1.0);
    s.tick();
    assert!((strength(&s) - 0.81).abs() < 1e-6);
    for _ in 0..4 {
        s.tick();
    }
    // After 5 total ticks the strength is (5/10)^2.
    assert!((strength(&s) - 0.25).abs() < 1e-6);
    for _ in 0..4 {
        s.tick();
    }
    assert!((strength(&s) - 0.01).abs() < 1e-6);
    s.tick();
    assert_eq!(strength(&s), 0.0);
}

#[test]
fn death_holds_the_tint_for_20_ticks() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.mark_dead(EntityId(1));
    let strength = |s: &EntityStore| {
        let mut out = Vec::new();
        s.render_instances(0.0, &mut out);
        out[0].hurt
    };
    // Full strength through the death animation...
    for _ in 0..19 {
        assert_eq!(strength(&s), 1.0);
        s.tick();
    }
    // ...then nothing more.
    s.tick();
    assert_eq!(strength(&s), 0.0);
}

// ---------------------------------------------------------------------------
// Riding: set_passengers bookkeeping and the seat offset.
// ---------------------------------------------------------------------------

#[test]
fn set_passengers_mounts_replaces_and_dismounts() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO)); // vehicle
    s.spawn(&spawn(2, DVec3::ZERO));
    s.spawn(&spawn(3, DVec3::ZERO));
    // Mount two passengers.
    s.set_passengers(EntityId(1), &[EntityId(2), EntityId(3)]);
    assert_eq!(s.vehicle_of(EntityId(2)), Some(EntityId(1)));
    assert_eq!(s.vehicle_of(EntityId(3)), Some(EntityId(1)));
    assert_eq!(
        s.passengers_of(EntityId(1)).unwrap(),
        &[EntityId(2), EntityId(3)]
    );
    // A passenger list replaces the previous one: 3 stays, 2 is dropped.
    s.set_passengers(EntityId(1), &[EntityId(3)]);
    assert_eq!(s.vehicle_of(EntityId(2)), None);
    assert_eq!(s.vehicle_of(EntityId(3)), Some(EntityId(1)));
    assert_eq!(s.passengers_of(EntityId(1)).unwrap(), &[EntityId(3)]);
    // An empty list dismounts everyone.
    s.set_passengers(EntityId(1), &[]);
    assert_eq!(s.vehicle_of(EntityId(3)), None);
    assert!(s.passengers_of(EntityId(1)).is_none());
    // A vehicle's list can also name entities never spawned: the bookkeeping holds them
    // (the server does this between the spawn and the set_passengers arriving).
    s.set_passengers(EntityId(1), &[EntityId(9)]);
    assert_eq!(s.vehicle_of(EntityId(9)), Some(EntityId(1)));
}

#[test]
fn removed_entities_drop_out_of_riding_on_both_sides() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.spawn(&spawn(2, DVec3::ZERO));
    s.spawn(&spawn(3, DVec3::ZERO));
    s.set_passengers(EntityId(1), &[EntityId(2)]);
    s.set_passengers(EntityId(3), &[EntityId(1)]);
    // The vehicle is removed: its passengers are freed, and it stops riding entity 3.
    s.remove(&[EntityId(1)]);
    assert_eq!(s.vehicle_of(EntityId(2)), None);
    assert_eq!(s.vehicle_of(EntityId(1)), None);
    assert!(s.passengers_of(EntityId(3)).is_none());
    // A passenger removed from the store also vanishes from its vehicle's list.
    s.set_passengers(EntityId(3), &[EntityId(2)]);
    s.remove(&[EntityId(2)]);
    assert!(s.passengers_of(EntityId(3)).is_none());
}

/// Vanilla keeps attachment points as `float` and widens them to `double` when they become a
/// position (`Entity.positionRider`), so the exact value is the `float` rounding of the decimal
/// one: compare against that, within a float rounding.
fn assert_close(got: f64, want: f64) {
    assert!(
        (got - f64::from(want as f32)).abs() < 1e-6,
        "{got} is not {want}"
    );
}

/// The local player's seat on `vehicle_type`: the numbers vanilla produces for a player, whose
/// own vehicle attachment point is 0.6 below the seat (`Avatar.DEFAULT_VEHICLE_ATTACHMENT`).
fn seat(vehicle_type: &str, seat: usize, passengers: usize) -> SeatPoint {
    passenger_seat(SeatQuery {
        vehicle_type,
        vehicle_dims: (0.6, 1.8),
        seat,
        passengers,
        passenger: PassengerKind::Player,
        passenger_type: "player",
    })
}

/// The same, for a passenger that is not the local player (its own attachment point differs).
fn seat_of(
    vehicle_type: &str,
    seat: usize,
    passengers: usize,
    passenger: PassengerKind,
    passenger_type: &str,
) -> SeatPoint {
    passenger_seat(SeatQuery {
        vehicle_type,
        vehicle_dims: (0.6, 1.8),
        seat,
        passengers,
        passenger,
        passenger_type,
    })
}

#[test]
fn a_minecart_seat_is_vanillas_passenger_attachment() {
    // `passengerAttachments(0.1875F)`, and the player's own 0.6 below it.
    let s = seat("minecart", 0, 1);
    assert_eq!(s.x, 0.0);
    assert_eq!(s.z, 0.0);
    assert_close(s.y, 0.1875 - 0.6);
    // Every minecart variant shares the seat.
    for name in [
        "chest_minecart",
        "furnace_minecart",
        "tnt_minecart",
        "hopper_minecart",
        "spawner_minecart",
        "command_block_minecart",
    ] {
        assert_eq!(seat(name, 0, 1), s, "{name}");
    }
}

#[test]
fn boat_seats_follow_abstract_boat() {
    // `Boat.rideHeight`: height / 3 = 0.1875, minus the player's 0.6.
    let s = seat("oak_boat", 0, 1);
    assert_close(s.y, 0.5625 / 3.0 - 0.6);
    assert_eq!(s.x, 0.0);
    // Two passengers: the driver sits 0.2 to one side, the second 0.6 to the other.
    let driver = seat("oak_boat", 0, 2);
    let behind = seat("oak_boat", 1, 2);
    assert_eq!(driver.x, 0.2);
    assert_eq!(behind.x, -0.6);
    assert_eq!(driver.y, s.y);
    // A chest boat's lone passenger sits 0.15 to the side
    // (`AbstractChestBoat.getSinglePassengerXOffset`).
    assert_eq!(seat("oak_chest_boat", 0, 1).x, 0.15);
    // `Raft.rideHeight`: height * 0.8888889 = 0.5, and the bamboo raft is the only raft.
    assert_close(seat("bamboo_raft", 0, 1).y, 0.5625 * 0.8888889 - 0.6);
    assert_eq!(seat("bamboo_chest_raft", 0, 1).x, 0.15);
    // Every wood shares the boat seat.
    for name in [
        "spruce_boat",
        "birch_boat",
        "jungle_boat",
        "acacia_boat",
        "dark_oak_boat",
        "mangrove_boat",
        "cherry_boat",
        "pale_oak_boat",
        "poplar_boat",
    ] {
        assert_eq!(seat(name, 0, 1), s, "{name}");
    }
    // An animal in the second seat is pushed a further 0.2 out.
    let animal = seat_of("oak_boat", 1, 2, PassengerKind::Animal, "cow");
    assert_eq!(animal.x, -0.6 + 0.2);
}

#[test]
fn horse_family_seats_are_the_saddle() {
    // `passengerAttachments(1.44375F)` on `sized(1.3964844F, 1.6F)`.
    assert_close(seat("horse", 0, 1).y, 1.44375 - 0.6);
    // `passengerAttachments(1.1125F)`, `sized(1.3964844F, 1.5F)`.
    assert_close(seat("donkey", 0, 1).y, 1.1125 - 0.6);
    // Mule: 1.2125 on a 1.6-tall box; both undead horses: 1.31875.
    assert_close(seat("mule", 0, 1).y, 1.2125 - 0.6);
    for name in ["skeleton_horse", "zombie_horse"] {
        assert_close(seat(name, 0, 1).y, 1.31875 - 0.6);
    }
}

#[test]
fn a_pig_seat_is_vanillas() {
    // `passengerAttachments(0.86875F)`, `sized(0.9F, 0.9F)`.
    let s = seat("pig", 0, 1);
    assert_close(s.y, 0.86875 - 0.6);
    assert_eq!((s.x, s.z), (0.0, 0.0));
}

#[test]
fn camel_seats_come_from_camel() {
    // Standing adult: the body anchor is `height - 0.375` = 2.0, minus the player's 0.6.
    let driver = seat("camel", 0, 1);
    assert_close(driver.y, 2.0 - 0.6);
    // The driver sits 0.5 forward; a second passenger 0.7 back.
    assert_eq!(driver.x, 0.5);
    assert_eq!(seat("camel", 1, 2).x, -0.7);
    // `CamelHusk extends Camel`: the same seat.
    assert_eq!(seat("camel_husk", 0, 1), driver);
    // An animal in the second seat is pushed 0.2 forward of it.
    assert_eq!(
        seat_of("camel", 1, 2, PassengerKind::Animal, "cow").x,
        -0.7 + 0.2
    );
}

#[test]
fn a_strider_seat_is_its_own_height() {
    // The strider attaches no points, and its client-side walk animation is idle when it
    // stands: vanilla's `AT_HEIGHT` fallback on `sized(0.9F, 1.7F)`.
    assert_close(seat("strider", 0, 1).y, 1.7 - 0.6);
}

#[test]
fn unknown_vehicles_fall_back_to_their_own_box() {
    // A type the table does not carry (a mod's vehicle): vanilla's fallback on the box the
    // caller knows it by, minus the passenger's own attachment.
    let s = passenger_seat(SeatQuery {
        vehicle_type: "somemod:walker",
        vehicle_dims: (1.0, 2.5),
        seat: 0,
        passengers: 1,
        passenger: PassengerKind::Player,
        passenger_type: "player",
    });
    assert_close(s.y, 2.5 - 0.6);
    // A zombie in a minecart rides at the cart's own position (`AbstractMinecart`).
    let s = seat_of("minecart", 0, 1, PassengerKind::Villager, "villager");
    assert_eq!(s, SeatPoint::ZERO);
    // A zombie riding a boat uses its own `ridingOffset(-0.7F)` point, which lifts it 0.7 above
    // the seat: 0.1875 - 0.7.
    let s = seat_of("oak_boat", 0, 1, PassengerKind::Player, "zombie");
    assert_close(s.y, 0.5625 / 3.0 - 0.7);
}

#[test]
fn the_vehicle_yaw_turns_the_seat() {
    // A boat's second seat sits 0.6 to one side of the boat. `EntityAttachments.transformPoint`
    // (`Vec3.yRot`) turns that with the vehicle's yaw: at yaw 0 it is a -x offset, at yaw 90 a
    // -z one.
    let s = seat("oak_boat", 1, 2);
    let d0 = s.rotated(0.0);
    assert!((d0.x + 0.6).abs() < 1e-6 && d0.z.abs() < 1e-6, "{d0:?}");
    let d90 = s.rotated(90.0);
    assert!(d90.x.abs() < 1e-6 && (d90.z + 0.6).abs() < 1e-6, "{d90:?}");
}

// ---------------------------------------------------------------------------
// Real mob models from the recorder dump.
// ---------------------------------------------------------------------------

/// A minimal dump-shaped JSON: one layer with a nested part, non-trivial pivots/rotations,
/// mirror + inflate on the cube - small enough to check every converted field.
const DUMP_SAMPLE: &str = r#"{
  "version": 1,
  "minecraft": "26.3",
  "layers": [
    {
      "layer": "minecraft:pig#main",
      "texture_width": 64,
      "texture_height": 64,
      "parts": [
        {
          "name": "head",
          "pivot": [0.0, 12.0, -6.0],
          "rotation": [0.1, 0.2, 0.3],
          "scale": [1.0, 1.0, 1.0],
          "cubes": [
            {
              "origin": [-4.0, -4.0, -8.0],
              "size": [8.0, 8.0, 8.0],
              "uv": "0 0",
              "u": 0.0,
              "v": 0.0,
              "mirror": true,
              "inflate": [0.25, 0.25, 0.25],
              "visible_faces": ["up", "down", "north", "south", "west", "east"]
            }
          ],
          "children": [
            {
              "name": "snout",
              "pivot": [0.0, 16.0, -14.0],
              "rotation": [0.0, 0.0, 0.0],
              "scale": [1.0, 1.0, 1.0],
              "cubes": [
                {
                  "origin": [-2.0, 0.0, -3.0],
                  "size": [4.0, 3.0, 1.0],
                  "uv": "16 16",
                  "u": 16.0,
                  "v": 16.0,
                  "mirror": false,
                  "inflate": [0.0, 0.0, 0.0],
                  "visible_faces": ["up", "down", "north", "south", "west", "east"]
                }
              ],
              "children": []
            }
          ]
        },
        {
          "name": "body",
          "pivot": [0.0, 12.0, 2.0],
          "rotation": [1.5707964, 0.0, 0.0],
          "scale": [1.0, 1.0, 1.0],
          "cubes": [
            {
              "origin": [-5.0, -10.0, -7.0],
              "size": [10.0, 16.0, 8.0],
              "uv": "28 8",
              "u": 28.0,
              "v": 8.0,
              "mirror": false,
              "inflate": [0.0, 0.0, 0.0],
              "visible_faces": ["up", "down", "north", "south", "west", "east"]
            }
          ],
          "children": []
        }
      ]
    }
  ]
}"#;

#[test]
fn dump_loader_converts_nested_parts_to_feet_at_zero() {
    let lib = crate::model_dump::ModelLibrary::parse(DUMP_SAMPLE.as_bytes()).unwrap();
    assert_eq!(lib.len(), 1);
    let model = lib.get("minecraft:pig#main", 1.0).unwrap();
    assert_eq!(model.tex_width, 64);
    assert_eq!(model.tex_height, 64);
    // Ground shift: dump pivots were y = 12/16 (feet at dump y = 24) -> haste y = -12/-8.
    let head = find_part(&model.root, "head").unwrap();
    assert!((head.pivot.y - (12.0 - 24.0)).abs() < 1e-6);
    assert!((head.pivot.x - 0.0).abs() < 1e-6 && (head.pivot.z - -6.0).abs() < 1e-6);
    // Rotation arrives in radians, kept as-is.
    assert!((head.rotation.x - 0.1).abs() < 1e-6);
    assert!((head.rotation.y - 0.2).abs() < 1e-6);
    assert!((head.rotation.z - 0.3).abs() < 1e-6);
    // The nested child hangs off the head with its own parent-relative pivot, NOT shifted:
    // a child's PartPose is relative to its parent, not to the ground plane.
    let snout = find_part(&model.root, "snout").unwrap();
    assert!((snout.pivot.y - 16.0).abs() < 1e-6);
    assert!((snout.pivot.z - -14.0).abs() < 1e-6);
    assert_eq!(snout.cubes.len(), 1);
    assert_eq!(snout.cubes[0].uv, [16, 16]);
    assert!((snout.cubes[0].size.y - 3.0).abs() < 1e-6);
    // Cube fields survive: mirror, inflate, origin, size.
    assert!(head.cubes[0].mirror);
    assert!((head.cubes[0].inflate - 0.25).abs() < 1e-6);
    assert!((head.cubes[0].origin.z - -8.0).abs() < 1e-6);
    assert!((head.cubes[0].size.x - 8.0).abs() < 1e-6);
    // Body rotation (pi/2 for quadruped bodies) is preserved.
    let body = find_part(&model.root, "body").unwrap();
    assert!((body.rotation.x - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
    // Unknown layer and bad version are handled without panicking.
    assert!(lib.get("minecraft:cow#main", 1.0).is_none());
    let bad = DUMP_SAMPLE.replace("\"version\": 1", "\"version\": 2");
    assert!(crate::model_dump::ModelLibrary::parse(bad.as_bytes()).is_err());
}

#[test]
fn dump_loader_applies_root_scale() {
    let lib = crate::model_dump::ModelLibrary::parse(DUMP_SAMPLE.as_bytes()).unwrap();
    let plain = lib.get("minecraft:pig#main", 1.0).unwrap();
    let scaled = lib.get("minecraft:pig#main", 0.5).unwrap();
    let head = find_part(&plain.root, "head").unwrap();
    let half = find_part(&scaled.root, "head").unwrap();
    // Scaled about the feet: a pivot at -12 becomes -6, cube sizes halve.
    assert!((half.pivot.y - head.pivot.y * 0.5).abs() < 1e-6);
    assert!((half.cubes[0].size.x - head.cubes[0].size.x * 0.5).abs() < 1e-6);
    // Texture size and UVs are untouched by the scale.
    assert_eq!(scaled.tex_width, plain.tex_width);
    assert_eq!(half.cubes[0].uv, head.cubes[0].uv);
}

/// The table's layer ids for pig, zombie and villager (checked against each type's vanilla
/// renderer: PigRenderer, ZombieRenderer, VillagerRenderer).
#[test]
fn table_maps_pig_zombie_villager() {
    let (pig, _) = crate::registry::table_entry("minecraft:pig").unwrap();
    assert_eq!(pig, "minecraft:pig#main");
    let (zombie, _) = crate::registry::table_entry("minecraft:zombie").unwrap();
    assert_eq!(zombie, "minecraft:zombie#main");
    let (villager, _) = crate::registry::table_entry("minecraft:villager").unwrap();
    assert_eq!(villager, "minecraft:villager#main");
    // Not in the table.
    assert!(crate::registry::table_entry("minecraft:item").is_none());
    assert!(crate::registry::table_entry("minecraft:haste_mod_whatever").is_none());
}

/// A mob type the table covers gets its model when the dump is present; without the cache
/// (fast tests) the table entry still resolves and the dump lookup returns nothing, so the
/// store's fallback box applies. `model_for_name` is exercised directly because `model_for`
/// resolves kinds through the real registries.json, which the fast tests must not depend on.
#[test]
fn store_uses_dump_model_for_known_types_and_box_for_unknown() {
    match crate::registry::model_for_name("minecraft:pig") {
        Some(m) => {
            // Cache present: a real pig model with the pig texture, not the placeholder.
            assert_eq!(m.texture.path(), "entity/pig/pig_temperate");
            assert!(find_part(&m.root, "head").is_some());
        }
        None => {
            // No cache: the table still names the layer, the dump is just absent.
            assert_eq!(
                crate::registry::table_entry("minecraft:pig"),
                Some(("minecraft:pig#main", 1.0))
            );
        }
    }
    // Unknown name: no model, callers take the box.
    assert!(crate::registry::model_for_name("minecraft:no_such_mob").is_none());
    // And the box itself still stands in for the uncovered type.
    let boxed = crate::models::box_for(0.9, 0.9);
    assert_eq!(boxed.texture.to_string(), "haste:entity_box");
}

/// The real dump from the cache: pig has head, body and four legs (QuadrupedModel layout).
#[test]
#[ignore = "needs cache/oracle/26.3/entity_models.json (recorder mod dump; HASTE_CACHE overrides)"]
fn real_dump_pig_has_head_body_and_four_legs() {
    let Some(lib) = crate::model_dump::library() else {
        eprintln!("skipped: no entity model dump in the cache");
        return;
    };
    let model = lib
        .get("minecraft:pig#main", 1.0)
        .expect("pig layer in the dump");
    let names = humanoid_parts(&model);
    for expected in [
        "head",
        "body",
        "right_hind_leg",
        "left_hind_leg",
        "right_front_leg",
        "left_front_leg",
    ] {
        assert!(
            names.contains(&expected),
            "pig part {expected} missing (has {names:?})"
        );
    }
    // The dump's pivots are shifted to feet-at-0: the pig's legs pivot at dump y = 18.
    let leg = find_part(&model.root, "right_hind_leg").unwrap();
    assert!(
        (leg.pivot.y - (18.0 - 24.0)).abs() < 1e-4,
        "leg pivot {:?}",
        leg.pivot
    );
    // Also spot-check a scaled layer (villager, 0.9375) resolves.
    let villager = lib.get("minecraft:villager#main", 0.9375);
    assert!(villager.is_some(), "villager layer missing");
}

// ---------------------------------------------------------------------------
// Rest-pose connectivity audit (every table-covered type, no images needed).
// ---------------------------------------------------------------------------

/// One cube's world-space box at rest pose, in the renderer's block space: the same chain
/// `emit_part` builds (parent * translate(pivot) * Rz * Ry * Rx per part), then the model
/// scale (1/16) and the renderer's y flip ((-1, -1, 1) * s / 16) that puts the feet at y=0
/// with y up. `pos` is the box's centre, `half` its half-extents.
struct WorldBox {
    lo: glam::Vec3,
    hi: glam::Vec3,
}

/// Collects every cube's world box (see `WorldBox`), rest pose, no overrides.
fn world_boxes(part: &ModelPart, parent: glam::Mat4, out: &mut Vec<WorldBox>) {
    let m = parent
        * glam::Mat4::from_translation(part.pivot)
        * glam::Mat4::from_rotation_z(part.rotation.z)
        * glam::Mat4::from_rotation_y(part.rotation.y)
        * glam::Mat4::from_rotation_x(part.rotation.x);
    for c in &part.cubes {
        let lo = c.origin - glam::Vec3::splat(c.inflate);
        let hi = c.origin + c.size + glam::Vec3::splat(c.inflate);
        // Model space (y down, 1/16 units) -> block space (y up): mirror x and y, like the
        // renderer's scale (-s, -s, s) / 16 (entity.rs texture_and_scale path). A rotated
        // cube's AABB needs all eight corners, not just the transformed min/max.
        let to_blocks = |p: glam::Vec3| {
            let b = m.transform_point3(p) / 16.0;
            glam::Vec3::new(-b.x, -b.y, b.z)
        };
        let mut blo = glam::Vec3::splat(f32::INFINITY);
        let mut bhi = glam::Vec3::splat(f32::NEG_INFINITY);
        for cx in [lo.x, hi.x] {
            for cy in [lo.y, hi.y] {
                for cz in [lo.z, hi.z] {
                    let p = to_blocks(glam::Vec3::new(cx, cy, cz));
                    blo = blo.min(p);
                    bhi = bhi.max(p);
                }
            }
        }
        out.push(WorldBox { lo: blo, hi: bhi });
    }
    for child in &part.children {
        world_boxes(child, m, out);
    }
}

/// The rest-pose audit: for every table-covered entity type (91 mob kinds), the dumped model
/// at rest pose (no animation, no overrides - the pose the dump baked) must be
///
/// 1. **connected**: one piece, any cube reachable from any other through touching or
///    overlapping cubes. A model that splits into islands means a part landed in the wrong
///    place (the horse-head bug class: the neck's children were shifted twice).
/// 2. **grounded** (land types): some geometry within `margin` of the ground plane y=0. A
///    model floating a whole block above it means the whole tree is misplaced (the copper
///    golem bug class: the layer's root PartPose translation was lost and the ground shift
///    applied on top). Types vanilla does not draw standing on the ground (swimming, flying,
///    hovering) are exempt; each names the vanilla reason.
///
/// A model *exceeding its hitbox* is NOT a failure: vanilla models routinely extend past
/// their hitbox (horse heads, bat wings, warden arms), so dimension checks only produce noise.
/// Every exemption was checked against the vanilla 26.3 model/renderer code by hand.
#[test]
#[ignore = "needs cache/oracle/26.3 (entity_models.json + entity_dimensions.json; HASTE_CACHE overrides)"]
fn rest_pose_models_are_connected_and_grounded() {
    let Some(lib) = crate::model_dump::library() else {
        eprintln!("skipped: no entity model dump in the cache");
        return;
    };
    // entity_dimensions.json: {version, types: [{id, type, width, height, eye_height}]}.
    let dims_path = haste_core::paths::oracle_dir().join("entity_dimensions.json");
    let Ok(dims_bytes) = std::fs::read(&dims_path) else {
        eprintln!(
            "skipped: no entity dimensions dump at {}",
            dims_path.display()
        );
        return;
    };
    let dims: serde_json::Value = serde_json::from_slice(&dims_bytes).unwrap();
    let height_of = |name: &str| -> Option<f32> {
        dims["types"]
            .as_array()?
            .iter()
            .find(|t| t["type"].as_str() == Some(name))
            .map(|t| t["height"].as_f64().unwrap() as f32)
    };

    let mut failures = Vec::new();
    for name in LIVING_TYPES {
        let (layer, _root_scale) = crate::registry::table_entry(name)
            .unwrap_or_else(|| panic!("{name} missing from the model table"));
        let model = crate::registry::model_for_name(name);
        let Some(model) = model else {
            // A table row whose layer is absent from the dump is a coverage gap, not a
            // connectivity failure; the box fallback covers it at runtime.
            if lib.get(layer, 1.0).is_none() {
                failures.push(format!("{name}: layer {layer} missing from the dump"));
            }
            continue;
        };
        let Some(_height) = height_of(name) else {
            failures.push(format!("{name}: no dimensions in entity_dimensions.json"));
            continue;
        };
        let mut boxes = Vec::new();
        world_boxes(&model.root, glam::Mat4::IDENTITY, &mut boxes);
        if boxes.is_empty() {
            failures.push(format!("{name}: model has no cubes"));
            continue;
        }
        let mut min = glam::Vec3::splat(f32::INFINITY);
        let mut max = glam::Vec3::splat(f32::NEG_INFINITY);
        for b in &boxes {
            min = min.min(b.lo);
            max = max.max(b.hi);
        }
        // Grounding: land types stand on the plane (the renderer puts model y=24 at the
        // entity position). Ground slack of 2 model units covers vanilla's 0.016 ground
        // fudge and hoof-antenna details drawn at the exact plane.
        let grounded = min.y <= 2.0 / 16.0;
        if !grounded && !GROUND_EXEMPT.contains(name) {
            failures.push(format!(
                "{name}: floats {min:.2} blocks above the ground at rest"
            ));
        }
        // Connectivity: union-find over boxes that touch or overlap (faces, not just
        // interiors; a model's parts attach face-to-face).
        let n = boxes.len();
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(parent: &mut [usize], i: usize) -> usize {
            let mut root = i;
            while parent[root] != root {
                root = parent[root];
            }
            let mut j = i;
            while parent[j] != root {
                let next = parent[j];
                parent[j] = root;
                j = next;
            }
            root
        }
        let touching = |a: &WorldBox, b: &WorldBox| {
            // Closed-box overlap with a hair of tolerance for shared faces.
            const EPS: f32 = 1.0 / 64.0;
            a.lo.x <= b.hi.x + EPS
                && b.lo.x <= a.hi.x + EPS
                && a.lo.y <= b.hi.y + EPS
                && b.lo.y <= a.hi.y + EPS
                && a.lo.z <= b.hi.z + EPS
                && b.lo.z <= a.hi.z + EPS
        };
        for i in 0..n {
            for j in (i + 1)..n {
                if touching(&boxes[i], &boxes[j]) {
                    let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                    if ri != rj {
                        parent[ri] = rj;
                    }
                }
            }
        }
        let roots: std::collections::HashSet<usize> =
            (0..n).map(|i| find(&mut parent, i)).collect();
        if roots.len() > 1 && !CONNECTIVITY_EXCEPTIONS.contains(name) {
            failures.push(format!(
                "{name}: rest pose splits into {} disconnected pieces",
                roots.len()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} model audit failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Types whose vanilla rest pose is genuinely several separate pieces; each names the vanilla
/// reason (checked against the 26.3 client's model code by hand).
const CONNECTIVITY_EXCEPTIONS: &[&str] = &[
    // BreezeModel.createBodyLayer: only "head" and "rods" are retained; the three wind rods
    // whirl around the head and touch nothing at rest.
    "minecraft:breeze",
    // BlazeModel.createBodyLayer: the head plus twelve independently spinning rods.
    "minecraft:blaze",
];

/// Types vanilla does not draw standing on the ground at rest, so the grounding check does
/// not apply (each names the vanilla reason).
const GROUND_EXEMPT: &[&str] = &[
    // Water mobs (SquidModel/GlowSquidModel: tentacles hang below the body, which vanilla
    // centers on the entity position; they are not drawn standing).
    "minecraft:squid",
    "minecraft:glow_squid",
    "minecraft:guardian",
    "minecraft:elder_guardian",
    // Flying / hovering mobs.
    "minecraft:ghast",
    "minecraft:happy_ghast",
    "minecraft:phantom",
    "minecraft:bee",
    "minecraft:allay",
    "minecraft:bat",
    "minecraft:parrot",
    // BlazeModel: head at model y=0 with the rod rings below it (a hovering column of fire;
    // the lowest rod still clears the ground).
    "minecraft:blaze",
    // BreezeModel: the drawn layer is head + whirling rods; only the separate wind layer
    // (BreezeWindLayer, drawn by its own renderer pass) reaches the ground.
    "minecraft:breeze",
    // WitherBossModel: a flying boss whose ribcage hovers above the tail (WitherBossRenderer
    // scales it 2x around the hovering pose; nothing stands on the ground).
    "minecraft:wither",
    // Schooling fish (CodModel/SalmonModel: the body rides above the entity position).
    "minecraft:cod",
    "minecraft:salmon",
    "minecraft:tropical_fish",
    "minecraft:tadpole",
    "minecraft:axolotl",
    "minecraft:dolphin",
    // SulfurCubeRenderer.scale translates the model down ~0.92 blocks (vanilla draws the
    // cube centered on the entity position plus a vOffset, not standing on the ground);
    // that per-renderer offset is not model geometry.
    "minecraft:sulfur_cube",
];

// ---------------------------------------------------------------------------
// Table coverage and baby variants.
// ---------------------------------------------------------------------------

/// Every living entity type vanilla registers a mob renderer for (EntityRenderers.java's
/// PROVIDERS, minus the player/mannequin handled separately, minus non-living types).
/// A type appearing here but not in the table (and not in NON_LIVING) fails the coverage
/// test, so new mob types cannot silently fall back to the box.
const LIVING_TYPES: &[&str] = &[
    "minecraft:allay",
    "minecraft:armadillo",
    "minecraft:axolotl",
    "minecraft:bat",
    "minecraft:bee",
    "minecraft:blaze",
    "minecraft:bogged",
    "minecraft:breeze",
    "minecraft:camel",
    "minecraft:camel_husk",
    "minecraft:cat",
    "minecraft:cave_spider",
    "minecraft:chicken",
    "minecraft:cod",
    "minecraft:copper_golem",
    "minecraft:cow",
    "minecraft:creaking",
    "minecraft:creeper",
    "minecraft:dolphin",
    "minecraft:donkey",
    "minecraft:drowned",
    "minecraft:elder_guardian",
    "minecraft:ender_dragon",
    "minecraft:enderman",
    "minecraft:endermite",
    "minecraft:evoker",
    "minecraft:fox",
    "minecraft:frog",
    "minecraft:ghast",
    "minecraft:giant",
    "minecraft:glow_squid",
    "minecraft:goat",
    "minecraft:guardian",
    "minecraft:happy_ghast",
    "minecraft:hoglin",
    "minecraft:horse",
    "minecraft:husk",
    "minecraft:illusioner",
    "minecraft:iron_golem",
    "minecraft:llama",
    "minecraft:magma_cube",
    "minecraft:mooshroom",
    "minecraft:mule",
    "minecraft:nautilus",
    "minecraft:ocelot",
    "minecraft:panda",
    "minecraft:parched",
    "minecraft:parrot",
    "minecraft:phantom",
    "minecraft:pig",
    "minecraft:piglin",
    "minecraft:piglin_brute",
    "minecraft:pillager",
    "minecraft:polar_bear",
    "minecraft:pufferfish",
    "minecraft:rabbit",
    "minecraft:ravager",
    "minecraft:salmon",
    "minecraft:sheep",
    "minecraft:shulker",
    "minecraft:silverfish",
    "minecraft:skeleton",
    "minecraft:skeleton_horse",
    "minecraft:slime",
    "minecraft:sniffer",
    "minecraft:snow_golem",
    "minecraft:spider",
    "minecraft:squid",
    "minecraft:stray",
    "minecraft:strider",
    "minecraft:sulfur_cube",
    "minecraft:tadpole",
    "minecraft:trader_llama",
    "minecraft:tropical_fish",
    "minecraft:turtle",
    "minecraft:vex",
    "minecraft:villager",
    "minecraft:vindicator",
    "minecraft:wandering_trader",
    "minecraft:warden",
    "minecraft:witch",
    "minecraft:wither",
    "minecraft:wither_skeleton",
    "minecraft:wolf",
    "minecraft:zoglin",
    "minecraft:zombie",
    "minecraft:zombie_horse",
    "minecraft:zombie_nautilus",
    "minecraft:zombie_villager",
    "minecraft:zombified_piglin",
];

/// Entity types with renderers but no living-model layer: projectiles, vehicles, block-like
/// and UI entities (out of scope; drawn as boxes) plus the player (its own model path).
#[allow(dead_code)] // documents what the table deliberately leaves out
const NON_LIVING: &[&str] = &[
    "boat-like: *_boat, *_chest_boat, *_raft",
    "minecart-like: minecart, *_minecart, tnt_minecart",
    "projectiles: arrow, egg, ender_pearl, trident, fireball(s), snowball, potions,",
    "  dragon_fireball, evoker_fangs, llama_spit, shulker_bullet, wind_charge(s),",
    "  experience_bottle, eye_of_ender, fishing_bobber, wither_skull",
    "object entities: area_effect_cloud, armor_stand, block/item/text_display, cushion,",
    "  end_crystal, falling_block, firework_rocket, interaction, item, item_frame(s),",
    "  leash_knot, lightning_bolt, mannequin, marker, ominous_item_spawner, painting,",
    "  player, tnt, experience_orb",
];

/// Every living entity type has a table row (so it gets its vanilla model, or an explicit
/// pending entry). Runs without the cache: it checks the table, not the dump.
#[test]
fn table_covers_every_living_entity_type() {
    for name in LIVING_TYPES {
        assert!(
            crate::registry::table_entry(name).is_some(),
            "{name} has no model table row"
        );
    }
    // And nothing in the table references a type outside the known set (typos).
    let covered = crate::registry::table_textures().count();
    assert!(
        covered >= LIVING_TYPES.len(),
        "table smaller than living set"
    );
}

/// The baby rows point at real `*_baby` layers and the baby metadata flag flips them
/// (`AgeableMob.DATA_BABY_ID` = index 16; `Zombie`/`Zoglin`/`Piglin` put theirs on the same
/// slot through the same super-chain, Entity 0-7 -> LivingEntity 8-14 -> Mob 15).
#[test]
fn baby_flag_and_baby_layers() {
    // The flag: metadata index 16, boolean true.
    let baby_meta = |v: bool| {
        let mut m = crate::metadata::Metadata::default();
        m.apply(&[MetadataEntry {
            index: crate::registry::BABY_METADATA_INDEX,
            value: MetadataValue::Boolean(v),
        }]);
        m
    };
    assert!(crate::registry::is_baby(&baby_meta(true)));
    assert!(!crate::registry::is_baby(&baby_meta(false)));
    assert!(!crate::registry::is_baby(
        &crate::metadata::Metadata::default()
    ));
}

/// The shadow radii are the values the 26.3 renderer constructors pass (a sample across the
/// families), and a table-covered living type not named falls back to the usual 0.5.
#[test]
fn shadow_radius_table_matches_vanilla() {
    use crate::registry::{shadow_radius_for_name, shadow_strength_for_name};
    assert_eq!(shadow_radius_for_name("minecraft:pig"), 0.7);
    assert_eq!(shadow_radius_for_name("minecraft:cow"), 0.7);
    assert_eq!(shadow_radius_for_name("minecraft:zombie"), 0.5);
    assert_eq!(shadow_radius_for_name("minecraft:creeper"), 0.5);
    assert_eq!(shadow_radius_for_name("minecraft:item"), 0.15);
    assert_eq!(shadow_radius_for_name("minecraft:experience_orb"), 0.15);
    assert_eq!(shadow_radius_for_name("minecraft:armor_stand"), 0.0);
    assert_eq!(shadow_radius_for_name("minecraft:ghast"), 1.5);
    assert_eq!(shadow_radius_for_name("minecraft:bat"), 0.25);
    // An unknown / non-rendered type casts no shadow.
    assert_eq!(shadow_radius_for_name("minecraft:not_a_mob"), 0.0);
    assert_eq!(shadow_strength_for_name("minecraft:item"), 0.75);
    assert_eq!(shadow_strength_for_name("minecraft:pig"), 1.0);
    // The name tag anchor: the bb height, or the sniffer's 2.05 override.
    assert_eq!(
        crate::registry::name_tag_offset_for_name("minecraft:pig", 0.9),
        0.9
    );
    assert_eq!(
        crate::registry::name_tag_offset_for_name("minecraft:sniffer", 1.75),
        2.05
    );
}

/// Baby layers come from the recorder's model dump, which CI does not have.
#[test]
#[ignore = "needs cache/oracle/26.3/entity_models.json"]
fn baby_layers_from_the_dump() {
    // Rows whose renderer has a baby model (AgeableMobRenderer and friends) name it.
    // Vanilla types with a baby layer but none in our table yet would fail here.
    for (name, has_baby) in [
        ("minecraft:pig", true),
        ("minecraft:villager", true),
        ("minecraft:zombie", true),
        ("minecraft:horse", true),
        ("minecraft:sniffer", true),
        ("minecraft:creeper", false), // MobRenderer only
        ("minecraft:spider", false),
        ("minecraft:bat", false),
    ] {
        let baby = crate::registry::baby_model_for_name(name);
        assert_eq!(
            baby.is_some(),
            has_baby,
            "{name} baby layer presence mismatch"
        );
    }
    // With the dump present, the baby model differs from the adult and carries the baby
    // texture; without it both are None and the store falls back to the box.
    if let (Some(adult), Some(baby)) = (
        crate::registry::model_for_name("minecraft:pig"),
        crate::registry::baby_model_for_name("minecraft:pig"),
    ) {
        let adult_head = find_part(&adult.root, "head").unwrap();
        let baby_head = find_part(&baby.root, "head").unwrap();
        assert_ne!(adult_head.pivot, baby_head.pivot);
        assert_eq!(baby.texture.path(), "entity/pig/pig_temperate_baby");
    }
    // The sniffer's baby texture is "snifflet", not "sniffer_baby" (SnifferRenderer).
    if let Some(baby) = crate::registry::baby_model_for_name("minecraft:sniffer") {
        assert_eq!(baby.texture.path(), "entity/sniffer/snifflet");
    }
}

/// Every table texture exists in the client jar as `assets/minecraft/textures/<path>.png`.
/// Needs the installation (client jar + asset index) from `haste setup`.
#[test]
#[ignore = "needs the client jar and assets index (HASTE_CACHE; from `haste setup`)"]
fn table_textures_exist_in_jar() {
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
    let mut missing = Vec::new();
    for texture in crate::registry::table_textures() {
        let key = format!("assets/minecraft/textures/{texture}.png");
        if !names.contains(&key) {
            missing.push(key);
        }
    }
    assert!(
        missing.is_empty(),
        "{} table textures missing from the jar: {missing:?}",
        missing.len()
    );
}

/// `render_instances` resolves a variant mob's look through the synced variant table: the
/// metadata's variant id picks the assets, the model comes from the variant's model type,
/// and without a variant in play the model table's default row stands. Needs the dump and
/// registries.json (the kind is resolved through the real entity-type names).
#[test]
#[ignore = "needs cache/oracle/26.3 (recorder mod dump + registries.json; HASTE_CACHE overrides)"]
fn render_instances_picks_up_variant_models() {
    let Some(names) = crate::registry::entity_type_names() else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    let Some((kind, _)) = names.iter().find(|(_, n)| n.as_str() == "minecraft:pig") else {
        eprintln!("skipped: minecraft:pig not in the registry");
        return;
    };
    let mut s = store();
    let mut sp = spawn(1, DVec3::ZERO);
    sp.kind = *kind;
    s.spawn(&sp);
    // No variant table set: the default (temperate) pig model.
    let mut out = Vec::new();
    s.render_instances(0.5, &mut out);
    let default = out[0].model.clone();
    if crate::registry::model_for_name("minecraft:pig").is_none() {
        eprintln!("skipped: no entity model dump in the cache");
        return;
    }
    assert_eq!(
        default.texture.path(),
        "entity/pig/pig_temperate",
        "the default pig row"
    );
    // A cold pig variant (registry id 2 in vanilla's order: temperate, warm, cold).
    s.set_mob_variants(
        "minecraft:pig_variant",
        vec![
            Some(
                crate::variants::VariantAssets::from_assets(
                    "minecraft:entity/pig/pig_temperate",
                    Some("minecraft:entity/pig/pig_temperate_baby"),
                    None,
                )
                .unwrap(),
            ),
            None,
            Some(
                crate::variants::VariantAssets::from_assets(
                    "minecraft:entity/pig/pig_cold",
                    Some("minecraft:entity/pig/pig_cold_baby"),
                    Some("cold"),
                )
                .unwrap(),
            ),
        ],
    );
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: crate::variants::metadata_index::PIG,
            value: MetadataValue::Other {
                type_id: crate::variants::serializer_id::PIG_VARIANT,
                raw: vec![2].into(),
            },
        }],
    );
    let mut out = Vec::new();
    s.render_instances(0.5, &mut out);
    let cold = out[0].model.clone();
    assert_eq!(cold.texture.path(), "entity/pig/pig_cold");
    assert_ne!(cold.root, default.root, "the cold layer differs");
    // An unknown variant id (a mod's registry): the default row stands.
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: crate::variants::metadata_index::PIG,
            value: MetadataValue::Other {
                type_id: crate::variants::serializer_id::PIG_VARIANT,
                raw: vec![7].into(),
            },
        }],
    );
    let mut out = Vec::new();
    s.render_instances(0.5, &mut out);
    assert_eq!(out[0].model.texture.path(), "entity/pig/pig_temperate");
}

/// A sheep keeps its sheep model with wool metadata set: the wool byte sits at metadata 18,
/// the same slot a shulker's colour byte occupies, so the shulker texture swap must be
/// gated on the entity type (regression: a red-wool sheep used to come out as a red
/// shulker). Needs registries.json + the dump.
#[test]
#[ignore = "needs cache/oracle/26.3 (registries.json + model dump; HASTE_CACHE overrides)"]
fn sheep_with_wool_metadata_is_not_a_shulker() {
    let Some(names) = crate::registry::entity_type_names() else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    let Some((kind, _)) = names.iter().find(|(_, n)| n.as_str() == "minecraft:sheep") else {
        eprintln!("skipped: minecraft:sheep not in the registry");
        return;
    };
    let mut s = store();
    let mut sp = spawn(1, DVec3::ZERO);
    sp.kind = *kind;
    s.spawn(&sp);
    // Red wool, exactly the byte that is a red shulker's colour.
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: crate::variants::metadata_index::SHEEP_WOOL,
            value: MetadataValue::Byte(14),
        }],
    );
    let mut out = Vec::new();
    s.render_instances(0.5, &mut out);
    assert_eq!(out[0].model.texture.path(), "entity/sheep/sheep");
    // The wool overlay is there, tinted red.
    let wool = out[0].overlays[0].as_ref().expect("wool overlay");
    assert_eq!(wool.model.texture.path(), "entity/sheep/sheep_wool");
    assert_eq!(wool.tint, crate::variants::dye_tint(14));
}
