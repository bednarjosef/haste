// Distance fog and the fog colour, matching vanilla's `FogRenderer.setupFog` /
// `FogRenderer.computeFogColor` and the `environment/*FogEnvironment` classes exactly.
//
// Vanilla keeps a fixed list of fog environments and picks two of them each frame: the first
// applicable one that `providesColor` gives the colour, and the first applicable one that
// `modifiesDarkness` adjusts the void darkness. `setupFog` additionally runs the first
// applicable environment's own distance setup (which is not necessarily the colour source: a
// blindness effect, for instance, sets the distances while the fluid or the atmosphere still
// provides the colour). The list order is Lava, PowderSnow, Blindness, Darkness, Water,
// Atmospheric.
//
// Vanilla splits fog distances into two bands, both linear:
// - the environmental band, set by the selected environment (biome/dimension attributes for
//   the atmosphere, fixed ramps for lava/powder snow, the effect ramp for blindness/darkness,
//   the biome water ramp for water) measured on the spherical distance, and
// - the render-distance band, measured on the cylindrical distance (max of horizontal
//   distance and |y|): start = render_distance_blocks - clamp(render_distance_blocks/10,
//   4, 64), end = render_distance_blocks.
// The fragment fog value is the max of both bands (fog.glsl `apply_fog`).
//
// The fog colour is the selected environment's base colour, then: the void darkness term
// (`FogRenderer.computeFogColor`, square of 1 - darkness, skipped in lava and powder snow),
// the boss-overlay world darkening, and the water-vision / night-vision brightening.
//
// The timeline tables (night fog/sky multipliers, sunrise/sunset colour keyframes) are
// re-typed from vanilla's overworld day timeline: piecewise linear between integer-tick
// keyframes over the 24000-tick day, values as plain floats (vanilla quantizes colours to
// 8 bit per channel only when packing into int, and lerps in float; the mix chain here
// stays float like vanilla's). They only apply to a dimension whose timeline is the
// overworld day (`day_timeline`).

use crate::SkyState;

/// `ARGB.vector3fFromRGB24` of an int colour (0xRRGGBB, alpha ignored) as sRGB floats.
pub const fn rgb24(c: i32) -> [f32; 3] {
    let c = c as u32;
    [
        ((c >> 16) & 0xFF) as f32 / 255.0,
        ((c >> 8) & 0xFF) as f32 / 255.0,
        (c & 0xFF) as f32 / 255.0,
    ]
}

/// Overworld dimension fog colour (`DimensionTypes` overworld `FOG_COLOR` 0xC0D8FF).
pub const OVERWORLD_FOG_COLOR: [f32; 3] = rgb24(-4138753);
/// Overworld sky colour (`OverworldBiomes.calculateSkyColor(0.8)` = 0x78A7FF).
pub const OVERWORLD_SKY_COLOR: [f32; 3] = [
    0x78 as f32 / 255.0,
    0xA7 as f32 / 255.0,
    0xFF as f32 / 255.0,
];
/// Lava fog colour (`LavaFogEnvironment.COLOR`, `ARGB.vector3fFromRGB24(-6743808)`).
pub const LAVA_FOG_COLOR: [f32; 3] = rgb24(-6743808);
/// Powdered-snow fog colour (`PowderedSnowFogEnvironment.COLOR`).
pub const POWDER_SNOW_FOG_COLOR: [f32; 3] = rgb24(-6308916);
/// Default water fog colour (`EnvironmentAttributes.WATER_FOG_COLOR` 0xFF04FDB3) used when the
/// biome does not override it.
pub const WATER_FOG_COLOR: [f32; 3] = rgb24(-16448205);
/// Default `WATER_FOG_START_DISTANCE` / `WATER_FOG_END_DISTANCE` attributes.
pub const WATER_FOG_START: f32 = -8.0;
pub const WATER_FOG_END: f32 = 96.0;

/// Vanilla `FogType` after `FogRenderer.getFogType` (`NONE` becomes `ATMOSPHERIC`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FogType {
    #[default]
    Atmospheric,
    Water,
    Lava,
    PowderSnow,
}

/// The environment `FogRenderer.setupFog` selected (the first applicable one), for logging and
/// the fog-distance band the shader uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FogEnvironment {
    Lava,
    PowderSnow,
    Blindness,
    Darkness,
    Water,
    Atmospheric,
}

/// The dimension (and biome) environment attributes the atmospheric fog reads. Vanilla resolves
/// these through `Camera.attributeProbe()` from the dimension type overlaid by the biome; the
/// client builds this from the dimension defaults and the camera biome's overrides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DimensionFog {
    /// `visual/fog_color` (biome/dimension `FOG_COLOR`).
    pub fog_color: [f32; 3],
    /// `visual/sky_color`.
    pub sky_color: [f32; 3],
    /// `visual/fog_start_distance` / `visual/fog_end_distance` (blocks).
    pub fog_start: f32,
    pub fog_end: f32,
    /// `visual/sky_fog_end_distance` (blocks).
    pub sky_fog_end: f32,
    /// `visual/cloud_fog_end_distance` (blocks).
    pub cloud_fog_end: f32,
    /// Whether the dimension runs the overworld day timeline (the night fog/sky multipliers and
    /// the sunrise/sunset tint). False for the Nether and the End.
    pub day_timeline: bool,
}

impl DimensionFog {
    pub fn overworld() -> Self {
        DimensionFog {
            fog_color: OVERWORLD_FOG_COLOR,
            sky_color: OVERWORLD_SKY_COLOR,
            fog_start: 0.0,
            fog_end: 1024.0,
            sky_fog_end: 512.0,
            cloud_fog_end: 2048.0,
            day_timeline: true,
        }
    }

    /// `DimensionTypes` nether: `FOG_START_DISTANCE` 10, `FOG_END_DISTANCE` 96; no fog colour
    /// (the nether biomes each set `FOG_COLOR`, so the default is black).
    pub fn nether() -> Self {
        DimensionFog {
            fog_color: [0.0, 0.0, 0.0],
            sky_color: [0.0, 0.0, 0.0],
            fog_start: 10.0,
            fog_end: 96.0,
            sky_fog_end: 512.0,
            cloud_fog_end: 2048.0,
            day_timeline: false,
        }
    }

    /// `DimensionTypes` end: `FOG_COLOR` -15199464, `SKY_COLOR` black.
    pub fn end() -> Self {
        DimensionFog {
            fog_color: rgb24(-15199464),
            sky_color: [0.0, 0.0, 0.0],
            fog_start: 0.0,
            fog_end: 1024.0,
            sky_fog_end: 512.0,
            cloud_fog_end: 2048.0,
            day_timeline: false,
        }
    }

    /// The camera biome's environment-attribute overrides (`BiomeClimate`'s fog fields).
    pub fn with_biome(mut self, biome: &BiomeFog) -> Self {
        if let Some(c) = biome.fog_color {
            self.fog_color = rgb24(c as i32);
        }
        if let Some(c) = biome.sky_color {
            self.sky_color = rgb24(c as i32);
        }
        if let Some(d) = biome.fog_start {
            self.fog_start = d;
        }
        if let Some(d) = biome.fog_end {
            self.fog_end = d;
        }
        if let Some(d) = biome.sky_fog_end {
            self.sky_fog_end = d;
        }
        if let Some(d) = biome.cloud_fog_end {
            self.cloud_fog_end = d;
        }
        self
    }
}

/// The fog-relevant environment attributes of one biome registry entry (the client parses them
/// from the synced registry NBT's `attributes` map). `None` keeps the dimension default.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BiomeFog {
    pub fog_color: Option<u32>,
    pub sky_color: Option<u32>,
    pub fog_start: Option<f32>,
    pub fog_end: Option<f32>,
    pub sky_fog_end: Option<f32>,
    pub cloud_fog_end: Option<f32>,
    pub water_fog_color: Option<u32>,
    pub water_fog_start: Option<f32>,
    pub water_fog_end: Option<f32>,
}

/// Everything `FogRenderer.setupFog` / `computeFogColor` needs beyond the sky state and the
/// camera position: what the camera is in, the player's effects and state, the dimension/biome
/// attributes and the boss-overlay values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogInputs {
    pub fog_type: FogType,
    pub dimension: DimensionFog,
    pub water_fog_color: [f32; 3],
    pub water_fog_start: f32,
    pub water_fog_end: f32,
    /// `LocalPlayer.getWaterVision()` (0 when the eye is not in water).
    pub water_vision: f32,
    pub spectator: bool,
    pub fire_resistance: bool,
    /// Remaining blindness ticks (`-1` infinite), `None` without the effect.
    pub blindness: Option<i32>,
    /// Remaining darkness ticks (`-1` infinite), `None` without the effect.
    pub darkness: Option<i32>,
    /// The darkness effect's `MobEffectInstance.BlendState` factor (0..1).
    pub darkness_blend: f32,
    /// Remaining night-vision ticks (`-1` infinite), `None` without the effect.
    pub night_vision: Option<i32>,
    /// `BossHealthOverlay.shouldCreateWorldFog()`.
    pub boss_world_fog: bool,
    /// `GameRenderer.bossOverlayWorldDarkening(partialTick)`.
    pub darken_world_amount: f32,
    /// `LevelData.voidDarknessOnsetRange()` (32, or 1 on a superflat world).
    pub void_darkness_onset_range: f32,
    /// `ClientLevel.getRainLevel(partialTick)` (0..1): darkens the sky colour mixed into
    /// the fog, and (with the biome's precipitation and the sky light at the camera) pulls
    /// the environmental fog bands in (`AtmosphericFogEnvironment.updateRainFogState`).
    pub rain_level: f32,
    /// `ClientLevel.getThunderLevel(partialTick)` (0..1, already scaled by the rain level):
    /// the sky colour's extra thunder darkening.
    pub thunder_level: f32,
    /// `AtmosphericFogEnvironment.rainFogMultiplier` (0..1, eased per frame by the client):
    /// rain level x sky-light factor x biome precipitation, pulling the environmental fog
    /// band in by -160/-256 blocks per unit.
    pub rain_fog_multiplier: f32,
}

impl Default for FogInputs {
    fn default() -> Self {
        FogInputs {
            fog_type: FogType::Atmospheric,
            dimension: DimensionFog::overworld(),
            water_fog_color: WATER_FOG_COLOR,
            water_fog_start: WATER_FOG_START,
            water_fog_end: WATER_FOG_END,
            water_vision: 0.0,
            spectator: false,
            fire_resistance: false,
            blindness: None,
            darkness: None,
            darkness_blend: 0.0,
            night_vision: None,
            boss_world_fog: false,
            darken_world_amount: 0.0,
            void_darkness_onset_range: 32.0,
            rain_level: 0.0,
            thunder_level: 0.0,
            rain_fog_multiplier: 0.0,
        }
    }
}

/// Vanilla `FogData`: the two fog bands plus the sky and cloud fades.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogData {
    pub environmental_start: f32,
    pub environmental_end: f32,
    pub render_distance_start: f32,
    pub render_distance_end: f32,
    pub sky_end: f32,
    pub cloud_end: f32,
    pub environment: FogEnvironment,
}

/// `FogEnvironment.isApplicable` for each environment in the vanilla list order.
fn applicable(f: &FogInputs, env: FogEnvironment) -> bool {
    match env {
        FogEnvironment::Lava => f.fog_type == FogType::Lava,
        FogEnvironment::PowderSnow => f.fog_type == FogType::PowderSnow,
        FogEnvironment::Blindness => f.blindness.is_some(),
        FogEnvironment::Darkness => f.darkness.is_some(),
        FogEnvironment::Water => f.fog_type == FogType::Water,
        FogEnvironment::Atmospheric => f.fog_type == FogType::Atmospheric,
    }
}

/// The first environment in vanilla's list that is applicable (`FogRenderer.setupFog` breaks at
/// the first, `computeFogColor` scans for the first colour source and the first darkness
/// modifier).
fn first_applicable(f: &FogInputs) -> FogEnvironment {
    [
        FogEnvironment::Lava,
        FogEnvironment::PowderSnow,
        FogEnvironment::Blindness,
        FogEnvironment::Darkness,
        FogEnvironment::Water,
        FogEnvironment::Atmospheric,
    ]
    .into_iter()
    .find(|e| applicable(f, *e))
    .expect("the atmospheric environment always applies")
}

/// The first applicable environment that `providesColor` (lava, powder snow, water and the
/// atmosphere; blindness and darkness do not).
fn color_source(f: &FogInputs) -> FogEnvironment {
    match f.fog_type {
        FogType::Lava => FogEnvironment::Lava,
        FogType::PowderSnow => FogEnvironment::PowderSnow,
        FogType::Water => FogEnvironment::Water,
        FogType::Atmospheric => FogEnvironment::Atmospheric,
    }
}

/// Vanilla `Mth.lerp(delta, start, end)`.
fn lerp(start: f32, end: f32, t: f32) -> f32 {
    start + (end - start) * t
}

/// Vanilla `Mth.clampedLerp`: factor below 0 gives min, above 1 gives max.
fn clamped_lerp(factor: f32, min: f32, max: f32) -> f32 {
    if factor < 0.0 {
        min
    } else if factor > 1.0 {
        max
    } else {
        min + factor * (max - min)
    }
}

/// Vanilla `FogRenderer.setupFog`: pick the environment's distances and the two bands.
/// `view_distance_chunks` is the effective render distance (server-capped).
pub fn setup_fog(view_distance_chunks: i32, f: &FogInputs) -> FogData {
    let rd = (view_distance_chunks.max(2) * 16) as f32;
    let env = first_applicable(f);
    let mut data = FogData {
        environmental_start: 0.0,
        environmental_end: 1024.0,
        render_distance_start: 0.0,
        render_distance_end: rd,
        sky_end: rd,
        cloud_end: rd,
        environment: env,
    };
    match env {
        // `LavaFogEnvironment.setupFog`: spectator sees far, fire resistance close, else the
        // vanilla 0.25..1 ramp.
        FogEnvironment::Lava => {
            if f.spectator {
                data.environmental_start = -8.0;
                data.environmental_end = rd * 0.5;
            } else if f.fire_resistance {
                data.environmental_start = 0.0;
                data.environmental_end = 5.0;
            } else {
                data.environmental_start = 0.25;
                data.environmental_end = 1.0;
            }
            data.sky_end = data.environmental_end;
            data.cloud_end = data.environmental_end;
        }
        // `PowderedSnowFogEnvironment.setupFog`.
        FogEnvironment::PowderSnow => {
            if f.spectator {
                data.environmental_start = -8.0;
                data.environmental_end = rd * 0.5;
            } else {
                data.environmental_start = 0.0;
                data.environmental_end = 2.0;
            }
            data.sky_end = data.environmental_end;
            data.cloud_end = data.environmental_end;
        }
        // `BlindnessFogEnvironment.setupFog`: 5 blocks (or 5..render distance by the last
        // second of the effect).
        FogEnvironment::Blindness => {
            let d = f.blindness.unwrap_or(-1);
            let distance = if d == -1 {
                5.0
            } else {
                lerp(rd, 5.0, (d as f32 / 20.0).min(1.0))
            };
            data.environmental_start = distance * 0.25;
            data.environmental_end = distance;
            data.sky_end = distance * 0.8;
            data.cloud_end = distance * 0.8;
        }
        // `DarknessFogEnvironment.setupFog`: render distance to 15 blocks by the blend factor.
        FogEnvironment::Darkness => {
            let distance = lerp(rd, 15.0, f.darkness_blend);
            data.environmental_start = distance * 0.75;
            data.environmental_end = distance;
            data.sky_end = distance;
            data.cloud_end = distance;
        }
        // `WaterFogEnvironment.setupFog`: the biome water ramp, shortened by the water vision.
        FogEnvironment::Water => {
            data.environmental_start = f.water_fog_start;
            data.environmental_end = f.water_fog_end * f.water_vision.max(0.25);
            data.sky_end = data.environmental_end;
            data.cloud_end = data.environmental_end;
        }
        // `AtmosphericFogEnvironment.setupFog`.
        FogEnvironment::Atmospheric => {
            data.environmental_start = f.dimension.fog_start;
            data.environmental_end = f.dimension.fog_end;
            // Rain pulls the environmental band in: start -160, end -256 per unit of the
            // eased rain fog multiplier (end floored at min(96, fogEnd)).
            data.environmental_start += -160.0 * f.rain_fog_multiplier;
            let min_end = 96.0f32.min(data.environmental_end);
            data.environmental_end =
                (data.environmental_end + -256.0 * f.rain_fog_multiplier).max(min_end);
            data.sky_end = rd.min(f.dimension.sky_fog_end);
            data.cloud_end = 2048.0f32.min(f.dimension.cloud_fog_end);
            if f.boss_world_fog {
                data.environmental_start = data.environmental_start.min(10.0);
                data.environmental_end = data.environmental_end.min(96.0);
                data.sky_end = data.environmental_end;
                data.cloud_end = data.environmental_end;
            }
        }
    }
    let span = (rd / 10.0).clamp(4.0, 64.0);
    data.render_distance_start = rd - span;
    data.render_distance_end = rd;
    data
}

/// The fog fragment factor: max of the two linear bands (fog.glsl `total_fog_value`).
#[inline]
pub fn fog_value(spherical: f32, cylindrical: f32, fog: &FogData) -> f32 {
    let linear = |d, start, end| {
        if d <= start {
            0.0
        } else if d >= end {
            1.0
        } else {
            (d - start) / (end - start)
        }
    };
    linear(spherical, fog.environmental_start, fog.environmental_end).max(linear(
        cylindrical,
        fog.render_distance_start,
        fog.render_distance_end,
    ))
}

/// Piecewise-linear timeline sampling over the 24000-tick day: segments between consecutive
/// keyframes plus the wrap segment from the last keyframe back to the first (vanilla
/// `KeyframeTrackSampler`: at a keyframe tick the keyframe's value, between them linear by
/// tick, `ticks` taken mod the period).
fn timeline_lerp<T, L>(keyframes: &[(i32, T)], ticks: i64, lerp: L) -> T
where
    T: Copy,
    L: Fn(T, T, f32) -> T,
{
    let t = ticks.rem_euclid(24000) as i32;
    let first = keyframes[0];
    let last = keyframes[keyframes.len() - 1];
    if t < first.0 || t > last.0 {
        // Wrap segment: `last` at its tick to `first` one period later.
        let span = 24000 + first.0 - last.0;
        let at = if t > last.0 { t } else { t + 24000 };
        let alpha = (at - last.0) as f32 / span as f32;
        return lerp(last.1, first.1, alpha);
    }
    for w in keyframes.windows(2) {
        if t <= w[1].0 {
            let span = w[1].0 - w[0].0;
            if span <= 0 {
                return w[1].1;
            }
            let alpha = (t - w[0].0) as f32 / span as f32;
            return lerp(w[0].1, w[1].1, alpha);
        }
    }
    last.1
}

fn lerp3(a: [f32; 3], b: [f32; 3], alpha: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * alpha,
        a[1] + (b[1] - a[1]) * alpha,
        a[2] + (b[2] - a[2]) * alpha,
    ]
}

fn lerp4(a: [f32; 4], b: [f32; 4], alpha: f32) -> [f32; 4] {
    [
        a[0] + (b[0] - a[0]) * alpha,
        a[1] + (b[1] - a[1]) * alpha,
        a[2] + (b[2] - a[2]) * alpha,
        a[3] + (b[3] - a[3]) * alpha,
    ]
}

fn timeline_sample3(keyframes: &[(i32, [f32; 3])], ticks: i64) -> [f32; 3] {
    timeline_lerp(keyframes, ticks, lerp3)
}

fn timeline_sample4(keyframes: &[(i32, [f32; 4])], ticks: i64) -> [f32; 4] {
    timeline_lerp(keyframes, ticks, lerp4)
}

/// Fog colour night multiplier (timeline `fog_color` MULTIPLY_RGB track: white by day,
/// (0.06, 0.06, 0.09) at night, ramps 11867-13670 and 22330-24000+133).
const FOG_COLOR_NIGHT: [(i32, [f32; 3]); 4] = [
    (133, [1.0, 1.0, 1.0]),
    (11867, [1.0, 1.0, 1.0]),
    (13670, [0.06, 0.06, 0.09]),
    (22330, [0.06, 0.06, 0.09]),
];

/// Sky colour night multiplier (same shape, black at night).
const SKY_COLOR_NIGHT: [(i32, [f32; 3]); 4] = [
    (133, [1.0, 1.0, 1.0]),
    (11867, [1.0, 1.0, 1.0]),
    (13670, [0.0, 0.0, 0.0]),
    (22330, [0.0, 0.0, 0.0]),
];

/// Sunrise/sunset colour (timeline `sunrise_sunset_color` track, ARGB as floats). The
/// int constants are vanilla's packed ARGB keyframe values.
const SUNRISE_SUNSET_COLOR: [(i32, [f32; 4]); 32] = [
    (71, argb(1609540403)),
    (310, argb(703969843)),
    (565, argb(117167155)),
    (730, argb(16770355)),
    (11270, argb(16770355)),
    (11397, argb(83679283)),
    (11522, argb(268028723)),
    (11690, argb(703969843)),
    (11929, argb(1609540403)),
    (12243, argb((-1310226637i32) as u32)),
    (12358, argb((-857440717i32) as u32)),
    (12512, argb((-371166669i32) as u32)),
    (12613, argb((-153261261i32) as u32)),
    (12732, argb((-19242189i32) as u32)),
    (12841, argb((-19440589i32) as u32)),
    (13035, argb((-321760973i32) as u32)),
    (13252, argb((-1043577037i32) as u32)),
    (13775, argb(918435635)),
    (13888, argb(532362547)),
    (14039, argb(163001139)),
    (14192, argb(11744051)),
    (21807, argb(11678515)),
    (21961, argb(163001139)),
    (22112, argb(532362547)),
    (22225, argb(918435635)),
    (22748, argb((-1043577037i32) as u32)),
    (22965, argb((-321760973i32) as u32)),
    (23159, argb((-19440589i32) as u32)),
    (23272, argb((-19242189i32) as u32)),
    (23488, argb((-371166669i32) as u32)),
    (23642, argb((-857440717i32) as u32)),
    (23757, argb((-1310226637i32) as u32)),
];

/// ARGB int to [r, g, b, a] floats (vanilla `ARGB.vector4fFromARGB32`).
const fn argb(c: u32) -> [f32; 4] {
    [
        ((c >> 16) & 0xFF) as f32 / 255.0,
        ((c >> 8) & 0xFF) as f32 / 255.0,
        (c & 0xFF) as f32 / 255.0,
        ((c >> 24) & 0xFF) as f32 / 255.0,
    ]
}

/// The sunrise/sunset colour this tick (sRGB floats, alpha = strength).
pub fn sunrise_sunset_color(time_of_day: i64) -> [f32; 4] {
    timeline_sample4(&SUNRISE_SUNSET_COLOR, time_of_day)
}

/// The overworld sky-dome colour this tick (sRGB): 0x78A7FF with the timeline's night
/// multiplier, then the weather layers (`WeatherAttributes.RAIN`/`THUNDER` over SKY_COLOR,
/// `ColorModifier.BLEND_TO_GRAY_RGB`): rain lerps toward the greyscale times 0.6 by 0.75
/// weighted by the rain level minus the thunder level, thunder toward the greyscale times
/// 0.24 by 0.94 weighted by the thunder level.
pub fn overworld_sky_color(time_of_day: i64, rain_level: f32, thunder_level: f32) -> [f32; 3] {
    let mut sky = overworld_sky_color_clear(time_of_day);
    let rain_only = (rain_level - thunder_level).clamp(0.0, 1.0);
    let blend = |col: [f32; 3], brightness: f32, factor: f32| {
        let grey = col[0] * 0.3 + col[1] * 0.59 + col[2] * 0.11;
        [
            col[0] + (grey * brightness - col[0]) * factor,
            col[1] + (grey * brightness - col[1]) * factor,
            col[2] + (grey * brightness - col[2]) * factor,
        ]
    };
    if rain_only > 0.0 {
        let target = blend(sky, 0.6, 0.75);
        sky = [
            sky[0] + (target[0] - sky[0]) * rain_only,
            sky[1] + (target[1] - sky[1]) * rain_only,
            sky[2] + (target[2] - sky[2]) * rain_only,
        ];
    }
    if thunder_level > 0.0 {
        let target = blend(sky, 0.24, 0.94);
        sky = [
            sky[0] + (target[0] - sky[0]) * thunder_level,
            sky[1] + (target[1] - sky[1]) * thunder_level,
            sky[2] + (target[2] - sky[2]) * thunder_level,
        ];
    }
    sky
}

/// The clear-weather overworld sky-dome colour this tick (sRGB): 0x78A7FF with the
/// timeline's night multiplier.
fn overworld_sky_color_clear(time_of_day: i64) -> [f32; 3] {
    let night = timeline_sample3(&SKY_COLOR_NIGHT, time_of_day);
    [
        OVERWORLD_SKY_COLOR[0] * night[0],
        OVERWORLD_SKY_COLOR[1] * night[1],
        OVERWORLD_SKY_COLOR[2] * night[2],
    ]
}

/// Direction of the sun's horizontal glow for the fog tint: +1 east (morning) or -1 west
/// (afternoon), from vanilla's `sunX = sin(sunAngle) > 0 ? -1 : 1` — the sun angle is 0 at
/// noon and 180 degrees at midnight, so its sine is positive exactly on the noon-to-midnight
/// half where the sun sets in the west.
pub fn sun_glow_side(time_of_day: i64) -> f32 {
    let t = (time_of_day.rem_euclid(24000)) as i32;
    if (6000..18000).contains(&t) {
        -1.0
    } else {
        1.0
    }
}

/// `AtmosphericFogEnvironment.getBaseColor`: the dimension/biome fog colour with the day
/// timeline's night multiplier and sunrise/sunset tint, mixed toward the sky colour by a
/// factor that widens as the render distance shrinks.
fn atmospheric_base(
    f: &FogInputs,
    time_of_day: i64,
    view_distance_chunks: i32,
    camera_forward: [f32; 3],
) -> [f32; 3] {
    let dim = &f.dimension;
    let mut fog = dim.fog_color;
    if dim.day_timeline {
        let night = timeline_sample3(&FOG_COLOR_NIGHT, time_of_day);
        fog = [fog[0] * night[0], fog[1] * night[1], fog[2] * night[2]];
        // Sunrise/sunset tint when looking at the sun (render distance >= 4 chunks).
        if view_distance_chunks >= 4 {
            let side = sun_glow_side(time_of_day);
            let looking = camera_forward[0] * side;
            if looking > 0.0 {
                let glow = sunrise_sunset_color(time_of_day);
                if glow[3] > 0.0 {
                    fog = lerp3(fog, [glow[0], glow[1], glow[2]], looking * glow[3]);
                }
            }
        }
    }
    // The FOG_COLOR weather layers (`WeatherAttributes.RAIN`/`THUNDER`, MULTIPLY_RGB):
    // rain multiplies (0.5, 0.5, 0.6) by the rain level minus the thunder level, thunder
    // (0.25, 0.25, 0.3) by the thunder level — the attribute system applies them to the
    // probed fog colour before `AtmosphericFogEnvironment.getBaseColor` mixes toward the
    // sky.
    {
        let rain_only = (f.rain_level - f.thunder_level).clamp(0.0, 1.0);
        if rain_only > 0.0 {
            let m = 1.0 - rain_only * 0.5;
            let mb = 1.0 - rain_only * 0.4;
            fog = [fog[0] * m, fog[1] * m, fog[2] * mb];
        }
        if f.thunder_level > 0.0 {
            let m = 1.0 - f.thunder_level * 0.75;
            let mb = 1.0 - f.thunder_level * 0.7;
            fog = [fog[0] * m, fog[1] * m, fog[2] * mb];
        }
    }
    // Mix toward the sky colour, more the shorter the render distance.
    let mut sky = dim.sky_color;
    if dim.day_timeline {
        let night = timeline_sample3(&SKY_COLOR_NIGHT, time_of_day);
        sky = [sky[0] * night[0], sky[1] * night[1], sky[2] * night[2]];
    }
    // `AtmosphericFogEnvironment.applyWeatherDarken`: rain scales r/g down 0.5 and b down
    // 0.4 per level, thunder all channels down 0.5 per level.
    if f.rain_level > 0.0 {
        let m = 1.0 - f.rain_level * 0.5;
        let mb = 1.0 - f.rain_level * 0.4;
        sky = [sky[0] * m, sky[1] * m, sky[2] * mb];
    }
    if f.thunder_level > 0.0 {
        let m = 1.0 - f.thunder_level * 0.5;
        sky = [sky[0] * m, sky[1] * m, sky[2] * m];
    }
    let sky_fog_end = (dim.sky_fog_end / 16.0).min(view_distance_chunks as f32);
    let mix = clamped_lerp(sky_fog_end / 32.0, 0.25, 1.0);
    let mix = 1.0 - mix.powf(0.25);
    lerp3(fog, sky, mix)
}

/// `BlindnessFogEnvironment.getModifiedDarkness`: full darkness except in the last second of
/// the effect, where it is the remaining fraction.
fn blindness_darkness(duration: i32, darkness: f32) -> f32 {
    if duration != -1 && duration <= 19 {
        (duration as f32 / 20.0).max(darkness)
    } else {
        1.0
    }
}

/// Vanilla `FogRenderer.computeFogColor`: the base colour, the void/effect darkness, the
/// boss-overlay darkening and the water/night-vision brightening.
#[allow(clippy::too_many_arguments)]
fn compute_fog_color(
    f: &FogInputs,
    time_of_day: i64,
    view_distance_chunks: i32,
    partial_tick: f32,
    camera_forward: [f32; 3],
    camera_y: f32,
    world_min_y: f32,
) -> [f32; 3] {
    let base = match color_source(f) {
        FogEnvironment::Lava => LAVA_FOG_COLOR,
        FogEnvironment::PowderSnow => POWDER_SNOW_FOG_COLOR,
        FogEnvironment::Water => f.water_fog_color,
        _ => atmospheric_base(f, time_of_day, view_distance_chunks, camera_forward),
    };

    // Void darkness below the world, then the first applicable darkness-modifying effect.
    let onset = f.void_darkness_onset_range;
    let mut darkness = ((onset + world_min_y - camera_y) / onset).clamp(0.0, 1.0);
    if let Some(d) = f.blindness {
        darkness = blindness_darkness(d, darkness);
    } else if f.darkness.is_some() {
        darkness = darkness.max(f.darkness_blend);
    }

    let mut color = base;
    if darkness > 0.0 && f.fog_type != FogType::Lava && f.fog_type != FogType::PowderSnow {
        let brightness = (1.0 - darkness) * (1.0 - darkness);
        color = [
            color[0] * brightness,
            color[1] * brightness,
            color[2] * brightness,
        ];
    }

    // Boss-overlay world darkening.
    if f.darken_world_amount > 0.0 {
        let a = f.darken_world_amount;
        color = [
            lerp(color[0], color[0] * 0.7, a),
            lerp(color[1], color[1] * 0.6, a),
            lerp(color[2], color[2] * 0.6, a),
        ];
    }

    // Water vision underwater, night vision otherwise.
    let brighten = match f.fog_type {
        FogType::Water => f.water_vision,
        _ => match (f.night_vision, f.darkness) {
            (Some(d), None) => crate::lightmap::night_vision_scale(d, partial_tick),
            _ => 0.0,
        },
    };
    if color[0] != 0.0 && color[1] != 0.0 && color[2] != 0.0 {
        let scale = 1.0 / color[0].max(color[1]).max(color[2]);
        color = [
            lerp(color[0], color[0] * scale, brighten),
            lerp(color[1], color[1] * scale, brighten),
            lerp(color[2], color[2] * scale, brighten),
        ];
    }
    color
}

/// The clear-air fog colour for the default overworld inputs (vanilla
/// `AtmosphericFogEnvironment.getBaseColor` plus the void darkness term of
/// `FogRenderer.computeFogColor`). Kept for the sky dome and the atmospheric tests.
pub fn atmospheric_fog_color(
    time_of_day: i64,
    view_distance_chunks: i32,
    camera_forward: [f32; 3],
    camera_y: f32,
    world_min_y: f32,
) -> [f32; 3] {
    compute_fog_color(
        &FogInputs::default(),
        time_of_day,
        view_distance_chunks,
        0.0,
        camera_forward,
        camera_y,
        world_min_y,
    )
}

/// The fog colour and distances for one frame, from the sky state and camera. Returns the
/// sRGB fog colour (haste's uniforms take linear; convert at the call site) and the fog
/// distances. `partial_tick` feeds the night-vision ramp.
pub fn frame_fog(
    sky: &SkyState,
    partial_tick: f32,
    camera_forward: [f32; 3],
    camera_y: f32,
    world_min_y: f32,
) -> ([f32; 3], FogData) {
    // The client sets `fog_end` to the render distance in blocks (SkyState's fields are the
    // pre-fog-work API; the render distance in chunks is what the fog needs).
    let view_distance_chunks = (sky.fog_end / 16.0).round() as i32;
    let data = setup_fog(view_distance_chunks, &sky.fog);
    let color = compute_fog_color(
        &sky.fog,
        sky.time_of_day,
        view_distance_chunks,
        partial_tick,
        camera_forward,
        camera_y,
        world_min_y,
    );
    (color, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    fn close3(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| close(*x, y))
    }

    fn inputs() -> FogInputs {
        FogInputs::default()
    }

    #[test]
    fn fog_distances_by_render_distance() {
        // rd 32: 512 blocks, span 512/10 = 51.2 -> start 460.8; sky end 512.
        let f = setup_fog(32, &inputs());
        assert_eq!(f.render_distance_end, 512.0);
        assert_eq!(f.render_distance_start, 460.8);
        assert_eq!(f.sky_end, 512.0);
        // rd 12: 192 blocks, span 19.2 -> start 172.8.
        let f = setup_fog(12, &inputs());
        assert_eq!(f.render_distance_end, 192.0);
        assert_eq!(f.render_distance_start, 172.8);
        assert_eq!(f.sky_end, 192.0);
        // rd 2: 32 blocks, span clamped up to 4 -> start 28.
        let f = setup_fog(2, &inputs());
        assert_eq!(f.render_distance_end, 32.0);
        assert_eq!(f.render_distance_start, 28.0);
        assert_eq!(f.sky_end, 32.0);
        // Environmental band is the 0..1024 overworld default everywhere.
        assert_eq!(f.environmental_start, 0.0);
        assert_eq!(f.environmental_end, 1024.0);
    }

    #[test]
    fn fog_value_is_max_of_spherical_and_cylindrical_bands() {
        let f = setup_fog(12, &inputs());
        // On the render distance band: cylindrical distance drives it.
        assert!(close(fog_value(180.0, 185.0, &f), (185.0 - 172.8) / 19.2));
        // Spherical beyond the environmental end saturates.
        assert_eq!(fog_value(1030.0, 0.0, &f), 1.0);
        // The environmental band starts at 0: any spherical distance has slight fog.
        assert!(close(fog_value(10.0, 10.0, &f), 10.0 / 1024.0));
        assert_eq!(fog_value(0.0, 0.0, &f), 0.0);
        // At the render distance end: full fog.
        assert_eq!(fog_value(500.0, 192.0, &f), 1.0);
    }

    #[test]
    fn fog_colour_noon_rd12() {
        let c = atmospheric_fog_color(6000, 12, [0.0, 0.0, -1.0], 64.0, -64.0);
        // Day: no night multiplier, no sunrise tint; mixed toward sky by the rd-12 factor
        // (1 - (0.25 + 0.75*0.375)^0.25).
        let mix = 1.0 - (0.25 + 0.75f32 * 12.0 / 32.0).powf(0.25);
        let expect: Vec<f32> = (0..3)
            .map(|i| {
                OVERWORLD_FOG_COLOR[i] + (OVERWORLD_SKY_COLOR[i] - OVERWORLD_FOG_COLOR[i]) * mix
            })
            .collect();
        assert!(close3(c, [expect[0], expect[1], expect[2]]), "{c:?}");
    }

    #[test]
    fn fog_colour_rd_variants_at_noon() {
        let noon = |rd: i32| atmospheric_fog_color(6000, rd, [0.0, 0.0, -1.0], 64.0, -64.0);
        // rd 32: no sky mix at all (sky fog end caps the mix factor at 1).
        let c = noon(32);
        assert!(close3(c, OVERWORLD_FOG_COLOR), "{c:?}");
        // rd 2: more sky mix than rd 12.
        let c2 = noon(2);
        let c12 = noon(12);
        let dist2 = (c2[0] - OVERWORLD_FOG_COLOR[0]).abs();
        let dist12 = (c12[0] - OVERWORLD_FOG_COLOR[0]).abs();
        assert!(dist2 > dist12, "shorter render distance mixes more sky");
    }

    #[test]
    fn fog_colour_midnight_rd12() {
        let c = atmospheric_fog_color(18000, 12, [0.0, 0.0, -1.0], 64.0, -64.0);
        // Night: fog 0xC0D8FF * (0.06, 0.06, 0.09), sky black.
        let night_fog = [
            OVERWORLD_FOG_COLOR[0] * 0.06,
            OVERWORLD_FOG_COLOR[1] * 0.06,
            OVERWORLD_FOG_COLOR[2] * 0.09,
        ];
        let mix = 1.0 - (0.25 + 0.75f32 * 12.0 / 32.0).powf(0.25);
        let expect: Vec<f32> = (0..3)
            .map(|i| night_fog[i] + (0.0 - night_fog[i]) * mix)
            .collect();
        assert!(close3(c, [expect[0], expect[1], expect[2]]), "{c:?}");
    }

    #[test]
    fn fog_colour_sunset_look_direction() {
        // t=12300: sunset, glow in the west (-x). Looking west tints; looking east does not.
        let west = atmospheric_fog_color(12300, 12, [-1.0, 0.0, 0.0], 64.0, -64.0);
        let east = atmospheric_fog_color(12300, 12, [1.0, 0.0, 0.0], 64.0, -64.0);
        assert!(
            west[0] > east[0],
            "looking at the sunset is redder: {west:?} {east:?}"
        );
        assert_eq!(sun_glow_side(12300), -1.0);
        assert_eq!(sun_glow_side(300), 1.0);
        // rd < 4: no tint even looking straight at the sun.
        let low = atmospheric_fog_color(12300, 2, [-1.0, 0.0, 0.0], 64.0, -64.0);
        let low_east = atmospheric_fog_color(12300, 2, [1.0, 0.0, 0.0], 64.0, -64.0);
        assert!(close3(low, low_east));
    }

    #[test]
    fn fog_colour_takes_the_weather_layers() {
        // The FOG_COLOR weather layers (WeatherAttributes RAIN/THUNDER, MULTIPLY_RGB):
        // rain (0.5, 0.5, 0.6) by rain minus thunder, thunder (0.25, 0.25, 0.3) by thunder —
        // applied to the base fog colour before the sky mix, exactly where the attribute
        // system delivers them. Full rain, no thunder: fog * (0.5, 0.5, 0.6).
        let rain_fog = |inputs: &FogInputs| {
            compute_fog_color(inputs, 6000, 32, 0.0, [0.0, 0.0, -1.0], 64.0, -64.0)
        };
        let mut inputs = FogInputs::default();
        let clear = rain_fog(&inputs);
        inputs.rain_level = 1.0;
        let rain = rain_fog(&inputs);
        for i in 0..3 {
            let m = [0.5, 0.5, 0.6][i];
            assert!(
                close(rain[i], OVERWORLD_FOG_COLOR[i] * m),
                "channel {i}: {}",
                rain[i]
            );
        }
        // rd 32: no sky mix, so the whole colour is the layered fog. Clear is the base.
        assert!(close3(clear, OVERWORLD_FOG_COLOR));
        // Full thunder (rain 1, thunder 1): the rain layer's weight is 0, the thunder layer
        // multiplies (0.25, 0.25, 0.3) — darker than rain.
        inputs.thunder_level = 1.0;
        let thunder = rain_fog(&inputs);
        for i in 0..3 {
            let m = [0.25, 0.25, 0.3][i];
            assert!(
                close(thunder[i], OVERWORLD_FOG_COLOR[i] * m),
                "channel {i}: {}",
                thunder[i]
            );
            assert!(thunder[i] < rain[i], "thunder darkens channel {i}");
        }
    }

    #[test]
    fn sunrise_colour_timeline_extremes() {
        // Night: alpha 0. Plain day: the 730..11270 plateau.
        let night = sunrise_sunset_color(18000);
        assert_eq!(night[3], 0.0);
        let day = sunrise_sunset_color(6000);
        let plateau = argb(16770355);
        assert!(close3(
            [day[0], day[1], day[2]],
            [plateau[0], plateau[1], plateau[2]]
        ));
        // Peak alpha somewhere in the 12300-13300 evening range.
        let peak = sunrise_sunset_color(12732);
        assert!(peak[3] > 0.9, "{}", peak[3]);
        // Ramps are linear: halfway along the 12243..12358 segment is the tick average.
        let a = sunrise_sunset_color(12243 + 57);
        let b0 = argb((-1310226637i32) as u32);
        let b1 = argb((-857440717i32) as u32);
        for i in 0..4 {
            assert!(close(a[i], b0[i] + (b1[i] - b0[i]) * 57.0 / 115.0), "{a:?}");
        }
    }

    #[test]
    fn timeline_wraps_the_day() {
        // At t=0 (before the first keyframe tick 71): the wrap segment from the last
        // keyframe (23757) to the first (71) one period later.
        let v = sunrise_sunset_color(0);
        let first = argb(1609540403);
        let last = argb((-1310226637i32) as u32);
        let span = 24000 + 71 - 23757;
        for i in 0..4 {
            let expect = last[i] + (first[i] - last[i]) * ((24000 - 23757) as f32 / span as f32);
            assert!(close(v[i], expect), "{v:?}");
        }
        // At the first keyframe's tick: its value.
        let v = sunrise_sunset_color(71);
        assert!(close3([v[0], v[1], v[2]], [first[0], first[1], first[2]]));
        assert!(close(v[3], first[3]));
    }

    #[test]
    fn void_darkness_below_the_world() {
        let above = atmospheric_fog_color(6000, 12, [0.0, 0.0, -1.0], 64.0, -64.0);
        let below = atmospheric_fog_color(6000, 12, [0.0, 0.0, -1.0], -96.0, -64.0);
        // 32 below min build height: full darkness squared.
        assert!(below.iter().all(|&c| close(c, 0.0)), "{below:?}");
        assert!(!above.iter().all(|&c| close(c, 0.0)));
    }

    // ---- the remaining environments (vanilla `environment/*FogEnvironment`) ----

    #[test]
    fn water_fog_distances_and_colour() {
        let mut f = inputs();
        f.fog_type = FogType::Water;
        f.water_fog_start = -8.0;
        f.water_fog_end = 96.0;
        f.water_vision = 0.5;
        let d = setup_fog(12, &f);
        assert_eq!(d.environment, FogEnvironment::Water);
        // `WaterFogEnvironment.setupFog`: end = 96 * max(0.25, 0.5) = 48; sky/cloud = end.
        assert_eq!(d.environmental_start, -8.0);
        assert_eq!(d.environmental_end, 48.0);
        assert_eq!(d.sky_end, 48.0);
        assert_eq!(d.cloud_end, 48.0);
        // Water vision below 0.25 clamps the ramp to a quarter.
        f.water_vision = 0.1;
        assert_eq!(setup_fog(12, &f).environmental_end, 24.0);
        // Colour is the biome water fog colour, brightened by the water vision.
        f.water_vision = 0.0;
        let c = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], 64.0, -64.0);
        assert!(close3(c, WATER_FOG_COLOR), "{c:?}");
        // Brightening lerps toward the normalised colour (max channel -> 1).
        f.water_vision = 1.0;
        let bright = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], 64.0, -64.0);
        let scale = 1.0
            / WATER_FOG_COLOR[0]
                .max(WATER_FOG_COLOR[1])
                .max(WATER_FOG_COLOR[2]);
        assert!(close3(
            bright,
            [
                WATER_FOG_COLOR[0] * scale,
                WATER_FOG_COLOR[1] * scale,
                WATER_FOG_COLOR[2] * scale
            ]
        ));
    }

    #[test]
    fn lava_fog_distances_and_colour() {
        let mut f = inputs();
        f.fog_type = FogType::Lava;
        // `LavaFogEnvironment.setupFog` default: 0.25..1.
        let d = setup_fog(12, &f);
        assert_eq!(d.environment, FogEnvironment::Lava);
        assert_eq!(d.environmental_start, 0.25);
        assert_eq!(d.environmental_end, 1.0);
        // Fire resistance: 0..5.
        f.fire_resistance = true;
        let d = setup_fog(12, &f);
        assert_eq!(d.environmental_start, 0.0);
        assert_eq!(d.environmental_end, 5.0);
        // Spectator: -8..render_distance/2.
        f.spectator = true;
        let d = setup_fog(12, &f);
        assert_eq!(d.environmental_start, -8.0);
        assert_eq!(d.environmental_end, 96.0);
        // Colour is the fixed lava colour; the void darkness term is skipped in lava.
        let c = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], -96.0, -64.0);
        assert!(close3(c, LAVA_FOG_COLOR), "{c:?}");
    }

    #[test]
    fn powder_snow_fog_distances_and_colour() {
        let mut f = inputs();
        f.fog_type = FogType::PowderSnow;
        // `PowderedSnowFogEnvironment.setupFog`: 0..2.
        let d = setup_fog(12, &f);
        assert_eq!(d.environment, FogEnvironment::PowderSnow);
        assert_eq!(d.environmental_start, 0.0);
        assert_eq!(d.environmental_end, 2.0);
        assert_eq!(d.sky_end, 2.0);
        // Spectator: -8..render_distance/2.
        f.spectator = true;
        let d = setup_fog(12, &f);
        assert_eq!(d.environmental_start, -8.0);
        assert_eq!(d.environmental_end, 96.0);
        // Colour is the fixed powder-snow colour; the void darkness term is skipped.
        let c = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], -96.0, -64.0);
        assert!(close3(c, POWDER_SNOW_FOG_COLOR), "{c:?}");
    }

    #[test]
    fn blindness_fog_distances_and_darkness() {
        let mut f = inputs();
        // `BlindnessFogEnvironment.setupFog`: an infinite effect is a flat 5 blocks.
        f.blindness = Some(-1);
        let d = setup_fog(12, &f);
        assert_eq!(d.environment, FogEnvironment::Blindness);
        assert_eq!(d.environmental_start, 1.25);
        assert_eq!(d.environmental_end, 5.0);
        assert_eq!(d.sky_end, 4.0);
        // A 10-tick effect: lerp(min(1, 10/20), render_distance, 5) = lerp(0.5, 192, 5) = 98.5.
        f.blindness = Some(10);
        let d = setup_fog(12, &f);
        assert!(close(d.environmental_end, 98.5), "{}", d.environmental_end);
        // Darkness: full while the effect runs, the remaining fraction in the last second.
        assert_eq!(blindness_darkness(-1, 0.0), 1.0);
        assert_eq!(blindness_darkness(100, 0.0), 1.0);
        assert!(close(blindness_darkness(10, 0.0), 0.5));
        // The colour stays the atmosphere's (blindness does not provide a colour) but is
        // darkened to black by the full effect.
        f.blindness = Some(-1);
        let c = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], 64.0, -64.0);
        assert!(c.iter().all(|&v| close(v, 0.0)), "{c:?}");
    }

    #[test]
    fn darkness_fog_distances_and_darkness() {
        let mut f = inputs();
        f.darkness = Some(400);
        f.darkness_blend = 1.0;
        // `DarknessFogEnvironment.setupFog`: lerp(blend, render_distance, 15); start 0.75d.
        let d = setup_fog(12, &f);
        assert_eq!(d.environment, FogEnvironment::Darkness);
        assert_eq!(d.environmental_start, 11.25);
        assert_eq!(d.environmental_end, 15.0);
        assert_eq!(d.sky_end, 15.0);
        // Halfway blend: lerp(0.5, 192, 15) = 103.5.
        f.darkness_blend = 0.5;
        let d = setup_fog(12, &f);
        assert!(close(d.environmental_end, 103.5), "{}", d.environmental_end);
        // The colour is the atmosphere's darkened by the blend factor.
        f.darkness_blend = 0.5;
        let base = atmospheric_fog_color(6000, 12, [0.0, 0.0, -1.0], 64.0, -64.0);
        let c = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], 64.0, -64.0);
        let brightness = (1.0 - 0.5f32) * (1.0 - 0.5);
        assert!(close3(
            c,
            [
                base[0] * brightness,
                base[1] * brightness,
                base[2] * brightness
            ]
        ));
    }

    #[test]
    fn blindness_wins_over_darkness() {
        // Vanilla's list order puts blindness before darkness: with both, blindness sets the
        // distances and its full darkness.
        let mut f = inputs();
        f.blindness = Some(-1);
        f.darkness = Some(400);
        f.darkness_blend = 0.0;
        assert_eq!(first_applicable(&f), FogEnvironment::Blindness);
        assert_eq!(setup_fog(12, &f).environmental_end, 5.0);
    }

    #[test]
    fn effect_environment_overrides_water_distances_but_not_colour() {
        // Underwater with blindness: blindness sets the distances, water provides the colour.
        let mut f = inputs();
        f.fog_type = FogType::Water;
        f.water_fog_color = [0.1, 0.2, 0.3];
        f.blindness = Some(-1);
        let d = setup_fog(12, &f);
        assert_eq!(d.environment, FogEnvironment::Blindness);
        assert_eq!(d.environmental_end, 5.0);
        // Full blindness darkens the water colour to black.
        let c = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], 64.0, -64.0);
        assert!(c.iter().all(|&v| close(v, 0.0)), "{c:?}");
    }

    #[test]
    fn nether_dimension_fog() {
        let mut f = inputs();
        f.dimension = DimensionFog::nether();
        // `DimensionTypes` nether: `FOG_START_DISTANCE` 10, `FOG_END_DISTANCE` 96.
        let d = setup_fog(12, &f);
        assert_eq!(d.environmental_start, 10.0);
        assert_eq!(d.environmental_end, 96.0);
        // No day timeline: the fog colour is the biome colour mixed toward the (black) sky.
        f.dimension = f.dimension.with_biome(&BiomeFog {
            fog_color: Some(0x330808),
            ..Default::default()
        });
        let c = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], 70.0, 0.0);
        let sky_fog_end = (512.0 / 16.0f32).min(12.0);
        let mix = 1.0 - clamped_lerp(sky_fog_end / 32.0, 0.25, 1.0).powf(0.25);
        let base = rgb24(0x330808);
        let expect = [
            base[0] + (0.0 - base[0]) * mix,
            base[1] + (0.0 - base[1]) * mix,
            base[2] + (0.0 - base[2]) * mix,
        ];
        assert!(close3(c, expect), "{c:?} {expect:?}");
    }

    #[test]
    fn end_dimension_fog_colour() {
        let mut f = inputs();
        f.dimension = DimensionFog::end();
        let c = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], 64.0, 0.0);
        // End fog colour mixed toward the black End sky.
        let base = rgb24(-15199464);
        let sky_fog_end = (512.0 / 16.0f32).min(12.0);
        let mix = 1.0 - clamped_lerp(sky_fog_end / 32.0, 0.25, 1.0).powf(0.25);
        let expect = [
            base[0] * (1.0 - mix),
            base[1] * (1.0 - mix),
            base[2] * (1.0 - mix),
        ];
        assert!(close3(c, expect), "{c:?} {expect:?}");
    }

    #[test]
    fn boss_world_fog_clamps_the_atmosphere() {
        let mut f = inputs();
        f.boss_world_fog = true;
        let d = setup_fog(12, &f);
        // `AtmosphericFogEnvironment.setupFog` with `shouldCreateWorldFog`: start <= 10,
        // end <= 96, sky/cloud = end.
        assert_eq!(d.environmental_start, 0.0);
        assert_eq!(d.environmental_end, 96.0);
        assert_eq!(d.sky_end, 96.0);
        assert_eq!(d.cloud_end, 96.0);
    }

    #[test]
    fn boss_darken_world_amount() {
        let mut f = inputs();
        f.darken_world_amount = 1.0;
        let base = atmospheric_fog_color(6000, 12, [0.0, 0.0, -1.0], 64.0, -64.0);
        let c = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], 64.0, -64.0);
        assert!(close3(c, [base[0] * 0.7, base[1] * 0.6, base[2] * 0.6]));
    }

    #[test]
    fn night_vision_brightens_but_not_with_darkness() {
        let mut f = inputs();
        f.night_vision = Some(-1);
        let base = atmospheric_fog_color(6000, 12, [0.0, 0.0, -1.0], 64.0, -64.0);
        let c = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], 64.0, -64.0);
        let scale = 1.0 / base[0].max(base[1]).max(base[2]);
        assert!(close3(
            c,
            [base[0] * scale, base[1] * scale, base[2] * scale]
        ));
        // With darkness too, the night-vision branch is skipped.
        f.darkness = Some(400);
        f.darkness_blend = 0.0;
        let c = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], 64.0, -64.0);
        assert!(close3(c, base));
    }
}
