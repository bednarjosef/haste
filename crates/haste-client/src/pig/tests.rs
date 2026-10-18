//! Tests for the pig's ridden movement against vanilla's own formulas, on the flat stone
//! platform of the evidence area.
use super::*;
use crate::item_based_steering::{boost_total_of, SteerEnv, DATA_BOOST_TIME};
use crate::travel::{GROUND_ACCELERATION, HORIZONTAL_DRAG};
use haste_core::entity::{MetadataEntry, MetadataValue};
use haste_physics::testing::{states, TestShapes, TestWorld};

/// The pig's box, from its `EntityType`'s `sized(0.9F, 0.9F)`.
const PIG: (f32, f32) = (0.9, 0.9);

/// A pig with its base attributes on the stone platform (flat, top at y = 0).
fn pig_at(pos: DVec3) -> Pig {
    Pig::new(
        pos,
        0.0,
        PIG,
        DEFAULT_MOVEMENT_SPEED,
        crate::horse::DEFAULT_GRAVITY,
    )
}

fn env<'a>(world: &'a TestWorld, shapes: &'a TestShapes) -> SteerEnv<'a> {
    SteerEnv {
        world,
        shapes,
        blocks: shapes,
        warm_blocks: None,
    }
}

/// The pig's unboosted forward acceleration of one ground tick: the attribute speed times
/// `Pig.getRiddenSpeed`'s own 0.225, times the ground acceleration of the platform's
/// slipperiness (the pig's input is a constant full forward, so the rider's 0.98 input scale
/// never applies).
fn ground_acceleration() -> f64 {
    let slip = 0.6f32;
    let speed = (DEFAULT_MOVEMENT_SPEED * 0.225) as f32;
    f64::from(speed * (GROUND_ACCELERATION / (slip * slip * slip)))
}

/// The pig's acceleration in the air (the flying speed: a tenth of the speed).
fn air_acceleration() -> f64 {
    DEFAULT_MOVEMENT_SPEED * 0.225 * 0.1
}

/// The horizontal drags of the platform: on the ground and in the air.
fn ground_drag() -> f64 {
    f64::from(0.6f32 * HORIZONTAL_DRAG)
}

/// The forward speed after `ticks` ticks of riding. The first tick's move carries no
/// downward motion (a fresh simulation starts where the server's move packets left the pig,
/// with a zero delta), so `onGround` drops for the second tick and that one accelerates and
/// drags with the air values; from the third tick the pig is grounded again.
fn walk_speed(ticks: u32) -> f64 {
    let (a, f) = (ground_acceleration(), ground_drag());
    let (a_air, f_air) = (air_acceleration(), f64::from(HORIZONTAL_DRAG));
    let mut d = 0.0;
    for t in 1..=ticks {
        let (accel, drag) = if t == 2 { (a_air, f_air) } else { (a, f) };
        d = (d + accel) * drag;
    }
    d
}

/// A pig that has settled onto the ground (three ticks: the second is the airborne one).
fn settled() -> Pig {
    let world = TestWorld::flat(0, states::STONE);
    let shapes = TestShapes::default();
    let e = env(&world, &shapes);
    let mut p = pig_at(DVec3::new(0.5, 0.0, 0.5));
    for _ in 0..3 {
        p.set_look((0.0, 0.0));
        p.tick(&e);
    }
    assert!(p.on_ground, "the settle should end on the ground");
    assert!((p.delta.z - walk_speed(3)).abs() < 1.0e-9);
    p
}

#[test]
fn a_ridden_pig_walks_forward_at_vanillas_speed() {
    let world = TestWorld::flat(0, states::STONE);
    let shapes = TestShapes::default();
    let e = env(&world, &shapes);
    let mut p = settled();
    for _ in 0..20 {
        p.set_look((0.0, 0.0));
        p.tick(&e);
    }
    let speed = walk_speed(23);
    assert!(
        (p.delta.z - speed).abs() < 1.0e-6,
        "forward speed: {} vs {}",
        p.delta.z,
        speed
    );
    assert!(p.on_ground, "the pig fell through the floor");
    assert!(p.delta.x.abs() < 1.0e-9, "x drifted: {}", p.delta.x);
    // The rider's look steers: the pig walks along its own facing.
    p.set_look((90.0, 0.0));
    p.tick(&e);
    assert_eq!(p.yaw, 90.0);
    assert!(p.delta.x < 0.0, "walked west: {}", p.delta.x);
}

#[test]
fn a_boosted_pig_speeds_up_and_back_down() {
    let world = TestWorld::flat(0, states::STONE);
    let shapes = TestShapes::default();
    let e = env(&world, &shapes);
    let settled_ticks = 3;
    let mut p = settled();
    // The metadata update that carries the boost total starts it (ItemBasedSteering.onSynced).
    let total = 140;
    p.on_synced_boost(total);
    assert!(p.boosting());
    // The boost factor's series: 1 + 1.15 sin(boostTime / total * pi), the time counting from
    // the tick after the sync (tickBoost runs inside tickRidden, before the travel reads it).
    // The factor scales the acceleration the tick adds, not the speed the pig already has.
    let mut d = walk_speed(settled_ticks);
    for t in 1..=3 {
        p.set_look((0.0, 0.0));
        p.tick(&e);
        let factor = 1.0 + 1.15 * (t as f64 / total as f64 * std::f64::consts::PI).sin();
        d = (d + ground_acceleration() * factor) * ground_drag();
        assert!(
            (p.delta.z - d).abs() < 1.0e-6,
            "tick {t}: {} vs {}",
            p.delta.z,
            d
        );
    }
    // The boost ends once its time passes the total.
    for _ in 0..(total + 2) {
        p.set_look((0.0, 0.0));
        p.tick(&e);
    }
    assert!(!p.boosting(), "the boost ended");
}

#[test]
fn the_boost_total_comes_from_the_metadata_update() {
    let boost = [MetadataEntry {
        index: DATA_BOOST_TIME,
        value: MetadataValue::VarInt(317),
    }];
    assert_eq!(boost_total_of(&boost), Some(317));
    // Other entries carry no boost.
    let other = [MetadataEntry {
        index: 19,
        value: MetadataValue::VarInt(1),
    }];
    assert_eq!(boost_total_of(&other), None);
    assert_eq!(boost_total_of(&[]), None);
}

#[test]
fn the_server_correction_snaps_the_pig() {
    let world = TestWorld::flat(0, states::STONE);
    let shapes = TestShapes::default();
    let e = env(&world, &shapes);
    let mut p = settled();
    p.set_look((0.0, 0.0));
    p.tick(&e);
    p.correct(DVec3::new(10.0, 0.0, 10.0), 180.0);
    assert_eq!(p.pos, DVec3::new(10.0, 0.0, 10.0));
    assert_eq!(p.yaw, 180.0);
}

/// `ItemBasedSteering.tickBoost` compares the time before its post-increment, so a boost of
/// `total` runs `total + 1` ticks after the sync and ends on the next.
#[test]
fn the_boost_ends_the_tick_after_its_time_passes_the_total() {
    let world = TestWorld::flat(0, states::STONE);
    let shapes = TestShapes::default();
    let e = env(&world, &shapes);
    let mut p = settled();
    p.on_synced_boost(10);
    for _ in 0..11 {
        p.set_look((0.0, 0.0));
        p.tick(&e);
    }
    assert!(p.boosting(), "still boosting after total + 1 ticks");
    p.tick(&e);
    assert!(!p.boosting(), "ended on the next tick");
}
