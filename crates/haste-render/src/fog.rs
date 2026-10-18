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

mod atmospheric;
mod blindness;
mod lava;
mod powder_snow;
mod sky_color;
mod timeline;
mod water;

pub use atmospheric::{atmospheric_fog_color, OVERWORLD_FOG_COLOR, OVERWORLD_SKY_COLOR};
pub use lava::LAVA_FOG_COLOR;
pub use powder_snow::POWDER_SNOW_FOG_COLOR;
pub use sky_color::{overworld_sky_color, sun_glow_side};
pub use timeline::sunrise_sunset_color;
pub use water::{WATER_FOG_COLOR, WATER_FOG_END, WATER_FOG_START};

pub(crate) use timeline::timeline_lerp;

use atmospheric::atmospheric_base;
use blindness::blindness_darkness;
#[allow(unused_imports)]
use timeline::argb;
use timeline::lerp3;
use timeline::timeline_sample3;
use timeline::{FOG_COLOR_NIGHT, SKY_COLOR_NIGHT};

/// `ARGB.vector3fFromRGB24` of an int colour (0xRRGGBB, alpha ignored) as sRGB floats.
pub const fn rgb24(c: i32) -> [f32; 3] {
    let c = c as u32;
    [
        ((c >> 16) & 0xFF) as f32 / 255.0,
        ((c >> 8) & 0xFF) as f32 / 255.0,
        (c & 0xFF) as f32 / 255.0,
    ]
}

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
mod tests;
