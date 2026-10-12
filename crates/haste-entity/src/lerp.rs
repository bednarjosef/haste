//! Remote-entity position smoothing ("lerp steps").
//!
//! Observed vanilla behaviour (not decompiled code): a remote entity that receives a move packet
//! does not jump to the new position; it covers the distance over the next 3 ticks, each tick
//! travelling 1/`remaining` of what is left. A new packet arriving mid-flight extends the target
//! and restarts the 3-tick window, so under steady 20 Hz packets the entity trails the server
//! position by roughly 3 ticks. `prev_pos`/`pos` bracket each tick's step so the renderer can
//! interpolate with the partial tick.
use haste_core::DVec3;

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

    /// A short-delta move arrived (`delta` in blocks): extend the target, restart the window.
    pub fn on_delta(&mut self, delta: DVec3) {
        self.target += delta;
        self.remaining = 3;
    }

    /// A teleport arrived: abandon any in-flight smoothing and rest on the new position.
    pub fn on_teleport(&mut self, pos: DVec3) {
        self.target = pos;
        self.remaining = 0;
    }

    /// One 20 Hz step: brackets `pos` into `prev_pos`/`pos`.
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
