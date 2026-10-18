// The client's per-tick lightmap state, vanilla `LightmapRenderStateExtractor` and the
// boss-overlay/lightning inputs the packets feed it. The session owns one and ticks it; the
// render thread interpolates it by partial tick (`frame_view`).

use std::collections::HashMap;

use haste_protocol::packets::play::clientbound as cb;

/// The client's per-tick lightmap state: vanilla `LightmapRenderStateExtractor` (the
/// block-light flicker random walk, the effect-driven night vision and darkness) plus
/// `GameRenderer`'s boss-overlay world darkening and `ClientLevel`'s lightning sky flash. The
/// render thread interpolates it by partial tick (`frame_view`).
#[derive(Clone, Debug)]
pub struct LightmapState {
    /// `LightmapRenderStateExtractor.blockLightFlicker`.
    pub flicker: f32,
    rng: u64,
    /// Night-vision effect remaining ticks (`None` without it, `-1` infinite).
    pub night_vision: Option<i32>,
    /// Darkness effect remaining ticks.
    pub darkness: Option<i32>,
    /// `MobEffectInstance.BlendState` for the darkness effect (22 ticks in and out).
    pub darkness_blend: haste_render::lightmap::BlendState,
    /// Boss bars and their `(darkenScreen, createWorldFog)` flags (`BossEvent`), keyed by uuid
    /// bits.
    pub(super) boss_bars: HashMap<u128, (bool, bool)>,
    /// `GameRenderer.bossOverlayWorldDarkening` and its value one tick ago.
    pub boss_darkening: f32,
    pub boss_darkening_o: f32,
    /// `BossHealthOverlay.shouldCreateWorldFog()`: any bar asks for the thick boss fog.
    pub boss_world_fog: bool,
    /// `ClientLevel.skyFlashTime`: a lightning strike raises the sky for 2 ticks.
    pub sky_flash_time: i32,
    /// The player's tick count (vanilla `LivingEntity.tickCount`), for the darkness pulse.
    pub tick_count: i32,
}

impl Default for LightmapState {
    fn default() -> Self {
        Self {
            flicker: 0.0,
            // Vanilla seeds `RandomSource.create()` from a unique random seed; a fixed seed
            // keeps Haste's runs reproducible and the walk's values are not observable.
            rng: 0x9e37_79b9_7f4a_7c15,
            night_vision: None,
            darkness: None,
            darkness_blend: haste_render::lightmap::BlendState::default(),
            boss_bars: HashMap::new(),
            boss_darkening: 0.0,
            boss_darkening_o: 0.0,
            boss_world_fog: false,
            sky_flash_time: 0,
            tick_count: 0,
        }
    }
}

impl LightmapState {
    /// A uniform f32 in 0..1 (vanilla `RandomSource.nextFloat`; xorshift64*, seed fixed).
    fn next_float(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        ((x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32) / (1u32 << 24) as f32
    }

    /// One game tick: `LightmapRenderStateExtractor.tick` (flicker), the effect durations and
    /// darkness blend (`MobEffectInstance.tickClient`), the boss-overlay darkening
    /// (`GameRenderer.tick`) and the lightning flash timer (`ClientLevel.tick`).
    pub fn tick(&mut self) {
        let (r1, r2, r3, r4) = (
            self.next_float(),
            self.next_float(),
            self.next_float(),
            self.next_float(),
        );
        self.flicker =
            haste_render::lightmap::block_light_flicker_tick(self.flicker, r1, r2, r3, r4);
        // MobEffectInstance.tickClient -> tickDownDuration; a finite effect that reaches 0 is
        // removed (`LivingEntity.tickEffects`), an infinite one (-1) stays.
        for slot in [&mut self.night_vision, &mut self.darkness] {
            if let Some(d) = slot {
                if *d > 0 {
                    *d -= 1;
                    if *d == 0 {
                        *slot = None;
                    }
                }
            }
        }
        // MobEffectInstance.BlendState.tick: the darkness effect blends over 22 ticks and
        // starts blending out 22 ticks before it ends (MobEffects.DARKNESS).
        let has_darkness = matches!(self.darkness, Some(d) if d == -1 || d > 22);
        self.darkness_blend.tick(has_darkness, 22);
        // GameRenderer.tick: while any boss bar asks to darken the screen, rise by 0.05/tick
        // (capped at 1), else fall by 0.0125/tick.
        self.boss_darkening_o = self.boss_darkening;
        if self.boss_bars.values().any(|(darken, _)| *darken) {
            self.boss_darkening = (self.boss_darkening + 0.05).min(1.0);
        } else if self.boss_darkening > 0.0 {
            self.boss_darkening -= 0.0125;
        }
        // `BossHealthOverlay.shouldCreateWorldFog`: any bar asks for the thick world fog.
        self.boss_world_fog = self.boss_bars.values().any(|(_, fog)| *fog);
        if self.sky_flash_time > 0 {
            self.sky_flash_time -= 1;
        }
        self.tick_count = self.tick_count.wrapping_add(1);
    }

    /// An `update_mob_effect` for us: keep the duration of the night-vision or darkness effect.
    pub(super) fn update_effect(
        &mut self,
        ids: crate::player::interact::LightmapEffectIds,
        effect: i32,
        duration: i32,
    ) {
        if ids.night_vision == Some(effect) {
            self.night_vision = Some(duration);
        }
        if ids.darkness == Some(effect) {
            self.darkness = Some(duration);
        }
    }

    /// A `remove_mob_effect` for us.
    pub(super) fn remove_effect(
        &mut self,
        ids: crate::player::interact::LightmapEffectIds,
        effect: i32,
    ) {
        if ids.night_vision == Some(effect) {
            self.night_vision = None;
        }
        if ids.darkness == Some(effect) {
            self.darkness = None;
        }
    }

    /// A `boss_event`: `BossEvent` flags (bit 0x01 darkenScreen, bit 0x04 createWorldFog) on
    /// add/update flags.
    pub(super) fn boss_event(&mut self, id: u128, action: &cb::BossEventAction) {
        let flags = |f: u8| (f & 0x01 != 0, f & 0x04 != 0);
        match action {
            cb::BossEventAction::Add { flags: f, .. } => {
                self.boss_bars.insert(id, flags(*f));
            }
            cb::BossEventAction::Remove => {
                self.boss_bars.remove(&id);
            }
            cb::BossEventAction::UpdateFlags(f) => {
                if let Some(entry) = self.boss_bars.get_mut(&id) {
                    *entry = flags(*f);
                }
            }
            _ => {}
        }
    }

    /// `LightningBolt.tick` on the client: `ClientLevel.setSkyFlashTime(2)`.
    pub(super) fn lightning(&mut self) {
        self.sky_flash_time = 2;
    }
}
