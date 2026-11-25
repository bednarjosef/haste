//! Creative flight. Documented: ~10.92 m/s flying, ~21.6 m/s sprint-flying (wiki "Flying"; the
//! "Transportation" page rounds to 11.0 / 22.0). Horizontal drag 0.91 is the air drag (wiki
//! "Entity"); with acceleration 0.05 x 0.98 that gives 0.544 b/t = 10.89 m/s, sprint (x2) 21.78.
//! The vertical impulse and the vertical restore factor come from the 26.3 client:
//! `LocalPlayer.aiStep` (three times the flying speed per tick of jump/sneak) and
//! `Player.travel` (the y velocity returns to its tick-start value times 0.6).
use haste_core::input::InputState;
use haste_core::view::{BlockView, CollisionShapes};

use crate::movement::{accelerate, move_entity, Impulse, HORIZONTAL_DRAG};
use crate::{BlockPhysics, PlayerAttributes, PlayerState};

/// Sprint-flying doubles the flying speed (wiki "Flying": 21.6 vs 10.92 m/s, ~2x).
pub const SPRINT_FLY_MULTIPLIER: f32 = 2.0;
/// Vertical impulse per tick while holding jump/sneak, in units of flying speed (vanilla
/// `LocalPlayer.aiStep`).
pub const VERTICAL_FLY_FACTOR: f64 = 3.0;
/// Vertical restore factor: the y velocity at the end of the travel is the tick-start y velocity
/// times this (vanilla `Player.travel`: `originalMovementY * 0.6`).
pub const VERTICAL_FLY_DRAG: f64 = 0.6;

/// vanilla `LocalPlayer.aiStep`: while flying with a controlled camera, jump/sneak add three
/// times the flying speed to the y velocity, before the travel.
pub(crate) fn apply_vertical_impulse(
    state: &mut PlayerState,
    input: &InputState,
    attrs: &PlayerAttributes,
) {
    let vertical = attrs.flying_speed as f64 * VERTICAL_FLY_FACTOR;
    if input.jump {
        state.velocity.y += vertical;
    }
    if input.sneak {
        state.velocity.y -= vertical;
    }
}

pub(crate) fn travel(
    state: &mut PlayerState,
    input: &InputState,
    imp: Impulse,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    blocks: &dyn BlockPhysics,
) {
    apply_vertical_impulse(state, input, attrs);
    let speed = attrs.flying_speed
        * if state.sprinting {
            SPRINT_FLY_MULTIPLIER
        } else {
            1.0
        };
    accelerate(state, imp, speed);
    let motion = state.velocity;
    move_entity(state, attrs, motion, world, shapes, blocks);
    let drag = HORIZONTAL_DRAG as f64;
    state.velocity.x *= drag;
    state.velocity.z *= drag;
    state.velocity.y *= VERTICAL_FLY_DRAG;
    // Landing ends creative flight, except for spectators (vanilla `LocalPlayer.aiStep`:
    // `onGround() && abilities.flying && !gameMode.isSpectator()`).
    if state.on_ground && !state.spectating {
        state.flying = false;
    }
}
