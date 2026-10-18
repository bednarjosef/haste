//! Tests for the strider's ridden speed against vanilla's `Strider.getRiddenSpeed`, its stand
//! on lava and its warm/cold decision.
use super::*;
use crate::travel::{GROUND_ACCELERATION, HORIZONTAL_DRAG};
use haste_core::entity::{MetadataEntry, MetadataValue};
use haste_physics::testing::{states, TestShapes, TestWorld};

/// The strider's box, from its `EntityType`'s `sized(0.9F, 1.7F)`.
const STRIDER: (f32, f32) = (0.9, 1.7);

/// `#strider_warm_blocks` test fake: only the test's warm state is in it.
struct Warm(bool);

impl crate::item_based_steering::WarmBlocks for Warm {
    fn is_strider_warm(&self, s: haste_core::BlockStateId) -> bool {
        self.0 && s == states::LAVA_SOURCE
    }
}

fn env_for<'a>(
    world: &'a TestWorld,
    shapes: &'a TestShapes,
    warm: Option<&'a dyn crate::item_based_steering::WarmBlocks>,
) -> SteerEnv<'a> {
    SteerEnv {
        world,
        shapes,
        blocks: shapes,
        warm_blocks: warm,
    }
}

fn walk(suffocating: bool, ticks: u32) -> Strider {
    let world = TestWorld::flat(0, states::STONE);
    let shapes = TestShapes::default();
    let env = env_for(&world, &shapes, None);
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
    // Settle on the lava pool (the second tick is the airborne one, as for the pig), then one
    // grounded tick on the surface: the speed gained is the acceleration, dragged. The
    // warm/cold flag is the client's own decision (inside lava -> warm).
    let mut world = TestWorld::flat(0, states::STONE);
    for x in -60..61 {
        for z in -60..61 {
            world.set(x, -1, z, states::LAVA_SOURCE);
        }
    }
    let shapes = TestShapes::default();
    let warm = Warm(true);
    let env = env_for(&world, &shapes, Some(&warm));
    let run = |ticks: u32| {
        let mut s = Strider::new(
            glam::DVec3::new(0.5, 0.0, 0.5),
            0.0,
            STRIDER,
            DEFAULT_MOVEMENT_SPEED,
            crate::horse::DEFAULT_GRAVITY,
        );
        for _ in 0..ticks {
            s.set_look((0.0, 0.0));
            s.tick(&env);
        }
        s
    };
    let warm_before = run(3).delta.z;
    let after_warm = run(4);
    let warm_z = after_warm.delta.z;
    let drag = f64::from(0.6f32 * HORIZONTAL_DRAG);
    let expect = (warm_before + ground_acceleration(0.55)) * drag;
    assert!(
        (warm_z - expect).abs() < 1.0e-9,
        "warm {warm_z} vs {expect}"
    );
    assert!(!after_warm.suffocating);
    // The same strider on cold stone walks at the suffocating modifier (the client's own
    // decision: no warm block in or under the feet, no lava touch).
    let cold_before = walk(true, 3).delta.z;
    let after_cold = walk(true, 4);
    let cold_z = after_cold.delta.z;
    let expect = (cold_before + ground_acceleration(0.35)) * drag;
    assert!(
        (cold_z - expect).abs() < 1.0e-9,
        "cold {cold_z} vs {expect}"
    );
    assert!(after_cold.suffocating);
    assert!(cold_z < warm_z);
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

/// A strider dropped onto a lava pool comes to rest at the fluid's surface, vanilla's
/// `floatStrider` + `LiquidBlock.getCollisionShape`: standing on the liquid column, its feet
/// at the lava cell's floor plus half a block (the `Block.column(16, 0, 8)` top).
#[test]
fn a_strider_stands_on_a_lava_pool_at_the_surface_height() {
    // A lava pool on stone, its surface cell at y = -1 (a source, air above), far enough that
    // the walking strider (full forward, ~0.08 blocks a tick) does not reach its edge.
    let mut world = TestWorld::flat(0, states::STONE);
    for x in -60..61 {
        for z in -60..61 {
            world.set(x, -1, z, states::LAVA_SOURCE);
        }
    }
    let shapes = TestShapes::default();
    let warm = Warm(true);
    let env = env_for(&world, &shapes, Some(&warm));
    let mut s = Strider::new(
        glam::DVec3::new(0.5, 0.5, 0.5),
        0.0,
        STRIDER,
        DEFAULT_MOVEMENT_SPEED,
        crate::horse::DEFAULT_GRAVITY,
    );
    for _ in 0..100 {
        s.set_look((0.0, 0.0));
        s.tick(&env);
    }
    // `floatStrider`'s stand branch: on the ground, at the column's top (y = -0.5), never
    // sinking below it, and warm (inside lava, so not suffocating).
    assert!(s.on_ground, "not standing: y = {}", s.pos.y);
    assert!((s.pos.y - -0.5).abs() < 1.0e-9, "y = {}", s.pos.y);
    assert!(!s.suffocating);
}

/// The same strider walking off the lava onto stone is cold the same tick it leaves, not a
/// server round-trip later (the client's own `setSuffocating`).
#[test]
fn the_speed_switches_when_the_strider_leaves_the_lava() {
    // Stone with top surface y = 0 and a lava strip sunk into it: the strip's cells (z = -4..5)
    // hold lava at y = -1 (surface top at y = -0.5, half a block below the stone), so the
    // strider steps down onto it, walks across and climbs back out onto the stone.
    let mut world = TestWorld::flat(0, states::STONE);
    for z in -4..6 {
        for x in -4..5 {
            world.set(x, -1, z, states::LAVA_SOURCE);
        }
    }
    let shapes = TestShapes::default();
    let warm = Warm(true);
    let env = env_for(&world, &shapes, Some(&warm));
    let mut s = Strider::new(
        glam::DVec3::new(0.5, 0.0, -8.5),
        0.0,
        STRIDER,
        DEFAULT_MOVEMENT_SPEED,
        crate::horse::DEFAULT_GRAVITY,
    );
    let mut warm_ticks = 0;
    let mut cold_ticks = 0;
    for _ in 0..300 {
        s.set_look((0.0, 0.0));
        s.tick(&env);
        // The flag is decided from the position at the tick's start (vanilla `Strider.tick`
        // runs `setSuffocating` before the travel moves it), so the window leaves one tick
        // of walking (~0.1 blocks) at each edge: warm over the strip (cells z = -4..5),
        // cold on the stone past it (cell z = 6).
        if s.pos.z > -3.5 && s.pos.z < 5.5 {
            assert!(!s.suffocating, "cold while over lava at z {}", s.pos.z);
            warm_ticks += 1;
        } else if s.pos.z >= 6.5 {
            assert!(s.suffocating, "warm past the lava at z {}", s.pos.z);
            cold_ticks += 1;
        }
    }
    assert!(warm_ticks > 0, "never walked over the lava");
    assert!(cold_ticks > 0, "never left the lava");
}
