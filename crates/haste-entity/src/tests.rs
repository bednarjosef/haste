//! Tests for Task 1: tracking + interpolation. Timed tests are `#[ignore]`d in debug (too slow to
//! be meaningful) and run in release via `cargo test --release -- --ignored`.
use std::time::Instant;

use haste_core::entity::{EntitySpawn, MetadataEntry, MetadataValue, PlayerInfo, Pose};
use haste_core::game::GameMode;
use haste_core::ids::{EntityId, RegistryIndex};
use haste_core::model::EntityInstance;
use haste_core::DVec3;
use uuid::Uuid;

use crate::dims::EntityDimensions;
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

/// A dropped item entity carries its stack in metadata index 9 (`Item`): it reaches the
/// draw list with the item's registry id, the stack count and its age (partial-tick
/// interpolated), and an emptied stack clears it.
#[test]
fn item_entity_stack_from_metadata() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    let stack = |count, item: Option<u32>| MetadataEntry {
        index: 9,
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
    // A few idle ticks: no limb rotation at all (only the head aim remains).
    for _ in 0..5 {
        s.tick();
    }
    let mut out = Vec::new();
    s.render_instances(0.5, &mut out);
    let limbs: Vec<_> = out[0]
        .pose
        .rotations
        .iter()
        .filter(|(n, _)| n.ends_with("arm") || n.ends_with("leg"))
        .collect();
    assert!(limbs.is_empty(), "limb rotations at rest: {limbs:?}");
    // Walking: constant 0.1-block deltas each tick; amplitude is smoothed, bounded < pi/4.
    for _ in 0..20 {
        s.move_delta(EntityId(1), DVec3::new(0.1, 0.0, 0.0), true);
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
