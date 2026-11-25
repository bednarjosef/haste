//! The strider the local player steers with a warped fungus on a stick: vanilla's `Strider`
//! as the client's own authoritative instance, like the pig ([`crate::pig`]): a constant full
//! forward while the rider holds the stick, the look steers, and the boost comes from the
//! server's synched `DATA_BOOST_TIME`. `getRiddenSpeed` is slower out of the warmth
//! (`DATA_SUFFOCATING`): the client decides warm/cold itself each tick from the blocks around
//! (`Strider.tick`'s `setSuffocating`), instead of waiting for the server's flag. The strider
//! walks on lava (`canStandOnFluid`): the shared travel gives the fluid surface a collision
//! box ([`crate::travel`]) and `floatStrider` settles it there when it drops below.
use crate::item_based_steering::{ItemBasedSteering, SteerEnv};
use crate::travel::RiddenAnimal;
use glam::DVec3;
use haste_core::view::{BlockPhysics, FluidKind};
use haste_core::{Aabb, BlockStateId};

/// Vanilla `Strider.createAttributes`: `Attributes.MOVEMENT_SPEED` base 0.175F (the server
/// syncs the real value, with the suffocating modifier, through `update_attributes`; this is
/// the fallback).
pub const DEFAULT_MOVEMENT_SPEED: f64 = 0.175_f32 as f64;
/// Vanilla `Attributes.STEP_HEIGHT`'s default for a strider.
pub const STEP_HEIGHT: f32 = 0.6;
/// Vanilla `Strider.STEERING_MODIFIER` (0.55F) and `SUFFOCATE_STEERING_MODIFIER` (0.35F),
/// `getRiddenSpeed`'s factor in and out of the warmth.
const STEERING_MODIFIER: f32 = 0.55;
const SUFFOCATE_STEERING_MODIFIER: f32 = 0.35;
/// `Strider.DATA_SUFFOCATING`'s synched data index (after `DATA_BOOST_TIME`).
pub const DATA_SUFFOCATING: u8 = 19;
/// A strider we steer, with the boost state of vanilla's `ItemBasedSteering`.
pub struct Strider {
    /// Feet position (the same point an entity's `pos` is).
    pub pos: DVec3,
    /// Blocks per tick, vanilla's `deltaMovement`.
    pub delta: DVec3,
    /// Vanilla's `yRot`: follows the rider's yaw (`Strider.tickRidden`).
    pub yaw: f32,
    /// Vanilla's `xRot`: half the rider's pitch.
    pub pitch: f32,
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub vertical_collision: bool,
    /// `Attributes.MOVEMENT_SPEED` (synced by the server).
    pub movement_speed: f64,
    /// `Attributes.GRAVITY` (the default 0.08 for a strider).
    pub gravity: f64,
    /// The strider's (width, height) from its entity type's dimensions.
    dims: (f32, f32),
    /// The boost (`Strider.steering`).
    pub steering: ItemBasedSteering,
    /// `Strider.isSuffocating` (the synched flag).
    pub suffocating: bool,
    /// The rider's look of the previous tick (`tickRidden` reads `controller.getYRot()`).
    look: (f32, f32),
}

impl Strider {
    /// A strider at `pos` facing `yaw`, sized by the dimensions its registry entry carries, with
    /// the attributes the server last synced (the strider's base values as the fallback).
    pub fn new(
        pos: DVec3,
        yaw: f32,
        dims: (f32, f32),
        movement_speed: f64,
        gravity: f64,
    ) -> Strider {
        Strider {
            pos,
            delta: DVec3::ZERO,
            yaw,
            pitch: 0.0,
            on_ground: true,
            horizontal_collision: false,
            vertical_collision: false,
            movement_speed,
            gravity,
            dims,
            steering: ItemBasedSteering::default(),
            suffocating: false,
            look: (yaw, 0.0),
        }
    }

    /// The server's correction of our simulation (vanilla
    /// `ClientPacketListener.handleMoveVehicle`): snapped to where the server says it is.
    pub fn correct(&mut self, pos: DVec3, yaw: f32) {
        self.pos = pos;
        self.yaw = yaw;
    }

    /// The rider's look for the next tick (the vehicle ticks before its passenger, so what it
    /// reads is one tick old).
    pub fn set_look(&mut self, look: (f32, f32)) {
        self.look = look;
    }

    /// One tick of the strider we steer, in vanilla's order for a ridden strider: the warm/cold
    /// decision (`Strider.tick`'s `setSuffocating`, before `super.tick()` runs the travel), the
    /// velocity thresholds (`aiStep`), then `tickRidden` (the rotation follows the rider and the
    /// boost advances) and the shared `travelInAir` with `Strider`'s own input and speed, and
    /// `floatStrider` after the move (vanilla runs it at the end of `tick`).
    pub fn tick(&mut self, env: &SteerEnv<'_>) {
        self.update_suffocating(env);
        if self.delta.x.abs() < 0.003 {
            self.delta.x = 0.0;
        }
        if self.delta.y.abs() < 0.003 {
            self.delta.y = 0.0;
        }
        if self.delta.z.abs() < 0.003 {
            self.delta.z = 0.0;
        }
        self.tick_ridden();
        // getRiddenInput: a constant full forward; the rider steers with the look alone.
        crate::travel::travel_in_air(
            self,
            (0.0, 1.0),
            self.yaw,
            env.world,
            env.shapes,
            env.blocks,
        );
        self.float_strider(env);
    }

    /// The client's own warm/cold decision, vanilla `Strider.tick`'s `setSuffocating` (the
    /// server ticks the same code and syncs `DATA_SUFFOCATING`; computing it here makes the
    /// speed switch on the right tick instead of a network round-trip later). Warm is the
    /// `#strider_warm_blocks` tag on the block inside or below the feet, or any lava the box
    /// touches (`getFluidHeight(LAVA) > 0`). Without the tag (no `update_tags` from the server
    /// yet) only lava warms — the tag's other members are fire-family blocks.
    fn update_suffocating(&mut self, env: &SteerEnv<'_>) {
        let feet = haste_core::BlockPos::from_dvec3(self.pos);
        let warm_block = |s: BlockStateId| {
            is_lava(env.blocks, s) || env.warm_blocks.is_some_and(|w| w.is_strider_warm(s))
        };
        let in_warm_blocks = warm_block(env.world.block_state(feet))
            || warm_block(env.world.block_state(feet.add(0, -1, 0)));
        let fluid_height = fluid_touch_height(self.aabb(), env);
        let warm = in_warm_blocks || fluid_height > 0.0;
        self.suffocating = !warm;
    }

    /// Vanilla `Strider.floatStrider`, after the move: a strider inside lava either stands on
    /// the surface (its feet above the liquid collision column and no lava above) or bobs up
    /// (`delta * 0.5 + 0.05`).
    fn float_strider(&mut self, env: &SteerEnv<'_>) {
        if !is_lava(
            env.blocks,
            env.world
                .block_state(haste_core::BlockPos::from_dvec3(self.pos)),
        ) {
            return;
        }
        // `context.isAbove(this.getLiquidCollisionShape(), this.blockPosition(), true)`: the
        // feet above the column's half-block top, and no lava directly above the block.
        let feet = haste_core::BlockPos::from_dvec3(self.pos);
        let above = env.world.block_state(feet.add(0, 1, 0));
        let stands = self.pos.y > feet.y as f64 + 0.5 - 1.0e-5 && !is_lava(env.blocks, above);
        if stands {
            self.on_ground = true;
        } else {
            self.delta = self.delta * 0.5 + DVec3::new(0.0, 0.05, 0.0);
        }
    }

    /// Vanilla `Strider.tickRidden`: the rotation follows the rider (the pitch halved) and the
    /// boost advances (`ItemBasedSteering.tickBoost`: the boost ends once its time passes the
    /// total).
    fn tick_ridden(&mut self) {
        self.pitch = self.look.1 * 0.5;
        self.yaw = self.look.0;
        self.steering.tick_boost();
    }

    /// A `set_entity_data` for the strider: `DATA_SUFFOCATING`.
    pub fn on_metadata(&mut self, entries: &[haste_core::entity::MetadataEntry]) {
        for e in entries {
            if e.index == DATA_SUFFOCATING {
                if let haste_core::entity::MetadataValue::Boolean(b) = e.value {
                    self.suffocating = b;
                }
            }
        }
    }
}

impl crate::travel::RiddenAnimal for Strider {
    fn pos(&self) -> DVec3 {
        self.pos
    }
    fn delta(&self) -> DVec3 {
        self.delta
    }
    fn set_delta(&mut self, d: DVec3) {
        self.delta = d;
    }
    fn add_pos(&mut self, d: DVec3) {
        self.pos += d;
    }
    fn aabb(&self) -> Aabb {
        Aabb::entity(self.pos, f64::from(self.dims.0), f64::from(self.dims.1))
    }
    /// `getRiddenSpeed`: `movement_speed * (0.35 suffocating, else 0.55) * boostFactor`.
    fn speed(&self) -> f32 {
        let factor = if self.suffocating {
            SUFFOCATE_STEERING_MODIFIER
        } else {
            STEERING_MODIFIER
        };
        (self.movement_speed * f64::from(factor) * f64::from(self.steering.boost_factor())) as f32
    }
    fn step_height(&self) -> f64 {
        f64::from(STEP_HEIGHT)
    }
    fn gravity(&self) -> f64 {
        self.gravity
    }
    fn on_ground(&self) -> bool {
        self.on_ground
    }
    fn set_on_ground(&mut self, g: bool) {
        self.on_ground = g;
    }
    fn set_collisions(&mut self, horizontal: bool, vertical: bool) {
        self.horizontal_collision = horizontal;
        self.vertical_collision = vertical;
    }
    /// `Strider.canStandOnFluid`: lava.
    fn stands_on_fluid(&self) -> Option<haste_core::view::FluidKind> {
        Some(haste_core::view::FluidKind::Lava)
    }
}

/// Whether `state` carries lava (`Strider.canStandOnFluid`'s test, `fluid.is(FluidTags.LAVA)`).
pub fn is_lava(blocks: &dyn BlockPhysics, state: BlockStateId) -> bool {
    blocks
        .fluid(state)
        .is_some_and(|f| f.kind == FluidKind::Lava)
}

/// The lava the strider's box touches, as vanilla `Entity.getFluidHeight(LAVA)`: the fluid
/// top minus the box bottom, over the box's deflated cells (the fluid trackers read the box
/// shrunk by 0.001), or 0 when no lava reaches the box.
fn fluid_touch_height(aabb: Aabb, env: &SteerEnv<'_>) -> f64 {
    let box_shrunk = Aabb {
        min: aabb.min + DVec3::splat(0.001),
        max: aabb.max - DVec3::splat(0.001),
    };
    let (x0, x1) = (
        box_shrunk.min.x.floor() as i32,
        (box_shrunk.max.x.ceil() - 1.0) as i32,
    );
    let (y0, y1) = (
        box_shrunk.min.y.floor() as i32,
        (box_shrunk.max.y.ceil() - 1.0) as i32,
    );
    let (z0, z1) = (
        box_shrunk.min.z.floor() as i32,
        (box_shrunk.max.z.ceil() - 1.0) as i32,
    );
    let mut height = 0.0;
    for x in x0..=x1 {
        for y in y0..=y1 {
            for z in z0..=z1 {
                let pos = haste_core::BlockPos::new(x, y, z);
                let state = env.world.block_state(pos);
                let Some(f) = env.blocks.fluid(state) else {
                    continue;
                };
                if f.kind != FluidKind::Lava {
                    continue;
                }
                // `FluidState.getHeight`: the full block when the same fluid sits above.
                let top = if is_lava(env.blocks, env.world.block_state(pos.add(0, 1, 0))) {
                    1.0
                } else {
                    f.height as f64
                };
                let fluid_top = y as f64 + top;
                if fluid_top >= box_shrunk.min.y {
                    height = (fluid_top - aabb.min.y).max(height);
                }
            }
        }
    }
    height
}

/// Whether a `minecraft:entity_type` registry path is the strider this module steers.
pub fn is_strider_type(path: &str) -> bool {
    path == "strider"
}

#[cfg(test)]
mod tests;
