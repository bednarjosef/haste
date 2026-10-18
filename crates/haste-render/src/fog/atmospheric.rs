// Atmospheric / biome fog (vanilla `AtmosphericFogEnvironment`) and the overworld
// dimension constants it reads.

// Overworld dimension fog colour (`DimensionTypes` overworld `FOG_COLOR` 0xC0D8FF).
pub const OVERWORLD_FOG_COLOR: [f32; 3] = super::rgb24(-4138753);
/// Overworld sky colour (`OverworldBiomes.calculateSkyColor(0.8)` = 0x78A7FF).
pub const OVERWORLD_SKY_COLOR: [f32; 3] = [
    0x78 as f32 / 255.0,
    0xA7 as f32 / 255.0,
    0xFF as f32 / 255.0,
];

/// `AtmosphericFogEnvironment.getBaseColor`: the dimension/biome fog colour with the day
/// timeline's night multiplier and sunrise/sunset tint, mixed toward the sky colour by a
/// factor that widens as the render distance shrinks.
pub(super) fn atmospheric_base(
    f: &super::FogInputs,
    time_of_day: i64,
    view_distance_chunks: i32,
    camera_forward: [f32; 3],
) -> [f32; 3] {
    let dim = &f.dimension;
    let mut fog = dim.fog_color;
    if dim.day_timeline {
        let night = super::timeline_sample3(&super::FOG_COLOR_NIGHT, time_of_day);
        fog = [fog[0] * night[0], fog[1] * night[1], fog[2] * night[2]];
        // Sunrise/sunset tint when looking at the sun (render distance >= 4 chunks).
        if view_distance_chunks >= 4 {
            let side = super::sun_glow_side(time_of_day);
            let looking = camera_forward[0] * side;
            if looking > 0.0 {
                let glow = super::sunrise_sunset_color(time_of_day);
                if glow[3] > 0.0 {
                    fog = super::lerp3(fog, [glow[0], glow[1], glow[2]], looking * glow[3]);
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
        let night = super::timeline_sample3(&super::SKY_COLOR_NIGHT, time_of_day);
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
    let mix = super::clamped_lerp(sky_fog_end / 32.0, 0.25, 1.0);
    let mix = 1.0 - mix.powf(0.25);
    super::lerp3(fog, sky, mix)
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
    super::compute_fog_color(
        &super::FogInputs::default(),
        time_of_day,
        view_distance_chunks,
        0.0,
        camera_forward,
        camera_y,
        world_min_y,
    )
}
