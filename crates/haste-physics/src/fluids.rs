//! Water and lava: contact detection, drag, buoyancy/sinking, swimming up.
//!
//! What is documented: horizontal steady speeds in water and lava (wiki "Walking" table: 1.960 and
//! 0.784 m/s, 0.576 / 0.490 on soul sand) and sprint-swimming (wiki "Swimming": 3.918 m/s). The
//! drags below are derived from those numbers. The vertical behaviour (sinking, swimming up) is not
//! described on any public page we found: those constants are marked UNVERIFIED and must be
//! settled by traces.
use haste_core::input::InputState;
use haste_core::view::{BlockView, CollisionShapes};
use haste_core::{BlockPos, DVec3};

use crate::movement::{accelerate, apply_speed_factor, move_entity, Impulse};
use crate::{BlockPhysics, FluidKind, PlayerAttributes, PlayerState};

/// Acceleration in fluids. 0.02 (as in air); with drag 0.8 it gives 0.098 b/t = 1.960 m/s.
pub const FLUID_ACCELERATION: f32 = 0.02;
/// Water drag on all axes: derived from 1.960 m/s = 20 x 0.0196 / (1 - 0.8).
pub const WATER_DRAG: f64 = 0.8;
/// Horizontal water drag while sprint-swimming: derived from 3.918 m/s ~ 20 x 0.0196 / (1 - 0.9).
pub const WATER_DRAG_SPRINTING: f64 = 0.9;
/// Lava drag on all axes: derived from 0.784 m/s = 20 x 0.0196 / (1 - 0.5).
pub const LAVA_DRAG: f64 = 0.5;
/// UNVERIFIED: gravity in water is `gravity / 16`.
pub const WATER_GRAVITY_DIVISOR: f64 = 16.0;
/// UNVERIFIED: gravity in lava is `gravity / 4`.
pub const LAVA_GRAVITY_DIVISOR: f64 = 4.0;
/// UNVERIFIED: holding jump in a fluid adds this to vertical velocity each tick.
pub const SWIM_UP: f64 = 0.04;
/// UNVERIFIED: holding sneak in water subtracts this (wiki "Swimming": sneaking sinks faster).
pub const SWIM_DOWN: f64 = 0.04;
/// Box shrink before testing fluid contact (assumption).
const CONTACT_SHRINK: f64 = 0.001;

/// Updates `in_water`/`in_lava` (water wins) and returns whether the eyes are under water.
/// Contact = the slightly shrunk box overlaps a fluid cell below its surface. The surface is
/// `FluidInfo::height`, or the full cell when the same fluid is directly above (wiki "Water").
pub(crate) fn update_fluid_state(
    state: &mut PlayerState,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    blocks: &dyn BlockPhysics,
) -> bool {
    let b = state.aabb(attrs);
    let eye_y = state.pos.y + attrs.eye_height_for(state.pose);
    let (min, max) = (
        b.min + DVec3::splat(CONTACT_SHRINK),
        b.max - DVec3::splat(CONTACT_SHRINK),
    );
    let (mut water, mut lava, mut eyes) = (false, false, false);
    for x in min.x.floor() as i32..=max.x.floor() as i32 {
        for z in min.z.floor() as i32..=max.z.floor() as i32 {
            for y in min.y.floor() as i32..=max.y.max(eye_y).floor() as i32 {
                let p = BlockPos::new(x, y, z);
                let Some(f) = blocks.fluid(world.block_state(p)) else {
                    continue;
                };
                let full = blocks
                    .fluid(world.block_state(p.add(0, 1, 0)))
                    .is_some_and(|a| a.kind == f.kind);
                let surface = y as f64 + if full { 1.0 } else { f.height as f64 };
                if f.kind == FluidKind::Water
                    && eye_y >= y as f64
                    && eye_y < surface
                    && x == state.pos.x.floor() as i32
                    && z == state.pos.z.floor() as i32
                {
                    eyes = true;
                }
                if (y as f64) < max.y && surface > min.y {
                    match f.kind {
                        FluidKind::Water => water = true,
                        FluidKind::Lava => lava = true,
                    }
                }
            }
        }
    }
    state.in_water = water;
    state.in_lava = lava && !water;
    if water {
        state.fall_distance = 0.0;
    }
    eyes && water
}

pub(crate) fn swim_up(state: &mut PlayerState) {
    state.velocity.y += SWIM_UP;
}

pub(crate) fn travel_water(
    state: &mut PlayerState,
    input: &InputState,
    imp: Impulse,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    blocks: &dyn BlockPhysics,
) {
    if input.sneak {
        state.velocity.y -= SWIM_DOWN;
    }
    let h_drag = if state.sprinting {
        WATER_DRAG_SPRINTING
    } else {
        WATER_DRAG
    };
    accelerate(state, imp, FLUID_ACCELERATION);
    let motion = state.velocity;
    move_entity(state, attrs, motion, world, shapes, blocks);
    apply_speed_factor(state, world, blocks);
    state.velocity.x *= h_drag;
    state.velocity.y *= WATER_DRAG;
    state.velocity.z *= h_drag;
    if !state.swimming {
        state.velocity.y -= attrs.gravity / WATER_GRAVITY_DIVISOR;
    }
}

pub(crate) fn travel_lava(
    state: &mut PlayerState,
    _input: &InputState,
    imp: Impulse,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    blocks: &dyn BlockPhysics,
) {
    accelerate(state, imp, FLUID_ACCELERATION);
    let motion = state.velocity;
    move_entity(state, attrs, motion, world, shapes, blocks);
    apply_speed_factor(state, world, blocks);
    state.velocity *= LAVA_DRAG;
    state.velocity.y -= attrs.gravity / LAVA_GRAVITY_DIVISOR;
}
