//! Active effects bookkeeping (player workstream): the `minecraft:mob_effect` indices the
//! movement tick, the lightmap and the screen effects need (resolved in
//! `MultiPlayerGameMode`'s data pass), the nausea blend factors the screen warp reads, and
//! one tick of each active `MobEffectInstance` (`LivingEntity.tickEffects`).

use super::Interaction;

#[derive(Clone, Copy, Debug)]
pub(super) struct EffectIds {
    pub(super) haste: i32,
    pub(super) conduit_power: i32,
    pub(super) mining_fatigue: i32,
    /// The two effects that blend, as vanilla's `MobEffects` builds them: nausea has a 150-tick
    /// fade in and a 20-tick fade out that starts 60 ticks early, darkness a 22-tick one.
    /// Their blend factors are what the screen effects read.
    pub(super) nausea: i32,
    pub(super) darkness: i32,
}

/// The `minecraft:mob_effect` indices the movement tick needs (levitation, slow falling and jump
/// boost), resolved from the game data. `None` until [`Interaction::prepare`] finds them, like
/// [`LightmapEffectIds`].
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MovementEffectIds {
    pub(crate) levitation: Option<i32>,
    pub(crate) slow_falling: Option<i32>,
    pub(crate) jump_boost: Option<i32>,
}

/// The `minecraft:mob_effect` indices the lightmap needs (night vision and darkness), resolved
/// from the game data. `None` until [`Interaction::prepare`] finds them (mod registries, or a
/// report without them, leave the lightmap without those effects).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LightmapEffectIds {
    pub(crate) night_vision: Option<i32>,
    pub(crate) darkness: Option<i32>,
    /// The blindness effect (fog environment `BlindnessFogEnvironment`).
    pub(crate) blindness: Option<i32>,
    /// The fire-resistance effect (lava fog distance `LavaFogEnvironment`).
    pub(crate) fire_resistance: Option<i32>,
}

impl Interaction {
    /// The `minecraft:mob_effect` index of nausea, the effect whose blend factor drives the
    /// nausea overlay and the screen warp (`MobEffects.NAUSEA`).
    pub(super) fn nausea_id(&self) -> Option<i32> {
        self.effect_ids.map(|e| e.nausea)
    }

    /// Whether `effect` is `MobEffects.DARKNESS`, the other effect with a blend duration.
    pub(super) fn darkness_id(&self, effect: i32) -> bool {
        self.effect_ids.is_some_and(|e| e.darkness == effect)
    }

    /// The nausea blend factor between two frames, 0 without the effect
    /// (`LivingEntity.getEffectBlendFactor`).
    pub fn nausea_factor(&self, partial_tick: f32) -> f32 {
        self.nausea_id()
            .and_then(|id| self.effects.get(&id))
            .map_or(0.0, |e| e.blend_factor(partial_tick))
    }

    /// The nausea blend factor pair (the previous frame's and this tick's), for the frame's
    /// lerp (`BlendState.factorPreviousFrame`, `.factor`).
    pub fn nausea_blend_pair(&self) -> (f32, f32) {
        self.nausea_id()
            .and_then(|id| self.effects.get(&id))
            .map_or((0.0, 0.0), |e| e.blend_pair())
    }

    /// Blind (vanilla `Player.isMobilityRestricted`: has `MobEffects.BLINDNESS`): no sprinting.
    pub fn is_blind(&self) -> bool {
        self.lightmap_effect_ids
            .blindness
            .is_some_and(|i| self.effects.contains_key(&i))
    }

    /// The movement effects the physics tick reads, from our active effects
    /// (`LivingEntity.hasEffect` / `getEffect(...).getAmplifier()`): the levitation and jump-boost
    /// amplifiers (`None` without the effect) and whether slow falling is on.
    pub fn movement_effects(&self) -> (Option<i32>, bool, Option<i32>) {
        let level = |id: Option<i32>| id.and_then(|i| self.effects.get(&i)).map(|e| e.amplifier);
        let ids = self.movement_effect_ids;
        (
            level(ids.levitation),
            ids.slow_falling
                .is_some_and(|i| self.effects.contains_key(&i)),
            level(ids.jump_boost),
        )
    }

    /// One tick of the active effects (`LivingEntity.tickEffects`: each instance ticks its
    /// duration and its vanilla blend state on the client).
    pub(crate) fn tick_effects(&mut self) {
        for e in self.effects.values_mut() {
            e.tick();
        }
    }
}
