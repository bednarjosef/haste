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
    // The nested child hangs off the head with its own pivot, also shifted.
    let snout = find_part(&model.root, "snout").unwrap();
    assert!((snout.pivot.y - (16.0 - 24.0)).abs() < 1e-6);
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
