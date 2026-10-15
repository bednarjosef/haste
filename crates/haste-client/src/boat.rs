//! The boat the local player steers: vanilla's `AbstractBoat` as the client's own authoritative
//! instance (`Entity.isLocalInstanceAuthoritative`: a vehicle whose controlling passenger is the
//! local player is simulated on this side, and the server follows the `move_vehicle` packets we
//! send it). One function per vanilla method, in the order `AbstractBoat.tick` runs them.
//!
//! Per tick (vanilla `AbstractBoat.tick`, then `ClientLevel.tickNonPassenger`): the status
//! ([`Boat::status_of`], vanilla `getStatus`), [`Boat::float_boat`], [`Boat::control_boat`],
//! [`Boat::move_boat`] (vanilla's `Entity.move`, swept by `haste_physics::collision::collide`).
//! The input the paddles read is set from the held keys by the passenger's `rideTick`, one tick
//! late, exactly as vanilla's tick order has it.

use glam::DVec3;
use haste_core::input::InputState;
use haste_core::view::{BlockPhysics, BlockView, CollisionShapes, FluidKind};
use haste_core::{Aabb, BlockPos};
use haste_physics::collision::{collide, no_collision};

/// Vanilla `AbstractBoat.getDefaultGravity` (and `Entity.getGravity`, no gravity effect on the
/// client side of a boat).
pub const GRAVITY: f64 = 0.04;
/// The share of a buoyant nudge that survives this tick (`AbstractBoat.floatBoat`).
const BUOYANCY_GRAVITY: f64 = GRAVITY / 0.65;
/// The nudge's damping (0.75) and the friction outside water (0.05), vanilla `floatBoat`.
const BUOYANCY_DAMPING: f64 = 0.75;
const DEFAULT_FRICTION: f32 = 0.05;
/// `floatBoat`: the friction of a boat in water, under water and in the air.
const WATER_FRICTION: f32 = 0.9;
const UNDER_WATER_FRICTION: f32 = 0.45;
const AIR_FRICTION: f32 = 0.9;
/// A boat under flowing water sinks at this vertical speed (`floatBoat`).
const UNDER_FLOWING_WATER_VSPEED: f64 = -7.0e-4;
/// `floatBoat`: the buoyancy of a boat whose top is under water.
const UNDER_WATER_BUOYANCY: f64 = 0.01;
/// `controlBoat`: how the held keys push the boat.
const TURN_INPUT: f32 = 1.0;
const TURN_ACCELERATION: f32 = 0.005;
const FORWARD_ACCELERATION: f32 = 0.04;
const BACKWARD_ACCELERATION: f32 = -0.005;
/// `floatBoat`: how far a boat that touches water is lifted onto the water line.
const WATER_LANDING_OFFSET: f64 = 0.101;
/// Degrees to radians, as vanilla's `(float) (Math.PI / 180.0)` holds it.
const DEG_TO_RAD: f32 = std::f32::consts::PI / 180.0;

/// Where the boat is relative to the world: vanilla's `AbstractBoat.Status`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    /// Floating on the surface (`checkInWater` found water under the hull).
    InWater,
    /// The boat's top is under a water source.
    UnderWater,
    /// The boat's top is under flowing water.
    UnderFlowingWater,
    /// Resting on a block (`getGroundFriction` found one below).
    OnLand,
    /// Neither in water nor on ground.
    InAir,
}

/// The held keys a boat reads (vanilla `AbstractBoat.setInput`): `LocalPlayer.rideTick` hands the
/// boat the raw key states, which its next tick's `controlBoat` uses.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct BoatInput {
    pub left: bool,
    pub right: bool,
    pub forward: bool,
    pub back: bool,
}

impl BoatInput {
    /// The keys a passenger hands its boat: vanilla `LocalPlayer.rideTick`'s
    /// `boat.setInput(keyPresses.left(), keyPresses.right(), keyPresses.forward(),
    /// keyPresses.backward())`.
    pub fn from_input(i: &InputState) -> Self {
        Self {
            left: i.left,
            right: i.right,
            forward: i.forward,
            back: i.back,
        }
    }
}

/// What a boat tick reads from the world: blocks and fluids for the status, the collision
/// shapes for the sweep.
pub struct BoatEnv<'a> {
    pub world: &'a dyn BlockView,
    pub shapes: &'a dyn CollisionShapes,
    pub blocks: &'a dyn BlockPhysics,
}

/// A client-authoritative boat, with the state vanilla's `AbstractBoat` keeps (the paddles' input,
/// the turn speed, the water level and the land friction).
pub struct Boat {
    /// Feet position (the same point an entity's `pos` is).
    pub pos: DVec3,
    /// Blocks per tick, vanilla's `deltaMovement`.
    pub delta: DVec3,
    /// Vanilla's `yRot`; grows past 360 while a paddle turns (vanilla never wraps it).
    pub yaw: f32,
    /// The turn speed the left/right paddles build up, damped by the friction each tick.
    pub delta_rotation: f32,
    pub status: Status,
    pub old_status: Status,
    /// The water surface the hull floats in, from this tick's status check.
    pub water_level: f64,
    /// The friction of the ground the boat rests on (`getGroundFriction`).
    pub land_friction: f32,
    /// Last tick's vertical delta: `AbstractBoat.checkFallDamage` keeps it for
    /// `getWaterLevelAbove`.
    pub last_yd: f64,
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub vertical_collision: bool,
    /// The boat's (width, height) from its entity type's dimensions (registry data).
    dims: (f32, f32),
    input: BoatInput,
    paddle_left: bool,
    paddle_right: bool,
}

impl Boat {
    /// A boat at `pos`, facing `yaw`, sized by the dimensions its registry entry carries. Its
    /// status is the one the world gives it: the client's instance of the entity has been ticking
    /// since it spawned (the boat is not simulated, but its status is), so on mount both the old
    /// and the new status are the current one.
    pub fn new(pos: DVec3, yaw: f32, dims: (f32, f32), env: &BoatEnv<'_>) -> Boat {
        let mut boat = Boat {
            pos,
            delta: DVec3::ZERO,
            yaw,
            delta_rotation: 0.0,
            status: Status::InAir,
            old_status: Status::InAir,
            water_level: pos.y,
            land_friction: DEFAULT_FRICTION,
            last_yd: 0.0,
            on_ground: false,
            horizontal_collision: false,
            vertical_collision: false,
            dims,
            input: BoatInput::default(),
            paddle_left: false,
            paddle_right: false,
        };
        let status = boat.status_of(env);
        boat.status = status;
        boat.old_status = status;
        boat
    }

    pub const fn width(&self) -> f32 {
        self.dims.0
    }

    pub const fn height(&self) -> f32 {
        self.dims.1
    }

    /// Vanilla `AbstractBoat.setInput`, called by the passenger's `rideTick`.
    pub fn set_input(&mut self, input: BoatInput) {
        self.input = input;
    }

    /// The paddle state for the packet: vanilla `AbstractBoat.getPaddleState`, whose second half
    /// (a controlling passenger exists) always holds for the boat we steer.
    pub const fn paddle(&self, side: usize) -> bool {
        if side == 0 {
            self.paddle_left
        } else {
            self.paddle_right
        }
    }

    /// The box the world collides with (vanilla's entity box: feet at `pos`, centred).
    pub fn aabb(&self) -> Aabb {
        Aabb::entity(self.pos, f64::from(self.width()), f64::from(self.height()))
    }

    /// The server's correction of our simulation (vanilla
    /// `ClientPacketListener.handleMoveVehicle` with `ClientboundMoveVehiclePacket`): the boat
    /// is snapped to where the server says it is, and the answer carries it back.
    pub fn correct(&mut self, pos: DVec3, yaw: f32) {
        self.pos = pos;
        self.yaw = yaw;
    }

    /// One tick of the boat we steer, in vanilla `AbstractBoat.tick`'s order. The paddles' input
    /// is the one set by the previous tick's `rideTick`, as in vanilla's tick order.
    pub fn tick(&mut self, env: &BoatEnv<'_>) {
        self.old_status = self.status;
        self.status = self.status_of(env);
        self.float_boat(env);
        self.control_boat();
        self.move_boat(env);
    }

    /// Vanilla `AbstractBoat.getStatus`: water above the hull, water under the hull, ground
    /// below, else air. Each branch also stores what the next tick's `floatBoat` needs.
    fn status_of(&mut self, env: &BoatEnv<'_>) -> Status {
        if let Some(status) = self.is_underwater(env) {
            self.water_level = self.aabb().max.y;
            return status;
        }
        if self.check_in_water(env) {
            return Status::InWater;
        }
        let friction = self.ground_friction(env);
        if friction > 0.0 {
            self.land_friction = friction;
            return Status::OnLand;
        }
        Status::InAir
    }

    /// Vanilla `AbstractBoat.isUnderwater`: the boat's top under water; flowing water reports
    /// `UnderFlowingWater` (a source water, height 8/9, reports `UnderWater` instead).
    fn is_underwater(&self, env: &BoatEnv<'_>) -> Option<Status> {
        let aabb = self.aabb();
        let max_y = aabb.max.y + 0.001;
        let mut under_water = None;
        for x in floor(aabb.min.x)..ceil(aabb.max.x) {
            for y in floor(aabb.max.y)..ceil(max_y) {
                for z in floor(aabb.min.z)..ceil(aabb.max.z) {
                    let Some(height) = water_height(env, x, y, z) else {
                        continue;
                    };
                    if (y as f32 + height) as f64 > max_y {
                        if !is_source(height) {
                            return Some(Status::UnderFlowingWater);
                        }
                        under_water = Some(Status::UnderWater);
                    }
                }
            }
        }
        under_water
    }

    /// Vanilla `AbstractBoat.checkInWater`: water touching the hull's bottom; stores the highest
    /// water surface in `water_level` for the buoyancy.
    fn check_in_water(&mut self, env: &BoatEnv<'_>) -> bool {
        let aabb = self.aabb();
        let mut in_water = false;
        self.water_level = f64::MIN;
        for x in floor(aabb.min.x)..ceil(aabb.max.x) {
            for y in floor(aabb.min.y)..ceil(aabb.min.y + 0.001) {
                for z in floor(aabb.min.z)..ceil(aabb.max.z) {
                    let Some(height) = water_height(env, x, y, z) else {
                        continue;
                    };
                    let surface = (y as f32 + height) as f64;
                    self.water_level = self.water_level.max(surface);
                    in_water |= aabb.min.y < surface;
                }
            }
        }
        in_water
    }

    /// Vanilla `AbstractBoat.getGroundFriction`: the average friction of the collision boxes in
    /// the thin slab under the hull (the block a lily pad sits in has none, so it never counts).
    /// No block down there is `0 / 0` in vanilla (NaN, which every comparison rejects); zero it
    /// is, for the same effect.
    fn ground_friction(&self, env: &BoatEnv<'_>) -> f32 {
        let aabb = self.aabb();
        let slab = Aabb::new(
            DVec3::new(aabb.min.x, aabb.min.y - 0.001, aabb.min.z),
            DVec3::new(aabb.max.x, aabb.min.y, aabb.max.z),
        );
        let (x0, x1) = (floor(slab.min.x) - 1, ceil(slab.max.x) + 1);
        let (y0, y1) = (floor(slab.min.y) - 1, ceil(slab.max.y) + 1);
        let (z0, z1) = (floor(slab.min.z) - 1, ceil(slab.max.z) + 1);
        let mut friction = 0.0;
        let mut count = 0;
        for x in x0..x1 {
            for z in z0..z1 {
                let edges =
                    usize::from(x == x0 || x == x1 - 1) + usize::from(z == z0 || z == z1 - 1);
                if edges == 2 {
                    continue;
                }
                for y in y0..y1 {
                    if edges > 0 && (y == y0 || y == y1 - 1) {
                        continue;
                    }
                    if !block_touches(env, x, y, z, &slab) {
                        continue;
                    }
                    friction += env
                        .blocks
                        .friction(env.world.block_state(BlockPos::new(x, y, z)));
                    count += 1;
                }
            }
        }
        if count == 0 {
            return 0.0;
        }
        friction / count as f32
    }

    /// Vanilla `AbstractBoat.getWaterLevelAbove`: the surface height of the water the boat is
    /// falling into, used by the landing snap in `float_boat`.
    fn water_level_above(&self, env: &BoatEnv<'_>) -> f64 {
        let aabb = self.aabb();
        let (min_x, max_x) = (floor(aabb.min.x), ceil(aabb.max.x));
        let (min_y, max_y) = (floor(aabb.max.y), ceil(aabb.max.y - self.last_yd));
        let (min_z, max_z) = (floor(aabb.min.z), ceil(aabb.max.z));
        for y in min_y..max_y {
            let mut block_height = 0.0f32;
            for x in min_x..max_x {
                for z in min_z..max_z {
                    if let Some(height) = water_height(env, x, y, z) {
                        block_height = block_height.max(height);
                    }
                    if block_height >= 1.0 {
                        break;
                    }
                }
                if block_height >= 1.0 {
                    break;
                }
            }
            if block_height < 1.0 {
                return (y as f32 + block_height) as f64;
            }
        }
        max_y as f64 + 1.0
    }

    /// Vanilla `AbstractBoat.floatBoat`: gravity, drag, buoyancy and the landing snap.
    fn float_boat(&mut self, env: &BoatEnv<'_>) {
        let mut vertical_speed = -GRAVITY;
        let mut buoyancy = 0.0;
        if self.old_status == Status::InAir
            && self.status != Status::InAir
            && self.status != Status::OnLand
        {
            // Touching water from a fall: lift the hull onto the water line.
            self.water_level = self.pos.y;
            let target_y =
                self.water_level_above(env) - f64::from(self.height()) + WATER_LANDING_OFFSET;
            if no_collision(
                &self
                    .aabb()
                    .offset(DVec3::new(0.0, target_y - self.pos.y, 0.0)),
                env.world,
                env.shapes,
            ) {
                self.pos.y = target_y;
                self.delta.y = 0.0;
                self.last_yd = 0.0;
            }
            self.status = Status::InWater;
            return;
        }
        let friction = match self.status {
            Status::InWater => {
                buoyancy = (self.water_level - self.pos.y) / f64::from(self.height());
                WATER_FRICTION
            }
            Status::UnderFlowingWater => {
                vertical_speed = UNDER_FLOWING_WATER_VSPEED;
                WATER_FRICTION
            }
            Status::UnderWater => {
                buoyancy = UNDER_WATER_BUOYANCY;
                UNDER_WATER_FRICTION
            }
            Status::InAir => AIR_FRICTION,
            Status::OnLand => {
                // The boat we steer always has the local player as its controlling passenger.
                self.land_friction /= 2.0;
                self.land_friction
            }
        };
        let drag = f64::from(friction);
        self.delta = DVec3::new(
            self.delta.x * drag,
            self.delta.y + vertical_speed,
            self.delta.z * drag,
        );
        self.delta_rotation *= friction;
        if buoyancy > 0.0 {
            self.delta.y = (self.delta.y + buoyancy * BUOYANCY_GRAVITY) * BUOYANCY_DAMPING;
        }
    }

    /// Vanilla `AbstractBoat.controlBoat`: the paddles turn the boat and push it along its own
    /// facing, and they set the paddle state the packet reports.
    fn control_boat(&mut self) {
        let input = self.input;
        let mut acceleration = 0.0f32;
        if input.left {
            self.delta_rotation -= TURN_INPUT;
        }
        if input.right {
            self.delta_rotation += TURN_INPUT;
        }
        if input.right != input.left && !input.forward && !input.back {
            acceleration += TURN_ACCELERATION;
        }
        self.yaw += self.delta_rotation;
        if input.forward {
            acceleration += FORWARD_ACCELERATION;
        }
        if input.back {
            acceleration += BACKWARD_ACCELERATION;
        }
        let radians = self.yaw * DEG_TO_RAD;
        // Vanilla pushes along (-sin(yaw), cos(yaw)) in floats before widening to the vector.
        let push_x = (-radians).sin() * acceleration;
        let push_z = radians.cos() * acceleration;
        self.delta.x += f64::from(push_x);
        self.delta.z += f64::from(push_z);
        self.paddle_left = (input.right && !input.left) || input.forward;
        self.paddle_right = (input.left && !input.right) || input.forward;
    }

    /// Vanilla's `Entity.move` (`MoverType.SELF`) as the authoritative side runs it: the swept
    /// box, the collision flags, `AbstractBoat.checkFallDamage`'s `lastYd`, the (zero) bounce of
    /// `restituteMovementAfterCollisions` and the block speed factor.
    fn move_boat(&mut self, env: &BoatEnv<'_>) {
        let delta = self.delta;
        let movement = collide(self.aabb(), delta, env.world, env.shapes);
        self.pos += movement;
        // Vanilla `Entity.move`: the horizontal collision flags compare with `Mth.equal` (a
        // 1e-5 tolerance), the vertical one with plain `!=`, and on-ground is a downward one.
        let x_collision = !mth_equal(delta.x, movement.x);
        let z_collision = !mth_equal(delta.z, movement.z);
        self.horizontal_collision = x_collision || z_collision;
        // The side that simulates the boat also owns its vertical collision and on-ground flag.
        self.vertical_collision = delta.y != movement.y;
        self.on_ground = self.vertical_collision && delta.y < 0.0;
        // Vanilla `AbstractBoat.checkFallDamage`: the vertical delta this move started with.
        self.last_yd = self.delta.y;
        // Vanilla `Entity.restituteMovementAfterCollisions` with restitution 0: a collided
        // component stops (a boat never bounces).
        if x_collision {
            self.delta.x = 0.0;
        }
        if z_collision {
            self.delta.z = 0.0;
        }
        if self.vertical_collision {
            self.delta.y = 0.0;
        }
        // Vanilla `Entity.getBlockSpeedFactor`, applied to the horizontal delta.
        let factor = self.block_speed_factor(env);
        self.delta.x *= f64::from(factor);
        self.delta.z *= f64::from(factor);
    }

    /// Vanilla `Entity.getBlockSpeedFactor`: the block the hull is in (water reports its own,
    /// never the block below), else the block under the hull when it is slower.
    fn block_speed_factor(&self, env: &BoatEnv<'_>) -> f32 {
        let here = env.world.block_state(BlockPos::from_dvec3(self.pos));
        let here_factor = env.blocks.speed_factor(here);
        if water_height_at(env, here).is_some() {
            // Vanilla's water and bubble column: that block's own factor, never the one below.
            return here_factor;
        }
        if here_factor != 1.0 {
            return here_factor;
        }
        // Vanilla's `getBlockPosBelowThatAffectsMyMovement`, without the supporting block the
        // client does not track: the block 0.500001 below the hull.
        let below = BlockPos::from_dvec3(self.pos - DVec3::new(0.0, 0.500_001, 0.0));
        env.blocks.speed_factor(env.world.block_state(below))
    }
}

/// A water source's surface height within its cell: the split between standing in water and
/// under flowing water (`FluidState.getHeight`: level 0, and a waterlogged block, = 8/9, and a
/// falling column 1.0, which is not a source).
const SOURCE_HEIGHT: f32 = 8.0 / 9.0;

/// Whether a cell's water height means a source rather than a flow (`FluidState.isSource`, which
/// `FluidInfo` does not carry: a source and a waterlogged block both sit at 8/9, and every other
/// height, the falling column's 1.0 included, is a flow).
fn is_source(height: f32) -> bool {
    (height - SOURCE_HEIGHT).abs() < f32::EPSILON
}

/// How far a boat's passenger may look away from the boat's heading (vanilla's
/// `AbstractBoat.clampRotation` 105 degrees for a passenger that is not a mob).
const BOAT_LOOK_LIMIT: f32 = 105.0;

/// The look a boat's passenger ends a tick with (vanilla `AbstractBoat.positionRider`): the
/// boat's turn drags the look with it (`yRot + deltaRotation`), then `clampRotation` pulls it
/// back to within 105 degrees of the boat's own heading.
pub fn passenger_yaw(look: f32, boat: &Boat) -> f32 {
    let mut yaw = look + boat.delta_rotation;
    let delta = wrap_degrees(yaw - boat.yaw);
    let target = delta.clamp(-BOAT_LOOK_LIMIT, BOAT_LOOK_LIMIT);
    yaw += target - delta;
    yaw
}

/// Vanilla `Mth.wrapDegrees`: the same angle, in (-180, 180).
fn wrap_degrees(mut degrees: f32) -> f32 {
    degrees %= 360.0;
    if degrees >= 180.0 {
        degrees - 360.0
    } else if degrees < -180.0 {
        degrees + 360.0
    } else {
        degrees
    }
}

/// Whether a `minecraft:entity_type` registry path is vanilla's `AbstractBoat` family
/// (`minecraft:oak_boat`, `minecraft:bamboo_raft`, `minecraft:oak_chest_boat`, ...). Only the
/// boat we control is simulated here; every other vehicle (a mod's, a horse, a minecart) keeps
/// its server-driven position.
pub fn is_boat_type(path: &str) -> bool {
    path.ends_with("_boat") || path.ends_with("_raft")
}

/// The water height in one cell, or `None` when it holds no water (vanilla's `FluidTags.WATER`).
fn water_height(env: &BoatEnv<'_>, x: i32, y: i32, z: i32) -> Option<f32> {
    water_height_at(env, env.world.block_state(BlockPos::new(x, y, z)))
}

fn water_height_at(env: &BoatEnv<'_>, state: haste_core::BlockStateId) -> Option<f32> {
    let fluid = env.blocks.fluid(state)?;
    (fluid.kind == FluidKind::Water).then_some(fluid.height)
}

/// Whether a block's collision boxes touch `slab` (vanilla's `Shapes.joinIsNotEmpty` of the
/// block's collision shape with the boat's own).
fn block_touches(env: &BoatEnv<'_>, x: i32, y: i32, z: i32, slab: &Aabb) -> bool {
    let state = env.world.block_state(BlockPos::new(x, y, z));
    let origin = DVec3::new(f64::from(x), f64::from(y), f64::from(z));
    env.shapes
        .collision_boxes(state)
        .iter()
        .any(|b| b.offset(origin).intersects(slab))
}

fn floor(v: f64) -> i32 {
    v.floor() as i32
}

/// Vanilla `Mth.equal`: two doubles are the same movement component when they differ by less
/// than `1.0E-5` (the constant is a float in vanilla, widened here).
fn mth_equal(a: f64, b: f64) -> bool {
    (b - a).abs() < f64::from(1.0e-5f32)
}

fn ceil(v: f64) -> i32 {
    v.ceil() as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use haste_physics::testing::{states, TestShapes, TestWorld};

    /// The boat's box: vanilla `EntityTypes`' `sized(1.375F, 0.5625F)` for the oak boat.
    const BOAT: (f32, f32) = (1.375, 0.5625);

    /// Open water: a 17x17 pool of source water at y = 63, whose surface is `y + 8/9`
    /// (vanilla `FlowingFluid.getHeight`, level 0).
    fn pool() -> TestWorld {
        let mut world = TestWorld::new();
        world.fill((-8, 63, -8), (8, 63, 8), states::WATER_SOURCE);
        world
    }

    /// Open sky over the pool's water, no ground.
    fn over_water() -> (TestWorld, TestShapes) {
        (pool(), TestShapes::default())
    }

    /// A solid floor whose top is y = 65 (the `flat` helper puts the blocks below it).
    fn over_land() -> (TestWorld, TestShapes) {
        let mut world = TestWorld::new();
        world.fill((-8, 63, -8), (8, 64, 8), states::STONE);
        (world, TestShapes::default())
    }

    fn env<'a>(world: &'a TestWorld, shapes: &'a TestShapes) -> BoatEnv<'a> {
        BoatEnv {
            world,
            shapes,
            blocks: shapes,
        }
    }

    /// The forward speed a boat reaches after `ticks` paddle ticks: each tick the water drag
    /// (`floatBoat`, 0.9) takes a tenth of what is there and the forward paddle (`controlBoat`,
    /// 0.04) adds its nudge, so the speed climbs along `a * (1 - f^ticks) / (1 - f)` towards
    /// the 0.4 that `0.04 / (1 - 0.9)` gives.
    fn forward_speed(ticks: u32) -> f64 {
        let a = f64::from(FORWARD_ACCELERATION);
        let f = f64::from(WATER_FRICTION);
        a * (1.0 - f.powi(ticks as i32)) / (1.0 - f)
    }

    /// The distance that speed covers over `ticks` ticks: the sum of the series
    /// `a/(1-f) * (ticks - f * (1 - f^ticks) / (1 - f))`.
    fn forward_distance(ticks: u32) -> f64 {
        let a = f64::from(FORWARD_ACCELERATION);
        let f = f64::from(WATER_FRICTION);
        a / (1.0 - f) * (f64::from(ticks) - f * (1.0 - f.powi(ticks as i32)) / (1.0 - f))
    }

    /// The surface of the pool's water, as the status check stores it (the float arithmetic of
    /// vanilla's `y + getHeight`).
    fn surface() -> f64 {
        (63.0f32 + 8.0 / 9.0) as f64
    }

    /// Where a boat floats: the height at which the buoyant nudge (`floatBoat`,
    /// `(waterLevel - y) / bbHeight` times `gravity / 0.65`) exactly cancels the gravity, which
    /// is 0.65 of its height below the surface.
    fn float_height() -> f64 {
        surface() - 0.65 * f64::from(BOAT.1)
    }

    /// The keys one tick's `rideTick` hands the boat.
    fn keys(forward: bool, left: bool, right: bool, back: bool) -> BoatInput {
        BoatInput {
            left,
            right,
            forward,
            back,
        }
    }

    /// Twenty client ticks: the boat runs with the keys the previous tick's `rideTick` gave it,
    /// then the passenger hands it this tick's keys (vanilla's `AbstractBoat.tick` before
    /// `LocalPlayer.rideTick`).
    fn ticks(boat: &mut Boat, env: &BoatEnv<'_>, keys: BoatInput, count: u32) {
        for _ in 0..count {
            boat.tick(env);
            boat.set_input(keys);
        }
    }

    /// A boat placed at the surface of still water for 40 ticks stays within vanilla's float
    /// band, computed from `AbstractBoat.floatBoat`'s own formulas: each tick the vertical delta
    /// becomes `(dy - gravity + buoyancy * gravity / 0.65) * 0.75` with
    /// `buoyancy = (waterLevel - y) / bbHeight` (the water's drag never scales y), a damped
    /// convergence onto the equilibrium `surface - 0.65 * bbHeight` — the height the vanilla
    /// server floats an oak boat at (its rest at 70.5232 in a surface-70.8889 lane, measured
    /// over RCON). The overshoot of the damping is bounded by the gravity term times the
    /// damping window: the band is one gravity (0.04) below the equilibrium. A hull whose
    /// height is wrong (the live bug: the store's default 1.8 box instead of the oak boat's
    /// 0.5625) makes the buoyancy term 3.2x too weak and sinks past the band onto the floor
    /// within the 40 ticks.
    #[test]
    fn a_boat_at_the_surface_of_still_water_stays_in_vanillas_float_band() {
        let (world, shapes) = over_water();
        let env = env(&world, &shapes);
        // Placed a fifth of a block under the surface, hull height into the water.
        let start = surface() - 0.2;
        let mut boat = Boat::new(DVec3::new(0.5, start, 0.5), 0.0, BOAT, &env);
        assert_eq!(boat.status, Status::InWater);
        // The expected series from vanilla's formulas, ticked alongside the boat.
        let mut expected_y = start;
        let mut expected_dy = 0.0;
        for t in 0..40 {
            ticks(&mut boat, &env, BoatInput::default(), 1);
            let buoyancy = (surface() - expected_y) / f64::from(BOAT.1);
            expected_dy = (expected_dy - GRAVITY + buoyancy * BUOYANCY_GRAVITY) * BUOYANCY_DAMPING;
            expected_y += expected_dy;
            assert!(
                (boat.pos.y - expected_y).abs() < 1.0e-9,
                "tick {t}: y {} vs vanilla's {}",
                boat.pos.y,
                expected_y
            );
            // The band: never above the start, never a gravity below the equilibrium.
            assert!(
                boat.pos.y <= start + 1.0e-9,
                "tick {t}: y {} rose above the start {}",
                boat.pos.y,
                start
            );
            assert!(
                boat.pos.y >= float_height() - GRAVITY,
                "tick {t}: y {} sank below the float band",
                boat.pos.y
            );
        }
        // And it ends holding the vanilla equilibrium, not on the floor.
        assert!(
            (boat.pos.y - float_height()).abs() < 2.0e-3,
            "{} vs {}",
            boat.pos.y,
            float_height()
        );
        assert!(!boat.on_ground, "the hull reached the floor");
    }

    /// Holding W on open water for 20 ticks: vanilla's `controlBoat` adds 0.04 along the boat's
    /// facing and `floatBoat`'s 0.9 water drag takes 10% of it away each tick. The boat does not
    /// turn (no A/D), does not sink (the buoyancy holds it at its floating height) and floats
    /// straight forward.
    #[test]
    fn holding_forward_paddles_straight_at_vanillas_speed() {
        let (world, shapes) = over_water();
        let env = env(&world, &shapes);
        let mut boat = Boat::new(DVec3::new(0.5, float_height(), 0.5), 0.0, BOAT, &env);
        assert_eq!(boat.status, Status::InWater);
        ticks(&mut boat, &env, keys(true, false, false, false), 20);
        // The first tick's keys arrived before the mount, so 19 paddles pushed the boat.
        let paddles = 19;
        assert!(
            (boat.pos.z - (0.5 + forward_distance(paddles))).abs() < 1.0e-6,
            "z: {} vs {}",
            boat.pos.z,
            0.5 + forward_distance(paddles)
        );
        assert!(
            (boat.delta.z - forward_speed(paddles)).abs() < 1.0e-6,
            "forward speed: {} vs {}",
            boat.delta.z,
            forward_speed(paddles)
        );
        // Nothing pushes it sideways and nothing turns it.
        assert!(
            (boat.pos.x - 0.5).abs() < 1.0e-9,
            "x drifted: {}",
            boat.pos.x
        );
        assert!((boat.yaw).abs() < 1.0e-9, "yaw drifted: {}", boat.yaw);
        // The buoyancy holds the hull at its floating height.
        assert!(
            (boat.pos.y - float_height()).abs() < 1.0e-9,
            "dipped or rose: {}",
            boat.pos.y
        );
        // Both paddles are turning, which `paddle_boat` reports.
        assert!(boat.paddle(0) && boat.paddle(1));
        // The box is the one the boat's type carries.
        assert_eq!((boat.width(), boat.height()), BOAT);
    }

    /// A boat left alone in water barely moves: each tick only the gravity and the buoyant nudge
    /// push it, and they cancel at the floating height.
    #[test]
    fn an_unpaddled_boat_holds_its_floating_height() {
        let (world, shapes) = over_water();
        let env = env(&world, &shapes);
        let mut boat = Boat::new(DVec3::new(0.5, float_height(), 0.5), 0.0, BOAT, &env);
        ticks(&mut boat, &env, BoatInput::default(), 20);
        assert!(
            (boat.pos.y - float_height()).abs() < 1.0e-9,
            "{}",
            boat.pos.y
        );
        assert!(boat.pos.distance(DVec3::new(0.5, float_height(), 0.5)) < 1.0e-9);
        assert!(!boat.paddle(0) && !boat.paddle(1));
    }

    /// Holding A turns the boat at the rate vanilla's `deltaRotation` builds: the left paddle
    /// adds 1 degree of turn per tick and the water drag keeps 90% of it, so the turn rate
    /// climbs towards 10 degrees per tick (`1 / (1 - 0.9)`).
    #[test]
    fn holding_left_turns_the_boat() {
        let (world, shapes) = over_water();
        let env = env(&world, &shapes);
        let mut boat = Boat::new(DVec3::new(0.5, float_height(), 0.5), 0.0, BOAT, &env);
        ticks(&mut boat, &env, keys(false, true, false, false), 10);
        // The keys of the first tick were the ones the mount came with, so nine turns built the
        // yaw, each 90% of the rate before it plus a fresh 1.
        let mut yaw = 0.0f32;
        let mut rate = 0.0f32;
        for _ in 0..9 {
            rate = rate * WATER_FRICTION - TURN_INPUT;
            yaw += rate;
        }
        assert!((boat.yaw - yaw).abs() < 1.0e-4, "{} vs {yaw}", boat.yaw);
        assert!((boat.yaw + 34.867_844).abs() < 1.0e-4, "{}", boat.yaw);
        // A turn alone also creeps the boat forward (vanilla's `controlBoat` adds 0.005).
        assert!(boat.delta.z.abs() > 0.0);
        // Only one paddle turns, and it is the right one: vanilla reports the opposite paddle's
        // state for a turn (`setPaddleState(inputRight && !inputLeft || inputUp, ...)`).
        assert!(!boat.paddle(0) && boat.paddle(1));
    }

    /// The passenger's look follows the boat's turn, and may not stray further than vanilla's
    /// `clampRotation` 105 degrees from the boat's heading.
    #[test]
    fn the_passengers_look_is_clamped_to_the_boats_heading() {
        let (world, shapes) = over_water();
        let env = env(&world, &shapes);
        let mut boat = Boat::new(DVec3::new(0.5, float_height(), 0.5), 0.0, BOAT, &env);
        // Looking 200 degrees away from the boat (yaw 0) is pulled back, on the same side, to
        // 105 degrees of it.
        assert!(same_angle(passenger_yaw(200.0, &boat), -105.0));
        // A look inside the limit is left where it is; the boat's turn drags it along.
        assert!(same_angle(passenger_yaw(30.0, &boat), 30.0));
        boat.delta_rotation = 10.0;
        assert!(same_angle(passenger_yaw(30.0, &boat), 40.0));
        // Looking backwards (180 degrees off) still ends 105 degrees off.
        assert!(same_angle(passenger_yaw(-180.0, &boat), -105.0));
    }

    /// Two angles that name the same direction.
    fn same_angle(a: f32, b: f32) -> bool {
        (wrap_degrees(a - b)).abs() < 1.0e-6
    }

    /// A boat on land is on the land status with the block's friction, halved while a player
    /// drives it (vanilla `floatBoat`), and the drag is that friction.
    #[test]
    fn a_boat_on_land_uses_the_blocks_friction() {
        let (world, shapes) = over_land();
        let env = env(&world, &shapes);
        let mut boat = Boat::new(DVec3::new(0.5, 65.0, 0.5), 0.0, BOAT, &env);
        assert_eq!(boat.status, Status::OnLand);
        // TestShapes gives stone the vanilla default slipperiness 0.6, which the driven boat's
        // first tick halves (`floatBoat`).
        boat.tick(&env);
        assert!(
            (boat.land_friction - 0.3).abs() < 1.0e-7,
            "{}",
            boat.land_friction
        );
        // A boat drifting onto land keeps a horizontal delta of its own through the drag, which
        // is the shared friction: each tick takes 70% of it away, and halves the friction the
        // next tick will use (vanilla halves it every tick a player drives it).
        boat.delta.z = 1.0;
        ticks(&mut boat, &env, BoatInput::default(), 1);
        assert!((boat.delta.z - 0.3).abs() < 1.0e-7, "{}", boat.delta.z);
        assert!(boat.on_ground);
    }

    /// With nothing under it but air, the boat is in the air: the gravity is the only thing that
    /// moves it, and `floatBoat`'s air status applies its drag to the horizontal delta only
    /// (`setDeltaMovement(x * drag, y + vspeed, z * drag)`).
    #[test]
    fn a_boat_over_air_falls() {
        let world = TestWorld::new();
        let shapes = TestShapes::default();
        let env = env(&world, &shapes);
        let mut boat = Boat::new(DVec3::new(0.5, 70.0, 0.5), 0.0, BOAT, &env);
        assert_eq!(boat.status, Status::InAir);
        ticks(&mut boat, &env, BoatInput::default(), 1);
        assert!(
            (boat.delta.y - (-GRAVITY)).abs() < 1.0e-12,
            "{}",
            boat.delta.y
        );
        assert!((boat.pos.y - (70.0 - GRAVITY)).abs() < 1.0e-12);
    }

    /// Water deep enough to cover the hull keeps the boat under water (`getStatus`'s
    /// `isUnderwater`), where the drag is the under-water friction.
    #[test]
    fn a_boat_under_water_reports_the_under_water_status() {
        let mut world = TestWorld::new();
        world.fill((-8, 60, -8), (8, 70, 8), states::WATER_SOURCE);
        let shapes = TestShapes::default();
        let env = env(&world, &shapes);
        let mut boat = Boat::new(DVec3::new(0.5, 65.0, 0.5), 0.0, BOAT, &env);
        boat.tick(&env);
        assert_eq!(boat.status, Status::UnderWater);
        boat.delta = DVec3::new(0.0, -0.1, 1.0);
        boat.tick(&env);
        assert!(
            (boat.delta.z - f64::from(UNDER_WATER_FRICTION)).abs() < 1.0e-9,
            "{}",
            boat.delta.z
        );
    }
}
