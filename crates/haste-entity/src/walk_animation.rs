//! Vanilla `WalkAnimationState`: the walk cycle state `LivingEntity` tracks.

use crate::store::lerp;

/// Vanilla `WalkAnimationState`: the smoothed horizontal speed and the position that accumulates
/// it, which drive the limb swing. `LivingEntity.calculateEntityAnimation` feeds it the per-tick
/// horizontal distance x 4 (capped at 1) with a 0.4 smoothing factor
/// (`LivingEntity.updateWalkAnimation`); the renderer reads `position`/`speed` with the partial
/// tick.
#[derive(Clone, Copy, Debug)]
pub struct WalkAnimationState {
    speed_old: f32,
    speed: f32,
    position: f32,
    position_scale: f32,
}

impl Default for WalkAnimationState {
    fn default() -> Self {
        WalkAnimationState {
            speed_old: 0.0,
            speed: 0.0,
            position: 0.0,
            position_scale: 1.0,
        }
    }
}

impl WalkAnimationState {
    /// `WalkAnimationState.update`.
    pub fn update(&mut self, target_speed: f32, factor: f32, position_scale: f32) {
        self.speed_old = self.speed;
        self.speed += (target_speed - self.speed) * factor;
        self.position += self.speed;
        self.position_scale = position_scale;
    }

    /// `WalkAnimationState.stop`.
    pub fn stop(&mut self) {
        self.speed_old = 0.0;
        self.speed = 0.0;
        self.position = 0.0;
    }

    /// `WalkAnimationState.speed(partialTicks)`.
    pub fn speed(&self, partial: f32) -> f32 {
        lerp(partial, self.speed_old, self.speed).min(1.0)
    }

    /// `WalkAnimationState.position(partialTicks)`.
    pub fn position(&self, partial: f32) -> f32 {
        (self.position - self.speed * (1.0 - partial)) * self.position_scale
    }

    /// `WalkAnimationState.isMoving`.
    pub fn is_moving(&self) -> bool {
        self.speed > 1.0e-5
    }
}
