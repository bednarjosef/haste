//! Tests for the strider's ridden speed against vanilla's `Strider.getRiddenSpeed`.
use super::*;
use crate::travel::{GROUND_ACCELERATION, HORIZONTAL_DRAG};
use haste_core::entity::{MetadataEntry, MetadataValue};
use haste_physics::testing::{states, TestShapes, TestWorld};

/// The strider's box, from its `EntityType`'s `sized(0.9F, 1.7F)`.
const STRIDER: (f32, f32) = (0.9, 1.7);

fn walk(suffocating: bool, ticks: u32) -> Strider {
    let world = TestWorld::flat(0, states::STONE);
    let shapes = TestShapes::default();
    let env = SteerEnv {
        world: &world,
        shapes: &shapes,
        blocks: &shapes,
    };
    let mut s = Strider::new(
        glam::DVec3::new(0.5, 0.0, 0.5),
        0.0,
        STRIDER,
        DEFAULT_MOVEMENT_SPEED,
        crate::horse::DEFAULT_GRAVITY,
    );
    s.suffocating = suffocating;
    for _ in 0..ticks {
        s.set_look((0.0, 0.0));
        s.tick(&env);
    }
    s
}

/// One grounded tick's acceleration on stone: the attribute speed times the steering factor
/// (0.55F warm, 0.35F suffocating), times the ground acceleration of slipperiness 0.6.
fn ground_acceleration(factor: f32) -> f64 {
    let slip = 0.6f32;
    let speed = (DEFAULT_MOVEMENT_SPEED * f64::from(factor)) as f32;
    f64::from(speed * (GROUND_ACCELERATION / (slip * slip * slip)))
}

#[test]
fn a_warm_strider_walks_at_the_steering_modifier_and_a_cold_one_slower() {
    // Settle (the second tick is the airborne one, as for the pig), then one grounded tick:
    // the speed gained is the acceleration, dragged.
    let warm_before = walk(false, 3).delta.z;
    let warm = walk(false, 4).delta.z;
    let drag = f64::from(0.6f32 * HORIZONTAL_DRAG);
    let expect = (warm_before + ground_acceleration(0.55)) * drag;
    assert!((warm - expect).abs() < 1.0e-9, "warm {warm} vs {expect}");
    let cold_before = walk(true, 3).delta.z;
    let cold = walk(true, 4).delta.z;
    let expect = (cold_before + ground_acceleration(0.35)) * drag;
    assert!((cold - expect).abs() < 1.0e-9, "cold {cold} vs {expect}");
    assert!(cold < warm);
}

#[test]
fn the_suffocating_flag_comes_from_the_metadata_update() {
    let mut s = walk(false, 0);
    s.on_metadata(&[MetadataEntry {
        index: DATA_SUFFOCATING,
        value: MetadataValue::Boolean(true),
    }]);
    assert!(s.suffocating);
    assert!(is_strider_type("strider"));
    assert!(!is_strider_type("pig"));
}
