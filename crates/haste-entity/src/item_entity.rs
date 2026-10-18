//! A dropped item's client-side motion: vanilla `ItemEntity.tick`.
//!
//! The server sends the item's velocity with `add_entity` and with `set_entity_motion`
//! (`Entity.recreateFromPacket` / `Entity.lerpMotion` set the entity's delta movement). Because
//! `ItemEntity.getMoveSimulationType()` is `SERVER_AND_CLIENT`, the client keeps simulating it
//! between position updates: each tick it applies gravity, moves by the velocity, then applies
//! the air drag (`ItemEntity.tick` -> `applyGravity`, `move`, `setDeltaMovement(...multiply)`).
//! Haste stores that velocity on the entity (`Entity.velocity`) and simulates it here.

use crate::store::Entity;

/// `ItemEntity.getDefaultGravity()`.
pub const GRAVITY: f64 = 0.04;
/// `Entity.getAirDrag()` (the value `ItemEntity` inherits).
pub const AIR_DRAG: f64 = 0.98;

/// One `ItemEntity.tick` of movement, in vanilla's order: gravity, move, drag. Haste has no block
/// collision for remote entities yet, so the move is the raw velocity (vanilla's `move` returns
/// the delta unchanged in the air); the next position packet corrects any difference. When the
/// server reports the item on the ground, vanilla's `move` stops the downward motion at the
/// floor, so the simulation holds it there instead of letting it sink through (a resting item's
/// position is not resent by the server). The block friction and fluid movement vanilla reads
/// from the world are not available here, so the air drag stands in for the horizontal drag.
pub(crate) fn tick(e: &mut Entity) {
    // applyGravity: delta.y -= gravity (gravity is 0 when the entity is no-gravity, which Haste
    // does not track for items; vanilla's items are always affected).
    e.velocity.y -= GRAVITY;
    // move(MoverType.SELF, deltaMovement): position += delta, except that a downward move on the
    // ground is stopped at the floor (vanilla's collision).
    if e.on_ground && e.velocity.y < 0.0 {
        e.pos.x += e.velocity.x;
        e.pos.z += e.velocity.z;
        e.velocity.y = 0.0;
    } else {
        e.pos += e.velocity;
    }
    // ItemEntity.tick's drag: horizontal by ground friction, vertical by air drag. Without a
    // block lookup both use the air drag.
    e.velocity.x *= AIR_DRAG;
    e.velocity.y *= AIR_DRAG;
    e.velocity.z *= AIR_DRAG;
}
