//! Vanilla's `MobEffectInstance` and its `BlendState`: the nausea and darkness blend.
/// One active mob effect on us: the duration the server sent and vanilla's
/// `MobEffectInstance.BlendState`.
#[derive(Clone, Copy, Debug)]
pub struct Effect {
    /// Amplifier (level - 1).
    pub amplifier: i32,
    /// Ticks left; `-1` is infinite (`isInfiniteDuration`).
    pub duration: i32,
    /// `MobEffectInstance.BlendState.factor` and the one before this tick's, for the frame's
    /// `Mth.lerp(partialTicks, factorPreviousFrame, factor)`.
    factor: f32,
    prev_factor: f32,
    blend_in: i32,
    blend_out: i32,
    blend_out_advance: i32,
}

impl Effect {
    /// A new instance from `update_mob_effect`: the vanilla default blend durations are 0 and
    /// only nausea and darkness set them (`MobEffects`), which the caller resolves.
    pub fn new(
        amplifier: i32,
        duration: i32,
        blend: bool,
        blend_in: i32,
        blend_out: i32,
        blend_out_advance: i32,
    ) -> Effect {
        // A packet without the blend flag means `skipBlending`: the factor starts at its
        // target (`ClientPacketListener.handleUpdateMobEffect`).
        let instant = if blend { 0.0 } else { 1.0 };
        Effect {
            amplifier,
            duration,
            factor: instant,
            prev_factor: instant,
            blend_in,
            blend_out,
            blend_out_advance,
        }
    }

    /// Copies the previous instance's blend state (`LivingEntity.forceAddEffect` ->
    /// `newEffect.copyBlendState(previousEffect)`), so a refreshing packet does not restart the
    /// blend from scratch.
    pub fn copy_blend_from(&mut self, previous: &Effect) {
        self.factor = previous.factor;
        self.prev_factor = previous.prev_factor;
    }

    /// One client tick: `MobEffectInstance.tickClient` ticks the duration down, then the blend
    /// state eases the factor towards `!endsWithin(blendOutAdvance)`.
    pub fn tick(&mut self) {
        if self.duration > 0 {
            self.duration -= 1;
        }
        self.prev_factor = self.factor;
        let has_effect = !self.ends_within(self.blend_out_advance);
        let target = f32::from(has_effect);
        if self.factor != target {
            let blend_duration = if has_effect {
                self.blend_in
            } else {
                self.blend_out
            };
            if blend_duration == 0 {
                self.factor = target;
            } else {
                let max_delta = 1.0 / blend_duration as f32;
                self.factor += (target - self.factor).clamp(-max_delta, max_delta);
            }
        }
    }

    /// Vanilla's `MobEffectInstance.endsWithin`: an infinite duration never ends, and an ending
    /// one ends within `ticks` once its duration is at or below them.
    pub fn ends_within(&self, ticks: i32) -> bool {
        self.duration >= 0 && self.duration <= ticks
    }

    /// The blend factor between two frames (`MobEffectInstance.BlendState.getFactor`).
    pub fn blend_factor(&self, partial_tick: f32) -> f32 {
        self.prev_factor + (self.factor - self.prev_factor) * partial_tick.clamp(0.0, 1.0)
    }

    /// The frame's two endpoints of the blend (`BlendState.factorPreviousFrame`, `.factor`).
    pub fn blend_pair(&self) -> (f32, f32) {
        (self.prev_factor, self.factor)
    }
}
