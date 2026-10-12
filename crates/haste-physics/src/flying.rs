//! Creative flight. Documented: ~10.92 m/s flying, ~21.6 m/s sprint-flying (wiki "Flying"; the
//! "Transportation" page rounds to 11.0 / 22.0). Horizontal drag 0.91 is the air drag (wiki
//! "Entity"); with acceleration 0.05 x 0.98 that gives 0.544 b/t = 10.89 m/s, sprint (x2) 21.78.
//! Vertical flight speed and drag are not documented: UNVERIFIED constants below.
use haste_core::input::InputState;
use haste_core::view::{BlockView, CollisionShapes};

use crate::movement::{accelerate, move_entity, Impulse, HORIZONTAL_DRAG};
use crate::{BlockPhysics, PlayerAttributes, PlayerState};

/// Sprint-flying doubles the flying speed (wiki "Flying": 21.6 vs 10.92 m/s, ~2x).
pub const SPRINT_FLY_MULTIPLIER: f32 = 2.0;
/// UNVERIFIED: vertical impulse per tick while holding jump/sneak, in units of flying speed.
pub const VERTICAL_FLY_FACTOR: f64 = 3.0;
/// UNVERIFIED: vertical drag while flying.
pub const VERTICAL_FLY_DRAG: f64 = 0.6;

pub(crate) fn travel(
    state: &mut PlayerState,
    input: &InputState,
    imp: Impulse,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    _blocks: &dyn BlockPhysics,
) {
    let vertical = attrs.flying_speed as f64 * VERTICAL_FLY_FACTOR;
    if input.jump {
        state.velocity.y += vertical;
    }
    if input.sneak {
        state.velocity.y -= vertical;
    }
    let speed = attrs.flying_speed * if state.sprinting { SPRINT_FLY_MULTIPLIER } else { 1.0 };
    accelerate(state, imp, speed);
    let motion = state.velocity;
    move_entity(state, attrs, motion, world, shapes);
    let drag = HORIZONTAL_DRAG as f64;
    state.velocity.x *= drag;
    state.velocity.z *= drag;
    state.velocity.y *= VERTICAL_FLY_DRAG;
    // Landing ends creative flight (observed behaviour; assumption about the exact condition).
    if state.on_ground {
        state.flying = false;
    }
}
