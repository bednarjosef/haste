//! Vanilla `LivingEntity.travel` -> `travelInAir` for a mob whose controlling passenger is the
//! local player: the input acceleration, the swept move, gravity and the drags. Shared by the
//! ridden animals this client steers (the horse family, the pig, the strider); each module owns
//! its own input shaping and speed (`getRiddenInput`, `getRiddenSpeed`).
use glam::DVec3;
use haste_core::view::{BlockPhysics, BlockView, CollisionShapes};
use haste_core::Aabb;
use haste_physics::collision::collide_with_step;

/// Vanilla `LivingEntity.travelInAir`: the air drag applied to the horizontal velocity.
pub const HORIZONTAL_DRAG: f32 = 0.91;
/// Vanilla `LivingEntity.getFrictionInfluencedSpeed`: the ground-acceleration constant.
pub const GROUND_ACCELERATION: f32 = 0.216_000_02;
/// The slipperiness above which the ground acceleration formula applies (vanilla
/// `getFrictionInfluencedSpeed`: `blockFriction > 0.6`).
const SLIPPERY_THRESHOLD: f32 = 0.6;

/// What the shared travel needs from the steered animal: its simulated state (one flat struct
/// per animal module) and its world.
pub trait RiddenAnimal {
    /// Feet position.
    fn pos(&self) -> DVec3;
    /// Blocks per tick (`deltaMovement`).
    fn delta(&self) -> DVec3;
    fn set_delta(&mut self, d: DVec3);
    fn add_pos(&mut self, d: DVec3);
    /// The entity box at the current position.
    fn aabb(&self) -> Aabb;
    /// The `getRiddenSpeed` this tick (already includes any boost).
    fn speed(&self) -> f32;
    /// The `Attributes.STEP_HEIGHT` of this animal.
    fn step_height(&self) -> f64;
    /// `Attributes.GRAVITY` (the animal's own, synced or the default 0.08).
    fn gravity(&self) -> f64;
    fn on_ground(&self) -> bool;
    fn set_on_ground(&mut self, g: bool);
    fn set_collisions(&mut self, horizontal: bool, vertical: bool);
}

/// Vanilla `Entity.getBlockPosBelowThatAffectsMyMovement` = `getOnPos(0.500001)`: the block
/// half a block below the feet.
fn ground_block(pos: DVec3, world: &dyn BlockView) -> haste_core::BlockStateId {
    world.block_state(haste_core::BlockPos::from_dvec3(
        pos - DVec3::new(0.0, 0.500_001, 0.0),
    ))
}

/// One tick of the shared `travelInAir` for `a` with the shaped input `(strafe, forward)` and
/// its facing `yaw`: the ground acceleration from the block's slipperiness (in the air the
/// flying speed, a tenth of the speed — a player controls it), the move, then gravity and the
/// drags. Vanilla's `restituteMovementAfterCollisions` with a zero bounciness zeroes each
/// collided component.
pub fn travel_in_air(
    a: &mut impl RiddenAnimal,
    input: (f32, f32),
    yaw: f32,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    blocks: &dyn BlockPhysics,
) {
    let on_ground = a.on_ground();
    let slip = if on_ground {
        blocks.friction(ground_block(a.pos(), world))
    } else {
        1.0
    };
    let speed = a.speed();
    let acceleration = if on_ground && slip > SLIPPERY_THRESHOLD {
        speed * (GROUND_ACCELERATION / (slip * slip * slip))
    } else if on_ground {
        speed
    } else {
        // getFlyingSpeed: a player controls the animal, so its own speed x 0.1.
        speed * 0.1
    };
    move_relative(a, acceleration, input, yaw);
    let motion = a.delta();
    let moved = collide_with_step(a.aabb(), motion, a.step_height(), on_ground, world, shapes);
    a.add_pos(moved);
    let x_collision = moved.x != motion.x;
    let z_collision = moved.z != motion.z;
    let vertical_collision = moved.y != motion.y;
    a.set_collisions(x_collision || z_collision, vertical_collision);
    a.set_on_ground(vertical_collision && motion.y < 0.0);
    let mut d = a.delta();
    if x_collision {
        d.x = 0.0;
    }
    if z_collision {
        d.z = 0.0;
    }
    if vertical_collision {
        d.y = 0.0;
    }
    d.y -= a.gravity();
    d.y *= f64::from(VERTICAL_DRAG);
    let drag = f64::from(slip * HORIZONTAL_DRAG);
    d.x *= drag;
    d.z *= drag;
    a.set_delta(d);
}

/// Vanilla `Entity.moveRelative` -> `getInputVector`: the input rotated by the yaw, its length
/// normalised past 1.
fn move_relative(a: &mut impl RiddenAnimal, speed: f32, input: (f32, f32), yaw: f32) {
    let (x, z) = (f64::from(input.0), f64::from(input.1));
    let length = x * x + z * z;
    if length < 1.0e-7 {
        return;
    }
    let (mut x, mut z) = (x, z);
    if length > 1.0 {
        let l = length.sqrt();
        x /= l;
        z /= l;
    }
    x *= f64::from(speed);
    z *= f64::from(speed);
    let r = f64::from(yaw) * (std::f64::consts::PI / 180.0);
    let (sin, cos) = (r.sin(), r.cos());
    let mut d = a.delta();
    d.x += x * cos - z * sin;
    d.z += z * cos + x * sin;
    a.set_delta(d);
}

/// Vanilla `LivingEntity.travelInAir`'s vertical drag, for the callers that advance their own
/// jump arcs alongside.
pub const VERTICAL_DRAG: f32 = 0.98;
