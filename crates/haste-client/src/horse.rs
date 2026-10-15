//! The horse family the local player steers: vanilla's `AbstractHorse` as the client's own
//! authoritative instance (`Entity.isLocalInstanceAuthoritative`: a vehicle whose controlling
//! passenger is the local player is simulated on this side, and the server follows the
//! `move_vehicle` packets we send it). One function per vanilla method, in the order
//! `LivingEntity.aiStep` runs them for a ridden horse.
//!
//! Per tick: [`Horse::tick_ridden`] (vanilla `AbstractHorse.tickRidden`: the rotation follows the
//! rider, the charged jump fires from the ground), [`Horse::travel`] (vanilla
//! `LivingEntity.travel` -> `travelInAir`: the input acceleration, the swept move, gravity and
//! the drags). The input is the rider's held keys of the *previous* tick (vanilla's tick order:
//! the vehicle ticks before its passenger, so `controller.xxa`/`zza` are one tick old), exactly
//! as the boat's paddles are.

use glam::DVec3;
use haste_core::input::InputState;
use haste_core::view::{BlockPhysics, BlockView, CollisionShapes};
use haste_core::{Aabb, BlockPos};
use haste_physics::collision::collide_with_step;

/// Vanilla `Attributes.MOVEMENT_SPEED`'s base for `AbstractHorse.createBaseHorseAttributes`
/// (`getRiddenSpeed` reads the attribute; a server syncs the real per-horse value through
/// `update_attributes`, this is only its fallback).
pub const DEFAULT_MOVEMENT_SPEED: f64 = 0.225;
/// Vanilla `Attributes.JUMP_STRENGTH`'s base for the horse family (see above).
pub const DEFAULT_JUMP_STRENGTH: f64 = 0.7;
/// Vanilla `Attributes.STEP_HEIGHT`'s base for the horse family.
pub const STEP_HEIGHT: f32 = 1.0;
/// Vanilla `LivingEntity.travelInAir`: the air drag applied to the horizontal velocity.
const HORIZONTAL_DRAG: f32 = 0.91;
/// Vanilla `LivingEntity.travelInAir`: the vertical drag.
const VERTICAL_DRAG: f32 = 0.98;
/// Vanilla `Attributes.GRAVITY`'s default (0.08), what `travelInAir` subtracts.
pub const DEFAULT_GRAVITY: f64 = 0.08;
/// Vanilla `LivingEntity.getFrictionInfluencedSpeed`: the ground-acceleration constant.
const GROUND_ACCELERATION: f32 = 0.216_000_02;
/// The slipperiness above which the ground acceleration formula applies (vanilla
/// `getFrictionInfluencedSpeed`: `blockFriction > 0.6`).
const SLIPPERY_THRESHOLD: f32 = 0.6;
/// The rider's input scale (vanilla `LocalPlayer.modifyInput`: 0.98).
const INPUT_SCALE: f32 = 0.98;
/// Vanilla `AbstractHorse.SIDEWAYS_MOVE_SPEED_FACTOR` (0.5).
const SIDEWAYS_FACTOR: f32 = 0.5;
/// Vanilla `AbstractHorse.BACKWARDS_MOVE_SPEED_FACTOR` (0.25).
const BACKWARDS_FACTOR: f32 = 0.25;
/// The forward push of a charged jump (`AbstractHorse.executeRidersJump`: 0.4).
const JUMP_PUSH: f32 = 0.4;
/// Vanilla `PlayerRideableJumping.getPlayerJumpPendingScale`: 90 or more is a full jump.
const FULL_JUMP_AMOUNT: i32 = 90;
/// Vanilla `LocalPlayer.aiStep`'s charge: the scale is `ticks * 0.1` for the first ten ticks.
const CHARGE_STEP: f32 = 0.1;
/// Vanilla `LocalPlayer.aiStep`'s charge past ten ticks: `0.8 + 2 / (ticks - 9) * 0.1`, so the
/// scale approaches 1 without reaching it.
const CHARGE_LONG_BASE: f32 = 0.8;
/// Vanilla `AbstractHorse.setStanding`'s rear-up duration.
const STAND_TICKS: u32 = 20;

/// The state of one horse-family vehicle we steer. The attributes are the ones the server syncs
/// (`update_attributes`); their defaults are the horse family's base values, which vanilla's own
/// client would see before any sync lands.
pub struct Horse {
    /// Feet position (the same point an entity's `pos` is).
    pub pos: DVec3,
    /// Blocks per tick, vanilla's `deltaMovement`.
    pub delta: DVec3,
    /// Vanilla's `yRot`: follows the rider's yaw (`AbstractHorse.tickRidden`).
    pub yaw: f32,
    /// Vanilla's `xRot`: half the rider's pitch (`AbstractHorse.getRiddenRotation`).
    pub pitch: f32,
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub vertical_collision: bool,
    /// `AbstractHorse.playerJumpPendingScale`: the charged jump waiting for the ground.
    pub player_jump_pending_scale: f32,
    /// `AbstractHorse.allowStandSliding`: a charged jump may slide off a rear-up.
    allow_stand_sliding: bool,
    /// `AbstractHorse.standCounter`: ticks of rear-up left (`isStanding` while > 0).
    stand_counter: u32,
    /// The horse's (width, height) from its entity type's dimensions.
    dims: (f32, f32),
    /// `Attributes.MOVEMENT_SPEED` (synced by the server).
    pub movement_speed: f64,
    /// `Attributes.JUMP_STRENGTH` (synced by the server).
    pub jump_strength: f64,
    /// `Attributes.GRAVITY` (synced by the server; the horse family leaves it at the default).
    pub gravity: f64,
    /// The rider's input of the previous tick, as vanilla's tick order hands it to the vehicle
    /// (`controller.xxa` / `controller.zza`, already scaled and clamped like
    /// `LocalPlayer.modifyInput`).
    input: (f32, f32),
    /// The rider's look of the previous tick (`tickRidden` reads `controller.getYRot()`).
    look: (f32, f32),
}

impl Horse {
    /// A horse at `pos` facing `yaw`, sized by the dimensions its registry entry carries, with the
    /// attributes the server last synced (or the family defaults).
    pub fn new(
        pos: DVec3,
        yaw: f32,
        dims: (f32, f32),
        movement_speed: f64,
        jump_strength: f64,
        gravity: f64,
    ) -> Horse {
        Horse {
            pos,
            delta: DVec3::ZERO,
            yaw,
            pitch: 0.0,
            on_ground: true,
            horizontal_collision: false,
            vertical_collision: false,
            player_jump_pending_scale: 0.0,
            allow_stand_sliding: false,
            stand_counter: 0,
            dims,
            movement_speed,
            jump_strength,
            gravity,
            input: (0.0, 0.0),
            look: (yaw, 0.0),
        }
    }

    pub const fn width(&self) -> f32 {
        self.dims.0
    }

    pub const fn height(&self) -> f32 {
        self.dims.1
    }

    /// The box the world collides with (vanilla's entity box: feet at `pos`, centred).
    pub fn aabb(&self) -> Aabb {
        Aabb::entity(self.pos, f64::from(self.width()), f64::from(self.height()))
    }

    /// The server's correction of our simulation (vanilla
    /// `ClientPacketListener.handleMoveVehicle`): snapped to where the server says it is.
    pub fn correct(&mut self, pos: DVec3, yaw: f32) {
        self.pos = pos;
        self.yaw = yaw;
    }

    /// The rider's input for the next tick (vanilla `LocalPlayer.applyInput`: the vehicle ticks
    /// before its passenger, so what it reads is one tick old). `move` is the raw key impulses
    /// (strafe, forward), each -1, 0 or 1; `look` is the rider's yaw and pitch.
    pub fn set_input(&mut self, move_x: f32, move_z: f32, look: (f32, f32)) {
        self.input = (move_x, move_z);
        self.look = look;
    }

    /// Vanilla `AbstractHorse.onPlayerJump` (from the rider's charged jump): a rear-up that lets
    /// the horse slide forward, and the scale the pending jump fires with.
    pub fn on_player_jump(&mut self, jump_amount: i32) {
        if jump_amount < 0 {
            return;
        }
        self.allow_stand_sliding = true;
        // standIfPossible: a rear-up for twenty ticks.
        self.stand_counter = STAND_TICKS;
        self.player_jump_pending_scale = pending_scale(jump_amount);
    }

    /// One tick of the horse we steer, in vanilla `LivingEntity.aiStep`'s order for a ridden
    /// horse: the velocity thresholds (a non-player entity zeroes each small component on its
    /// own), then `tickRidden` and `travel` through `travelRidden`, then the walk animation.
    pub fn tick(&mut self, env: &HorseEnv<'_>) {
        if self.delta.x.abs() < 0.003 {
            self.delta.x = 0.0;
        }
        if self.delta.y.abs() < 0.003 {
            self.delta.y = 0.0;
        }
        if self.delta.z.abs() < 0.003 {
            self.delta.z = 0.0;
        }
        if self.stand_counter > 0 {
            self.stand_counter -= 1;
        }
        let input = self.ridden_input();
        self.tick_ridden(&input);
        self.travel(input, env);
    }

    /// Vanilla `AbstractHorse.getRiddenInput`: nothing while a rear-up holds the horse on the
    /// ground (unless the jump charged enough to slide), else the rider's strafe halved and the
    /// forward impulse, backwards quartered.
    fn ridden_input(&self) -> (f32, f32) {
        if self.on_ground
            && self.player_jump_pending_scale == 0.0
            && self.stand_counter > 0
            && !self.allow_stand_sliding
        {
            return (0.0, 0.0);
        }
        let (strafe, forward) = self.input;
        let forward = if forward <= 0.0 {
            forward * BACKWARDS_FACTOR
        } else {
            forward
        };
        (strafe * SIDEWAYS_FACTOR, forward)
    }

    /// Vanilla `AbstractHorse.tickRidden`: the rotation follows the rider (the pitch halved), and
    /// a charged jump fires from the ground.
    fn tick_ridden(&mut self, input: &(f32, f32)) {
        // getRiddenRotation: half the rider's pitch, the rider's yaw.
        self.pitch = self.look.1 * 0.5;
        self.yaw = self.look.0;
        if input.1 <= 0.0 {
            // gallopSoundCounter = 0 (no sounds yet).
        }
        if self.on_ground {
            if self.player_jump_pending_scale > 0.0 {
                self.execute_riders_jump(self.player_jump_pending_scale, *input);
            }
            self.player_jump_pending_scale = 0.0;
        }
    }

    /// Vanilla `AbstractHorse.executeRidersJump`: the jump impulse from the charge scale, plus
    /// the forward push along the yaw when the rider rides forward.
    fn execute_riders_jump(&mut self, amount: f32, input: (f32, f32)) {
        let impulse = self.jump_power(amount);
        self.delta.y = impulse;
        if input.1 > 0.0 {
            let r = self.yaw * (std::f32::consts::PI / 180.0);
            self.delta.x -= f64::from(JUMP_PUSH * r.sin() * amount);
            self.delta.z += f64::from(JUMP_PUSH * r.cos() * amount);
        }
    }

    /// Vanilla `LivingEntity.getJumpPower` for the horse family: the `JUMP_STRENGTH` attribute
    /// scaled by the charge (no jump boost effect, block jump factor 1 on the platform).
    fn jump_power(&self, amount: f32) -> f64 {
        self.jump_strength * f64::from(amount)
    }

    /// Vanilla `LivingEntity.travel` -> `travelInAir` with the ridden speed: the ground
    /// acceleration from the block's slipperiness, the swept move with the horse's step height,
    /// then gravity and the drags.
    fn travel(&mut self, input: (f32, f32), env: &HorseEnv<'_>) {
        // getRiddenSpeed: the MOVEMENT_SPEED attribute.
        let speed = self.movement_speed as f32;
        let slip = if self.on_ground {
            env.blocks
                .friction(ground_block(self.pos, self.on_ground, env))
        } else {
            1.0
        };
        // getFrictionInfluencedSpeed: on the ground the acceleration follows the block's
        // slipperiness; in the air the flying speed (the horse's speed * 0.1, a player controls
        // it). A horse slips on ice exactly like the player does.
        let acceleration = if self.on_ground && slip > SLIPPERY_THRESHOLD {
            speed * (GROUND_ACCELERATION / (slip * slip * slip))
        } else if self.on_ground {
            speed
        } else {
            speed * 0.1
        };
        self.move_relative(acceleration, input);
        let motion = self.delta;
        self.move_horse(motion, env);
        self.delta.y -= self.gravity;
        self.delta.y *= VERTICAL_DRAG as f64;
        let drag = f64::from(slip * HORIZONTAL_DRAG);
        self.delta.x *= drag;
        self.delta.z *= drag;
    }

    /// Vanilla `Entity.moveRelative` -> `getInputVector`: the input rotated by the yaw, its
    /// length normalised past 1.
    fn move_relative(&mut self, speed: f32, input: (f32, f32)) {
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
        let r = f64::from(self.yaw) * (std::f64::consts::PI / 180.0);
        let (sin, cos) = (r.sin(), r.cos());
        self.delta.x += x * cos - z * sin;
        self.delta.z += z * cos + x * sin;
    }

    /// Vanilla's `Entity.move` (`MoverType.SELF`) as the authoritative side runs it: the swept
    /// box with the horse's step height, the collision flags and the zeroed collided components
    /// (`restituteMovementAfterCollisions` with the horse's zero bounciness).
    fn move_horse(&mut self, motion: DVec3, env: &HorseEnv<'_>) {
        let moved = collide_with_step(
            self.aabb(),
            motion,
            f64::from(STEP_HEIGHT),
            self.on_ground,
            env.world,
            env.shapes,
        );
        self.pos += moved;
        let x_collision = moved.x != motion.x;
        let z_collision = moved.z != motion.z;
        self.horizontal_collision = x_collision || z_collision;
        self.vertical_collision = moved.y != motion.y;
        self.on_ground = self.vertical_collision && motion.y < 0.0;
        if x_collision {
            self.delta.x = 0.0;
        }
        if z_collision {
            self.delta.z = 0.0;
        }
        if self.vertical_collision {
            self.delta.y = 0.0;
        }
    }
}

/// Vanilla `PlayerRideableJumping.getPlayerJumpPendingScale`.
pub fn pending_scale(jump_amount: i32) -> f32 {
    if jump_amount >= FULL_JUMP_AMOUNT {
        1.0
    } else {
        0.4 + 0.4 * jump_amount as f32 / FULL_JUMP_AMOUNT as f32
    }
}

/// The rider's charged jump scale for one held-tick count (vanilla `LocalPlayer.aiStep`: the
/// `jumpRidingTicks` counter's own formula, with the charge starting on the press tick).
pub fn charge_scale(ticks: i32) -> f32 {
    if ticks < 10 {
        ticks as f32 * CHARGE_STEP
    } else {
        CHARGE_LONG_BASE + 2.0 / (ticks - 9) as f32 * CHARGE_STEP
    }
}

/// The floor of the charge as the packet carries it (vanilla
/// `LocalPlayer.sendRidingJump`: `Mth.floor(scale * 100)`).
pub fn charge_amount(scale: f32) -> i32 {
    (scale * 100.0).floor() as i32
}

/// The rider's move impulses from the held keys, as vanilla `KeyboardInput.tick` +
/// `LocalPlayer.modifyInput` produce them: one key each way is ±1, both-false 0, normalized
/// (diagonal keys share the length) and scaled by 0.98 onto the unit square.
pub fn rider_input(i: &InputState) -> (f32, f32) {
    let forward = impulse(i.forward, i.back);
    let strafe = impulse(i.left, i.right);
    let length = (strafe * strafe + forward * forward).sqrt();
    if length == 0.0 {
        return (0.0, 0.0);
    }
    let (mut x, mut z) = (strafe / length, forward / length);
    // modifyInputSpeedForSquareMovement: stretch onto the unit square, cap at 1.
    let ax = x.abs();
    let az = z.abs();
    let tan = if az > ax { ax / az } else { az / ax };
    let distance = (1.0 + tan * tan).sqrt();
    let modified = (length * INPUT_SCALE * distance).min(1.0);
    x *= modified;
    z *= modified;
    (x, z)
}

fn impulse(positive: bool, negative: bool) -> f32 {
    if positive == negative {
        0.0
    } else if positive {
        1.0
    } else {
        -1.0
    }
}

/// The block under the feet that the friction comes from (vanilla
/// `Entity.getBlockPosBelowThatAffectsMyMovement` = `getOnPos(0.500001)`): a horse on the ground
/// reads the block half a block below its feet.
fn ground_block(pos: DVec3, on_ground: bool, env: &HorseEnv<'_>) -> haste_core::BlockStateId {
    let _ = on_ground;
    env.world
        .block_state(BlockPos::from_dvec3(pos - DVec3::new(0.0, 0.500_001, 0.0)))
}

/// What a horse tick reads from the world.
pub struct HorseEnv<'a> {
    pub world: &'a dyn BlockView,
    pub shapes: &'a dyn CollisionShapes,
    pub blocks: &'a dyn BlockPhysics,
}

/// Whether a `minecraft:entity_type` registry path is the horse family this module steers
/// (`AbstractHorse` minus the camel, which has its own dash, and the llama branch, which no
/// player steers). Only the horse we control is simulated here; every other vehicle keeps its
/// server-driven position.
pub fn is_horse_type(path: &str) -> bool {
    matches!(
        path,
        "horse" | "donkey" | "mule" | "skeleton_horse" | "zombie_horse"
    )
}

#[cfg(test)]
mod tests;
