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
