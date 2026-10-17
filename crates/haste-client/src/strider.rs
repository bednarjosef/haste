//! The strider the local player steers with a warped fungus on a stick: vanilla's `Strider`
//! as the client's own authoritative instance, like the pig ([`crate::pig`]): a constant full
//! forward while the rider holds the stick, the look steers, and the boost comes from the
//! server's synched `DATA_BOOST_TIME`. `getRiddenSpeed` is slower out of the warmth
//! (`DATA_SUFFOCATING`, which the server syncs). Walking on lava (`canStandOnFluid`) needs the
//! fluid's collision shape in the swept move, which the shared travel does not have yet.
use crate::item_based_steering::{ItemBasedSteering, SteerEnv};
use glam::DVec3;
use haste_core::Aabb;

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

    /// One tick of the strider we steer, in vanilla `LivingEntity.aiStep`'s order for a ridden
    /// strider: the velocity thresholds, then `tickRidden` (the rotation follows the rider and the
    /// boost advances) and the shared `travelInAir` with `Strider`'s own input and speed.
    pub fn tick(&mut self, env: &SteerEnv<'_>) {
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
}

/// Whether a `minecraft:entity_type` registry path is the strider this module steers.
pub fn is_strider_type(path: &str) -> bool {
    path == "strider"
}

#[cfg(test)]
mod tests;
