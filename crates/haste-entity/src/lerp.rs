//! Remote-entity position and rotation smoothing: Haste's `LinearInterpolationHandler`.
//!
//! A remote entity that receives a move or rotation packet does not jump to the new values; it
//! covers the distance over the next 3 ticks (`LinearInterpolationHandler.DEFAULT_INTERPOLATION_STEPS`),
//! each tick travelling 1/`remaining` of what is left (`doInterpolate`: position and pitch by
//! `Mth.lerp`, yaw by `Mth.rotLerp`). A new target arriving mid-flight restarts the 3-tick
//! window (`AbstractInterpolationHandler.interpolateTo`), unless it is the target already in
//! flight; a packet without position or without rotation keeps the in-flight value for the part
//! it lacks. `prev_*`/current pairs bracket each tick's step so the renderer can interpolate with
//! the partial tick (`Entity.getYRot(partialTicks)` / `getXRot(partialTicks)`).
use haste_core::DVec3;

use crate::store::rot_lerp;

/// `LinearInterpolationHandler.DEFAULT_INTERPOLATION_STEPS`.
pub const INTERPOLATION_STEPS: u8 = 3;

pub(crate) struct LerpState {
    /// Where the entity is heading (blocks).
    target: DVec3,
    /// The yaw and pitch it is turning to (degrees).
    target_yaw: f32,
    target_pitch: f32,
    /// Ticks left in the current window; 0 = resting on the targets.
    remaining: u8,
}

impl LerpState {
    pub fn new(pos: DVec3, yaw: f32, pitch: f32) -> LerpState {
        LerpState {
            target: pos,
            target_yaw: yaw,
            target_pitch: pitch,
            remaining: 0,
        }
    }

    /// A short-delta move arrived (`delta` in blocks): the delta codec's base is the last
    /// target, so the new target is the old one plus the delta. `current` is the entity's
    /// present (pos, yaw, pitch), the rotation to keep when no interpolation is in flight.
    pub fn on_delta(&mut self, delta: DVec3, current: (DVec3, f32, f32)) {
        self.interpolate_to(Some(self.target + delta), None, current);
    }

    /// `AbstractInterpolationHandler.interpolateTo`: head for `pos` and/or `rot` over 3 steps.
    /// A part the packet lacks keeps the in-flight target, or the entity's `current` value when
    /// nothing is in flight. A target identical to the one already in flight does not restart
    /// the window (the `is` check before `startInterpolating`).
    pub fn interpolate_to(
        &mut self,
        pos: Option<DVec3>,
        rot: Option<(f32, f32)>,
        current: (DVec3, f32, f32),
    ) {
        let active = self.remaining > 0;
        let (cur_pos, cur_yaw, cur_pitch) = if active {
            (self.target, self.target_yaw, self.target_pitch)
        } else {
            current
        };
        let pos = pos.unwrap_or(cur_pos);
        let (yaw, pitch) = rot.unwrap_or((cur_yaw, cur_pitch));
        if active && self.target == pos && self.target_yaw == yaw && self.target_pitch == pitch {
            return;
        }
        self.target = pos;
        self.target_yaw = yaw;
        self.target_pitch = pitch;
        self.remaining = INTERPOLATION_STEPS;
    }

    /// `Entity.snapTo`: abandon any in-flight smoothing and rest on the new values.
    pub fn snap(&mut self, pos: DVec3, yaw: f32, pitch: f32) {
        self.target = pos;
        self.target_yaw = yaw;
        self.target_pitch = pitch;
        self.remaining = 0;
    }

    /// One 20 Hz step (`Entity.commonTick`: `setOldPosAndRot`, then `interpolate`): brackets
    /// each value into its previous/current pair and takes one `doInterpolate` step.
    pub fn tick(&mut self, state: &mut LerpTarget<'_>) {
        *state.prev_pos = *state.pos;
        *state.prev_yaw = *state.yaw;
        *state.prev_pitch = *state.pitch;
        if self.remaining > 0 {
            let alpha = 1.0 / f32::from(self.remaining);
            if self.remaining == 1 {
                *state.pos = self.target; // last step lands exactly
            } else {
                *state.pos += (self.target - *state.pos) / f64::from(self.remaining);
            }
            *state.yaw = rot_lerp(alpha, *state.yaw, self.target_yaw);
            *state.pitch += alpha * (self.target_pitch - *state.pitch);
            self.remaining -= 1;
        }
    }
}

/// The entity fields one `LerpState::tick` advances.
pub(crate) struct LerpTarget<'a> {
    pub pos: &'a mut DVec3,
    pub prev_pos: &'a mut DVec3,
    pub yaw: &'a mut f32,
    pub prev_yaw: &'a mut f32,
    pub pitch: &'a mut f32,
    pub prev_pitch: &'a mut f32,
}
