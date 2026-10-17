//! Remote-entity position smoothing: Haste's `LinearInterpolationHandler`.
//!
//! A remote entity that receives a move packet does not jump to the new position; it covers the
//! distance over the next 3 ticks (`LinearInterpolationHandler.DEFAULT_INTERPOLATION_STEPS`),
//! each tick travelling 1/`remaining` of what is left (`doInterpolate`). A new target arriving
//! mid-flight restarts the 3-tick window (`AbstractInterpolationHandler.interpolateTo`), unless
//! it is the target already in flight. `prev_pos`/`pos` bracket each tick's step so the renderer
//! can interpolate with the partial tick.
use haste_core::DVec3;

/// `LinearInterpolationHandler.DEFAULT_INTERPOLATION_STEPS`.
pub const INTERPOLATION_STEPS: u8 = 3;

pub(crate) struct LerpState {
    /// Where the entity is heading (blocks).
    target: DVec3,
    /// Ticks left in the current window; 0 = resting on `target`.
    remaining: u8,
}

impl LerpState {
    pub fn new(pos: DVec3) -> LerpState {
        LerpState {
            target: pos,
            remaining: 0,
        }
    }

    /// A short-delta move arrived (`delta` in blocks): the delta codec's base is the last
    /// target, so the new target is the old one plus the delta; restart the window.
    pub fn on_delta(&mut self, delta: DVec3) {
        self.on_target(self.target + delta);
    }

    /// An absolute target arrived (`entity_position_sync` within 64 blocks,
    /// `Entity.moveOrInterpolateTo`): head there over 3 steps. A target identical to the one
    /// already in flight does not restart the window (`AbstractInterpolationHandler.interpolateTo`
    /// checks `is` before `startInterpolating`).
    pub fn on_target(&mut self, pos: DVec3) {
        if self.remaining > 0 && self.target == pos {
            return;
        }
        self.target = pos;
        self.remaining = INTERPOLATION_STEPS;
    }

    /// `Entity.snapTo`: abandon any in-flight smoothing and rest on the new position.
    pub fn snap(&mut self, pos: DVec3) {
        self.target = pos;
        self.remaining = 0;
    }

    /// `InterpolationHandler.hasActiveInterpolation`.
    pub fn is_active(&self) -> bool {
        self.remaining > 0
    }

    /// One 20 Hz step (`Entity.commonTick`: old position recorded, then `interpolate`):
    /// brackets `pos` into `prev_pos`/`pos`.
    pub fn tick(&mut self, pos: &mut DVec3, prev_pos: &mut DVec3) {
        *prev_pos = *pos;
        if self.remaining > 0 {
            if self.remaining == 1 {
                *pos = self.target; // last step lands exactly
            } else {
                *pos += (self.target - *pos) / f64::from(self.remaining);
            }
            self.remaining -= 1;
        }
    }
}
