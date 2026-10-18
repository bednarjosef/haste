//! Vanilla `LivingEntity.travel` -> `travelInAir` for a mob whose controlling passenger is the
//! local player: the input acceleration, the swept move, gravity and the drags. Shared by the
//! ridden animals this client steers (the horse family, the pig, the strider); each module owns
//! its own input shaping and speed (`getRiddenInput`, `getRiddenSpeed`).
use glam::DVec3;
use haste_core::view::{BlockPhysics, BlockView, CollisionShapes, FluidKind};
use haste_core::{Aabb, BlockPos};
use haste_physics::collision::{collide_with_step, COLLISION_EPSILON};

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
    /// The fluid the animal walks on instead of through, as vanilla `LivingEntity.canStandOnFluid`
    /// answers it (`Strider`: lava). `None` (the default, vanilla's `false`) falls through fluids.
    fn stands_on_fluid(&self) -> Option<FluidKind> {
        None
    }
}

/// How many fluid surface cells one sweep can stand on: the animal's box spans at most a 3x3
/// footprint at these speeds, and only surface cells count (a cell with the same fluid above
/// is a column the animal is inside, never a floor), so 16 is beyond reach; a longer fall
/// simply stops collecting past it.
const FLUID_FLOOR_CELLS: usize = 16;

/// The fluid surface boxes the sweep must treat as ground, vanilla
/// `LiquidBlock.getCollisionShape` for every fluid cell the swept region touches, with the
/// moving animal as the collision context:
/// - only a *source* cell (the `LEVEL == 0` test; our fluid heights make a source surface
///   8/9, flowing lower, and a falling cell always has the same fluid above, caught next),
/// - with no same fluid directly above (`EntityCollisionContext.canStandOnFluid`: standing on
///   the surface, not inside a column),
/// - and only while the animal's feet are already above the cell's liquid column
///   (`EntityCollisionContext.isAbove`): the column is half a block
///   (`LivingEntity.getLiquidCollisionShape`, `Block.column(16, 0, 8)`).
///
/// Returns the boxes (world space) and how many filled.
fn fluid_floors(
    a: &dyn RiddenAnimal,
    movement: DVec3,
    step_height: f64,
    world: &dyn BlockView,
    blocks: &dyn BlockPhysics,
    stands_on: FluidKind,
) -> ([Aabb; FLUID_FLOOR_CELLS], usize) {
    let mut floors = [Aabb {
        min: DVec3::ZERO,
        max: DVec3::ZERO,
    }; FLUID_FLOOR_CELLS];
    let mut n = 0;
    // The region the sweep collides for (the box stretched over the movement, raised by the
    // step height), like vanilla's `expandTowards` collider gathering.
    let mut region = stretch(&a.aabb(), movement);
    if step_height > 0.0 {
        region.max.y = region.max.y.max(a.aabb().max.y + step_height);
    }
    let (x0, x1) = (
        (region.min.x - COLLISION_EPSILON).floor() as i32,
        (region.max.x + COLLISION_EPSILON).floor() as i32,
    );
    let (z0, z1) = (
        (region.min.z - COLLISION_EPSILON).floor() as i32,
        (region.max.z + COLLISION_EPSILON).floor() as i32,
    );
    let (y0, y1) = (
        (region.min.y - COLLISION_EPSILON).floor() as i32,
        (region.max.y + COLLISION_EPSILON).floor() as i32,
    );
    let bottom = a.pos().y;
    for y in y0..=y1 {
        for x in x0..=x1 {
            for z in z0..=z1 {
                let pos = BlockPos::new(x, y, z);
                let state = world.block_state(pos);
                let Some(fluid) = blocks.fluid(state) else {
                    continue;
                };
                if fluid.kind != stands_on {
                    continue;
                }
                // `state.getValue(LEVEL) != 0`: only a source collides.
                if (fluid.height - 8.0 / 9.0).abs() > 1.0e-6 {
                    continue;
                }
                // `canStandOnFluid(fluidAbove, fluid)`: the same fluid above means a column.
                let above = world.block_state(pos.add(0, 1, 0));
                if blocks.fluid(above).is_some_and(|f| f.kind == stands_on) {
                    continue;
                }
                // `isAbove(shape, pos, true)`: the feet over the column's half-block top.
                if bottom <= y as f64 + 0.5 - 1.0e-5 {
                    continue;
                }
                let o = pos.as_dvec3();
                floors[n] = Aabb {
                    min: o,
                    max: DVec3::new(o.x + 1.0, o.y + 0.5, o.z + 1.0),
                };
                n += 1;
                if n == FLUID_FLOOR_CELLS {
                    return (floors, n);
                }
            }
        }
    }
    (floors, n)
}

/// The box stretched over a movement (the region a sweep gathers colliders from).
fn stretch(aabb: &Aabb, m: DVec3) -> Aabb {
    Aabb {
        min: aabb.min + m.min(DVec3::ZERO),
        max: aabb.max + m.max(DVec3::ZERO),
    }
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
    // The fluid the animal walks on gives the surface a collision box (vanilla
    // `LiquidBlock.getCollisionShape`, part of the sweep's collider gathering like any block
    // shape); without one the plain sweep runs.
    let moved = match a.stands_on_fluid() {
        Some(stands_on) => {
            let (floors, n) = fluid_floors(a, motion, a.step_height(), world, blocks, stands_on);
            haste_physics::collision::collide_with_step_and(
                a.aabb(),
                motion,
                a.step_height(),
                on_ground,
                world,
                shapes,
                &floors[..n],
            )
        }
        None => collide_with_step(a.aabb(), motion, a.step_height(), on_ground, world, shapes),
    };
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
