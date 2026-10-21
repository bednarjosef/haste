//! Tracking emitter tests: the sixteen-attempt tick's offsets and velocities, the three-tick
//! life, and the `animate` packet's critical hit actions against the real `EntityStore`.

use std::sync::Arc;

use glam::DVec3;
use haste_audio::random::LegacyRandom;
use haste_core::entity::EntitySpawn;
use haste_core::{EntityId, Identifier, RegistryIndex};
use haste_protocol::haste_core::game::{DimensionInfo, GameMode};
use haste_protocol::packets::play::clientbound as cb;
use haste_protocol::ClientboundPlay;

use super::*;
use crate::net::{new_entity_store, Session};

/// The local player's entity id in these tests.
const OWN: i32 = 7;

fn body() -> Body {
    Body {
        pos: DVec3::new(10.0, 64.0, -3.0),
        width: 0.6,
        height: 1.8,
    }
}

fn emitter(kind: &str) -> TrackingEmitter {
    TrackingEmitter {
        target: EntityId(5),
        kind: Identifier::minecraft(kind),
        life: 0,
        body: body(),
    }
}

fn session() -> Session {
    Session::new(
        EntityId(OWN),
        GameMode::Creative,
        DimensionInfo::overworld(),
        Arc::new(Vec::new()),
    )
}

fn spawn_zombie(store: &mut haste_entity::EntityStore, pos: DVec3) {
    store.spawn(&EntitySpawn {
        id: EntityId(5),
        uuid: Default::default(),
        kind: RegistryIndex(0),
        pos,
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 0.0,
        data: 0,
        velocity: DVec3::ZERO,
    });
}

fn animate(entity_id: i32, animation: u8) -> ClientboundPlay {
    ClientboundPlay::Animate(cb::Animate {
        entity_id,
        animation,
    })
}

#[test]
fn each_kept_particle_sits_in_the_body_and_flies_with_vanillas_velocity() {
    let mut e = emitter("crit");
    let mut rng = LegacyRandom::new(1);
    let mut out = Vec::new();
    e.tick(&mut rng, &mut out);
    assert!(!out.is_empty() && out.len() <= 16);
    for s in &out {
        // The velocity is `(xa, ya + 0.2, za)` with the offset inside the unit sphere.
        let (xa, ya, za) = (
            f64::from(s.offset[0]),
            f64::from(s.offset[1]) - 0.2,
            f64::from(s.offset[2]),
        );
        assert!(xa * xa + ya * ya + za * za <= 1.0 + 1e-5);
        // `getX(xa / 4)`, `getY(0.5 + ya / 4)`, `getZ(za / 4)` on the body's box.
        assert!((s.pos.x - (10.0 + 0.6 * xa / 4.0)).abs() < 1e-4);
        assert!((s.pos.y - (64.0 + 1.8 * (0.5 + ya / 4.0))).abs() < 1e-4);
        assert!((s.pos.z - (-3.0 + 0.6 * za / 4.0)).abs() < 1e-4);
        assert_eq!(s.kind, Identifier::minecraft("crit"));
        assert_eq!(s.count, 0);
    }
}

#[test]
fn a_tick_keeps_about_half_of_its_sixteen_attempts() {
    // The unit cube's corners are rejected: a sphere's share of the cube is pi / 6, so about
    // 8.4 of 16 attempts are kept.
    let mut e = emitter("crit");
    let mut rng = LegacyRandom::new(9);
    let mut out = Vec::new();
    let ticks = 1000;
    for _ in 0..ticks {
        e.life = 0;
        e.tick(&mut rng, &mut out);
    }
    let average = out.len() as f64 / ticks as f64;
    assert!((7.0..10.0).contains(&average), "average {average}");
}

#[test]
fn an_emitter_lives_three_ticks_including_the_constructors() {
    let mut e = emitter("crit");
    let mut rng = LegacyRandom::new(2);
    let mut out = Vec::new();
    e.tick(&mut rng, &mut out);
    assert_eq!(e.life, 1);
    assert!(e.alive());
    e.tick(&mut rng, &mut out);
    assert!(e.alive());
    e.tick(&mut rng, &mut out);
    assert!(!e.alive(), "removed once its life reaches three");
}

#[test]
fn a_critical_hit_starts_a_crit_emitter_that_spawns_now() {
    let mut s = session();
    let mut store = new_entity_store(8);
    spawn_zombie(&mut store, DVec3::new(0.0, 64.0, 0.0));
    let handled = s.apply_animate(animate(5, 1), &store);
    assert!(handled.is_none());
    assert_eq!(s.tracking_emitters.len(), 1);
    assert!(!s.particle_spawns.is_empty() && s.particle_spawns.len() <= 16);
    assert!(s
        .particle_spawns
        .iter()
        .all(|p| p.kind == Identifier::minecraft("crit")));
}

#[test]
fn a_magic_critical_hit_starts_an_enchanted_hit_emitter() {
    let mut s = session();
    let mut store = new_entity_store(8);
    spawn_zombie(&mut store, DVec3::new(0.0, 64.0, 0.0));
    s.apply_animate(animate(5, 2), &store);
    assert_eq!(
        s.tracking_emitters[0].kind,
        Identifier::minecraft("enchanted_hit")
    );
}

#[test]
fn an_entity_we_do_not_hold_starts_no_emitter() {
    let mut s = session();
    let store = new_entity_store(8);
    s.apply_animate(animate(5, 1), &store);
    assert!(s.tracking_emitters.is_empty());
    assert!(s.particle_spawns.is_empty());
}

#[test]
fn the_emitter_follows_its_entity_and_is_removed_after_three_ticks() {
    let mut s = session();
    let mut store = new_entity_store(8);
    spawn_zombie(&mut store, DVec3::new(0.0, 64.0, 0.0));
    s.apply_animate(animate(5, 1), &store);
    s.particle_spawns.clear();
    store.teleport(EntityId(5), DVec3::new(100.0, 64.0, 0.0), 0.0, 0.0, true);
    s.tick_tracking_emitters(&store);
    assert_eq!(s.tracking_emitters.len(), 1);
    // The spread is a quarter of the box's width each way: at most half a block for a box up to
    // two blocks wide (the store's box for a type the test registry does not name).
    assert!(s
        .particle_spawns
        .iter()
        .all(|p| (p.pos.x - 100.0).abs() <= 0.5));
    s.tick_tracking_emitters(&store);
    s.tick_tracking_emitters(&store);
    assert!(s.tracking_emitters.is_empty(), "removed at life 3");
}
