//! Vanilla `LivingEntity.SwingState`: the hand swing the `swing_animation` packets drive.

use crate::store::lerp;

/// `LivingEntity.SwingState`: which hand swings, with which animation type and duration, and how
/// far the swing has progressed. `swing_animation` packets start it
/// (`ClientPacketListener.handleSwingAnimation` -> `LivingEntity.swing`).
#[derive(Clone, Copy, Debug, Default)]
pub struct SwingState {
    pub(crate) active: bool,
    pub(crate) hand: i32,
    pub(crate) animation: i32,
    pub(crate) duration: i32,
    pub(crate) ticks: i32,
    pub(crate) progress: f32,
    pub(crate) old_progress: f32,
}

impl SwingState {
    /// `LivingEntity.swing` -> `SwingState.startIfAble`.
    pub fn start(&mut self, hand: i32, animation: i32, duration: i32) {
        if self.active && self.ticks > 0 && self.ticks <= self.duration / 2 {
            return;
        }
        if !self.active
            || self.hand != hand
            || self.animation != animation
            || self.duration != duration
        {
            self.old_progress = 0.0;
            self.progress = 0.0;
        }
        self.hand = hand;
        self.animation = animation;
        self.duration = duration;
        self.active = true;
        self.ticks = 0;
    }

    /// `SwingState.tick`.
    pub fn tick(&mut self) {
        self.old_progress = self.progress;
        if self.active {
            if self.duration > 0 {
                self.progress = (self.ticks as f32 / self.duration as f32).min(1.0);
            }
            if self.ticks > self.duration {
                self.active = false;
                self.progress = 0.0;
            } else {
                self.ticks += 1;
            }
        } else {
            self.progress = 0.0;
        }
    }

    /// `LivingEntity.getSwingAnimation` -> `SwingState.getAnimation`.
    pub fn animation(&self, partial: f32) -> f32 {
        if self.progress >= self.old_progress {
            lerp(partial, self.old_progress, self.progress)
        } else {
            lerp(partial, self.old_progress, self.progress + 1.0)
        }
    }

    pub fn is_swinging(&self) -> bool {
        self.active
    }

    /// `SwingState.animation` (the raw progress, used by `LivingEntity.tick`'s body yaw).
    pub fn progress(&self) -> f32 {
        self.progress
    }

    /// The swinging hand (0 main, 1 off).
    pub fn hand(&self) -> i32 {
        self.hand
    }

    /// The swing animation type (0 none, 1 whack, 2 stab).
    pub fn animation_type(&self) -> i32 {
        self.animation
    }
}
