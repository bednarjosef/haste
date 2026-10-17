//! The pig the local player steers with a carrot on a stick: vanilla's `Pig` as the client's
//! own authoritative instance (`Entity.isLocalInstanceAuthoritative`, the same arrangement the
//! boat and the horse family use). One function per vanilla method, in the order
//! `LivingEntity.aiStep` runs them for a ridden pig.
//!
//! The rider does not walk the pig with W: `Pig.getRiddenInput` is a constant full forward
//! while the rider holds the carrot on a stick (`Pig.getControllingPassenger`'s own condition),
//! so the pig walks on its own and the rider only steers with the look. The boost a use click
//! starts (`FoodOnAStickItem.use` -> `ItemSteerable.boost`) is the server's: it writes the
//! boost's total length into the pig's synched data, and the metadata update starts our side of
//! it (`ItemBasedSteering.onSynced`).
use glam::DVec3;
use haste_core::entity::MetadataValue;
use haste_core::view::{BlockPhysics, BlockView, CollisionShapes};
use haste_core::Aabb;

/// Vanilla `Pig.createAttributes`: `Attributes.MOVEMENT_SPEED` base 0.25 (the server syncs the
/// real value through `update_attributes`; this is the fallback).
pub const DEFAULT_MOVEMENT_SPEED: f64 = 0.25;
/// Vanilla `Attributes.STEP_HEIGHT`'s default for a pig.
pub const STEP_HEIGHT: f32 = 0.6;
/// Vanilla `Pig.getRiddenSpeed`'s own factor.
const RIDDEN_SPEED_FACTOR: f64 = 0.225;
/// Vanilla `ItemBasedSteering.boostFactor`: `1 + 1.15 * sin(boostTime / total * PI)`.
const BOOST_AMPLITUDE: f32 = 1.15;
/// `Pig.DATA_BOOST_TIME`: the synched data index of the boost's total length (the wire
/// layout: `Entity`'s 8 slots, `LivingEntity`'s 6, `Mob`'s 1, `AgeableMob`'s 2, then `Pig`'s
/// own first accessor).
pub const DATA_BOOST_TIME: u8 = 18;

/// A pig we steer, with the boost state of vanilla's `ItemBasedSteering`.
pub struct Pig {
    /// Feet position (the same point an entity's `pos` is).
    pub pos: DVec3,
    /// Blocks per tick, vanilla's `deltaMovement`.
    pub delta: DVec3,
    /// Vanilla's `yRot`: follows the rider's yaw (`Pig.tickRidden`).
    pub yaw: f32,
    /// Vanilla's `xRot`: half the rider's pitch.
    pub pitch: f32,
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub vertical_collision: bool,
    /// `Attributes.MOVEMENT_SPEED` (synced by the server).
    pub movement_speed: f64,
    /// `Attributes.GRAVITY` (the default 0.08 for a pig).
    pub gravity: f64,
    /// The pig's (width, height) from its entity type's dimensions.
    dims: (f32, f32),
    /// `ItemBasedSteering.boosting`.
    boosting: bool,
    /// `ItemBasedSteering.boostTime`: the ticks the boost has run.
    boost_time: i32,
    /// The boost's total length (`DATA_BOOST_TIME`, the server's random 140..981).
    boost_time_total: i32,
    /// The rider's look of the previous tick (`tickRidden` reads `controller.getYRot()`).
    look: (f32, f32),
}

impl Pig {
    /// A pig at `pos` facing `yaw`, sized by the dimensions its registry entry carries, with
    /// the attributes the server last synced (the pig's base values as the fallback).
    pub fn new(pos: DVec3, yaw: f32, dims: (f32, f32), movement_speed: f64, gravity: f64) -> Pig {
        Pig {
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
            boosting: false,
            boost_time: 0,
            boost_time_total: 0,
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

    /// Vanilla `Pig.onSyncedDataUpdated` -> `ItemBasedSteering.onSynced`: a change of the boost
    /// total starts the boost from its first tick.
    pub fn on_synced_boost(&mut self, total: i32) {
        self.boosting = true;
        self.boost_time = 0;
        self.boost_time_total = total;
    }

    /// The boost's total from a `set_entity_data` metadata update (`DATA_BOOST_TIME`, a
    /// VarInt), if the update carries it.
    pub fn boost_total_of(entries: &[haste_core::entity::MetadataEntry]) -> Option<i32> {
        entries
            .iter()
            .find(|e| e.index == DATA_BOOST_TIME)
            .and_then(|e| match &e.value {
                MetadataValue::VarInt(v) => Some(*v),
                _ => None,
            })
    }

    /// One tick of the pig we steer, in vanilla `LivingEntity.aiStep`'s order for a ridden
    /// pig: the velocity thresholds, then `tickRidden` (the rotation follows the rider and the
    /// boost advances) and the shared `travelInAir` with `Pig`'s own input and speed.
    pub fn tick(&mut self, env: &PigEnv<'_>) {
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

    /// Vanilla `Pig.tickRidden`: the rotation follows the rider (the pitch halved) and the
    /// boost advances (`ItemBasedSteering.tickBoost`: the boost ends once its time passes the
    /// total).
    fn tick_ridden(&mut self) {
        self.pitch = self.look.1 * 0.5;
        self.yaw = self.look.0;
        if self.boosting {
            // `boostTime++ > total`: the time before this tick's increment is compared.
            let before = self.boost_time;
            self.boost_time += 1;
            if before > self.boost_time_total {
                self.boosting = false;
            }
        }
    }

    /// Whether the boost is running (for tests and the packet evidence).
    pub fn boosting(&self) -> bool {
        self.boosting
    }

    /// Vanilla `ItemBasedSteering.boostFactor`.
    fn boost_factor(&self) -> f32 {
        if self.boosting && self.boost_time_total > 0 {
            1.0 + BOOST_AMPLITUDE
                * (f64::from(self.boost_time) / f64::from(self.boost_time_total)
                    * std::f64::consts::PI)
                    .sin() as f32
        } else {
            1.0
        }
    }
}

impl crate::travel::RiddenAnimal for Pig {
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
    /// `getRiddenSpeed`: `movement_speed * 0.225 * boostFactor`.
    fn speed(&self) -> f32 {
        (self.movement_speed * RIDDEN_SPEED_FACTOR) as f32 * self.boost_factor()
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

/// What a pig tick reads from the world.
pub struct PigEnv<'a> {
    pub world: &'a dyn BlockView,
    pub shapes: &'a dyn CollisionShapes,
    pub blocks: &'a dyn BlockPhysics,
}

/// Whether a `minecraft:entity_type` registry path is the pig this module steers.
pub fn is_pig_type(path: &str) -> bool {
    path == "pig"
}

#[cfg(test)]
mod tests;
