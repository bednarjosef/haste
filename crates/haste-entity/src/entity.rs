//! `Entity`'s per-entity reads (yaw, pitch, body and head yaw between ticks).
//! The struct itself lives in `store.rs` next to the slots that hold it.

use haste_core::DVec3;

use crate::store::{Entity, DEATH_TICKS};
use crate::walk_swing::{lerp, rot_lerp, wrap_degrees};

impl Entity {
    /// `LivingEntity.isDeadOrDying` became true: `deathTime` starts counting on the next tick
    /// (`hurt_ticks` runs the 20-tick death counter down; see `animation::death_time`). A
    /// second trigger (event 3 after the health metadata, or the reverse) changes nothing.
    pub(crate) fn start_death(&mut self) {
        if !self.dead {
            self.dead = true;
            self.hurt_ticks = DEATH_TICKS;
        }
    }

    /// `Entity.getYRot(partialTicks)`: `Mth.rotLerp(partial, yRotO, yRot)`.
    pub fn yaw_at(&self, partial: f32) -> f32 {
        if partial == 1.0 {
            self.yaw
        } else {
            rot_lerp(partial, self.prev_yaw, self.yaw)
        }
    }

    /// `Entity.getXRot(partialTicks)`: `Mth.lerp(partial, xRotO, xRot)`.
    pub fn pitch_at(&self, partial: f32) -> f32 {
        if partial == 1.0 {
            self.pitch
        } else {
            lerp(partial, self.prev_pitch, self.pitch)
        }
    }

    /// `Entity.snapTo`: jump to the values with no interpolation (the old position and
    /// rotation move too, so no frame draws the gap) and drop any smoothing in flight.
    pub(crate) fn snap_to(&mut self, (pos, yaw, pitch): (DVec3, f32, f32)) {
        (self.pos, self.prev_pos) = (pos, pos);
        (self.yaw, self.prev_yaw) = (yaw, yaw);
        (self.pitch, self.prev_pitch) = (pitch, pitch);
        self.lerp.snap(pos, yaw, pitch);
    }

    /// `LinearInterpolationHandler.setInterpolationLength` (a display's teleport duration).
    pub(crate) fn set_interpolation_steps(&mut self, steps: u8) {
        self.lerp.set_steps(steps);
    }

    /// The present (pos, yaw, pitch): what `LerpState` keeps for the part a packet lacks.
    pub(crate) fn current(&self) -> (DVec3, f32, f32) {
        (self.pos, self.yaw, self.pitch)
    }

    /// Body yaw between the last two ticks (`Mth.rotLerp(partial, yBodyRotO, yBodyRot)`).
    pub fn body_yaw_at(&self, partial: f32) -> f32 {
        self.prev_body_yaw + partial * wrap_degrees(self.body_yaw - self.prev_body_yaw)
    }

    /// Head yaw between the last two ticks (`Mth.rotLerp(partial, yHeadRotO, yHeadRot)`).
    pub fn head_yaw_at(&self, partial: f32) -> f32 {
        self.prev_head_yaw + partial * wrap_degrees(self.head_yaw - self.prev_head_yaw)
    }
}
