//! A dropped item's client-side motion: vanilla `ItemEntity.tick`.
//!
//! The server sends the item's velocity with `add_entity` and with `set_entity_motion`
//! (`Entity.recreateFromPacket` / `Entity.lerpMotion` set the entity's delta movement). Because
//! `ItemEntity.getMoveSimulationType()` is `SERVER_AND_CLIENT`, the client keeps simulating it
//! between position updates: each tick it applies gravity, moves by the velocity, then applies
//! the air drag and the ground bounce (`ItemEntity.tick` -> `applyGravity`, `move`,
//! `setDeltaMovement(...multiply)`). Haste stores that velocity on the entity
//! (`Entity.velocity`) and simulates it here.

use haste_core::view::FluidKind;
use haste_core::DVec3;

use crate::store::Entity;

/// `ItemEntity.getDefaultGravity()` (`Entity.getGravity`, items are never no-gravity).
pub const GRAVITY: f64 = 0.04;
/// `Entity.getAirDrag()` (the value `ItemEntity` inherits): a float, promoted when it multiplies
/// the double velocity (`Vec3.multiply`).
pub const AIR_DRAG: f32 = 0.98;
/// `Entity.getFluidHeight(tag) > 0.1F`: the surface height at which the item switches from
/// gravity to fluid movement.
const FLUID_HEIGHT_THRESHOLD: f32 = 0.1;
/// `ItemEntity.setUnderwaterMovement` / `setUnderLavaMovement`'s multipliers.
const WATER_DRAG: f32 = 0.99;
const LAVA_DRAG: f32 = 0.95;

/// The world `ItemEntity.tick` reads: the friction of the block below the item, the fluid it
/// stands in, and the block collision for its `move`. `haste-entity` has no world view in
/// `EntityStore::tick` (the client's `EntitySink::tick` passes none), so the store ticks with
/// [`NoWorld`]; [`EntityStore::tick_in_world`](crate::store::EntityStore::tick_in_world) takes a
/// real one. The client's `World` already implements `haste_core::view::BlockView`,
/// `CollisionShapes` and `BlockPhysics`, so wiring it is an adapter.
pub trait ItemWorld {
    /// `Block.getFriction` of `Entity.getBlockPosBelowThatAffectsMyMovement`
    /// (`ItemEntity.getOnPos(0.999999F)`): 0.6 by default, 0.98 ice, 0.989 blue ice, 0.8 slime.
    fn friction_below(&self, pos: DVec3) -> f32;
    /// `Entity.getFluidHeight(tag)`: the fluid surface inside the item's box, in blocks.
    fn fluid_height(&self, pos: DVec3, kind: FluidKind) -> f32;
    /// `Entity.move(MoverType.SELF, deltaMovement)` against block collision shapes: the movement
    /// actually possible, with `on_ground` set when a downward move was clipped from below
    /// (vanilla's `setOnGroundWithMovement`). The item's box is 0.25 x 0.25 (`ItemEntity`
    /// dimensions).
    fn move_with_collision(&self, pos: DVec3, delta: DVec3, on_ground: &mut bool) -> DVec3;
}

/// The world when none is available: air (friction 0.6), no fluid, and no block collision. While
/// `on_ground` (the state the server last reported) a downward move is clipped to the floor,
/// which stands in for vanilla's `move` collision until the client wires its `World` in through
/// [`EntityStore::tick_in_world`](crate::store::EntityStore::tick_in_world).
pub struct NoWorld;

impl ItemWorld for NoWorld {
    fn friction_below(&self, _pos: DVec3) -> f32 {
        0.6
    }

    fn fluid_height(&self, _pos: DVec3, _kind: FluidKind) -> f32 {
        0.0
    }

    fn move_with_collision(&self, _pos: DVec3, delta: DVec3, on_ground: &mut bool) -> DVec3 {
        if *on_ground && delta.y < 0.0 {
            DVec3::new(delta.x, 0.0, delta.z)
        } else {
            delta
        }
    }
}

/// One `ItemEntity.tick` of movement, in vanilla's order: water / lava / gravity, then the
/// on-ground skip, else the move, the drag and the ground bounce.
pub(crate) fn tick(e: &mut Entity, world: &dyn ItemWorld) {
    // Water first, then lava, else gravity (vanilla checks `isInWater()` / `isInLava()` with
    // `getFluidHeight(...) > 0.1F`; a fluid height above the threshold implies being in it).
    if world.fluid_height(e.pos, FluidKind::Water) > FLUID_HEIGHT_THRESHOLD {
        fluid_movement(e, WATER_DRAG);
    } else if world.fluid_height(e.pos, FluidKind::Lava) > FLUID_HEIGHT_THRESHOLD {
        fluid_movement(e, LAVA_DRAG);
    } else {
        // `applyGravity`: `delta.y -= getGravity()`.
        e.velocity.y -= GRAVITY;
    }

    // A resting item only moves every 4th tick: `onGround && !(horizontalDistanceSqr > 1.0E-5F)
    // && (tickCount + id) % 4 != 0` skips the move, the drag and the bounce entirely.
    let horizontal_sq = e.velocity.x * e.velocity.x + e.velocity.z * e.velocity.z;
    let resting = e.on_ground
        && horizontal_sq <= f64::from(1.0e-5_f32)
        && (e.tick_count as i64 + i64::from(e.id.0)) % 4 != 0;
    if resting {
        return;
    }

    // `move(MoverType.SELF, deltaMovement)`: the delta is clipped against blocks; the velocity is
    // left unchanged (`Entity.move` does not rewrite the delta movement).
    let delta = world.move_with_collision(e.pos, e.velocity, &mut e.on_ground);
    e.pos += delta;

    // `airDrag = getAirDrag(); groundFriction = airDrag; if (onGround) groundFriction *= friction`.
    let ground_friction = if e.on_ground {
        AIR_DRAG * world.friction_below(e.pos)
    } else {
        AIR_DRAG
    };
    e.velocity.x *= f64::from(ground_friction);
    e.velocity.y *= f64::from(AIR_DRAG);
    e.velocity.z *= f64::from(ground_friction);

    // On the ground with a downward velocity: bounce at half speed (`multiply(1.0, -0.5, 1.0)`).
    if e.on_ground && e.velocity.y < 0.0 {
        e.velocity.y *= -0.5;
    }
}

/// `ItemEntity.setFluidMovement`: horizontal drag, and a small lift while rising slowly.
fn fluid_movement(e: &mut Entity, multiplier: f32) {
    let m = f64::from(multiplier);
    e.velocity.x *= m;
    if e.velocity.y < f64::from(0.06_f32) {
        e.velocity.y += f64::from(5.0e-4_f32);
    }
    e.velocity.z *= m;
}
