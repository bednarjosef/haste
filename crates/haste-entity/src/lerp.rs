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
//!
//! A minecart on the new behaviour has no interpolation (`AbstractMinecart
//! .createInterpolationHandler` returns `InterpolationHandler.NO_OP`): its packets set the
//! position and rotation at once (`Entity.moveOrInterpolateTo`), and its smooth movement comes
//! from the track steps instead ([`crate::new_minecart_behavior`]).
use haste_core::DVec3;

use crate::store::{rot_lerp, Entity};

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
    /// `AbstractInterpolationHandler.interpolationSteps`: the window length, 3 for most
    /// entities; a display entity's comes from its teleport duration (0 snaps).
    steps: u8,
    /// `InterpolationHandler.NO_OP`: targets are never eased toward.
    no_op: bool,
}

impl LerpState {
    pub fn new(pos: DVec3, yaw: f32, pitch: f32) -> LerpState {
        LerpState {
            target: pos,
            target_yaw: yaw,
            target_pitch: pitch,
            remaining: 0,
            steps: INTERPOLATION_STEPS,
            no_op: false,
        }
    }

    /// Switches to `InterpolationHandler.NO_OP` (a minecart on the new behaviour): packets
    /// apply at once from now on.
    pub fn set_no_op(&mut self) {
        self.no_op = true;
        self.remaining = 0;
    }

    /// `LinearInterpolationHandler.setInterpolationLength`: the next window's length (the one
    /// in flight keeps its remaining steps).
    pub fn set_steps(&mut self, steps: u8) {
        self.steps = steps;
    }

    /// `AbstractInterpolationHandler.interpolateTo`: head for `pos` and/or `rot` over 3 steps.
    /// A part the packet lacks keeps the in-flight target, or the entity's `current` value when
    /// nothing is in flight. A target identical to the one already in flight does not restart
    /// the window (the `is` check before `startInterpolating`). With a zero-step window the
    /// handler does not ease at all: the target comes back for the caller to `snapTo`
    /// (position, yaw, pitch), and nothing stays in flight.
    pub fn interpolate_to(
        &mut self,
        pos: Option<DVec3>,
        rot: Option<(f32, f32)>,
        current: (DVec3, f32, f32),
    ) -> Option<(DVec3, f32, f32)> {
        let active = self.remaining > 0;
        let (cur_pos, cur_yaw, cur_pitch) = if active {
            (self.target, self.target_yaw, self.target_pitch)
        } else {
            current
        };
        let pos = pos.unwrap_or(cur_pos);
        let (yaw, pitch) = rot.unwrap_or((cur_yaw, cur_pitch));
        if self.steps == 0 {
            self.snap(pos, yaw, pitch);
            return Some((pos, yaw, pitch));
        }
        if active && self.target == pos && self.target_yaw == yaw && self.target_pitch == pitch {
            return None;
        }
        self.target = pos;
        self.target_yaw = yaw;
        self.target_pitch = pitch;
        self.remaining = self.steps;
        None
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

impl Entity {
    /// `Entity.moveOrInterpolateTo`: hand the packet's position and/or rotation to the
    /// interpolation (which may ask for a snap, for a zero-step window), or, for the no-op
    /// handler, set them at once. The no-op handler still keeps the position as the base later
    /// short-delta moves add to.
    pub(crate) fn move_or_interpolate_to(&mut self, pos: Option<DVec3>, rot: Option<(f32, f32)>) {
        if self.lerp.no_op {
            if let Some(pos) = pos {
                self.lerp.target = pos;
                self.pos = pos;
            }
            if let Some((yaw, pitch)) = rot {
                self.yaw = yaw;
                self.pitch = pitch;
            }
            return;
        }
        let current = (self.pos, self.yaw, self.pitch);
        if let Some(snap) = self.lerp.interpolate_to(pos, rot, current) {
            self.snap_to(snap);
        }
    }

    /// A short-delta move arrived (`delta` in blocks): the delta codec's base is the last
    /// target, so the new position is the old target plus the delta.
    pub(crate) fn move_by_delta(&mut self, delta: DVec3) {
        self.move_or_interpolate_to(Some(self.lerp.target + delta), None);
    }
}
