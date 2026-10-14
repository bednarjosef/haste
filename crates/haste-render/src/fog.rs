// Distance fog and the fog colour, matching vanilla's clear-air overworld exactly
// (vanilla: FogRenderer/AtmosphericFogEnvironment and the sky.fsh/block.fsh fog include).
//
// Vanilla splits fog distances into two bands, both linear:
// - the environmental band (biome dimension attributes `fog_start_distance` 0 and
//   `fog_end_distance` 1024 in the overworld) measured on the spherical distance, and
// - the render-distance band, measured on the cylindrical distance (max of horizontal
//   distance and |y|): start = render_distance_blocks - clamp(render_distance_blocks/10,
//   4, 64), end = render_distance_blocks.
// The fragment fog value is the max of both bands (fog.glsl `apply_fog`).
//
// The fog colour is the dimension/biome fog colour (overworld 0xC0D8FF) with the day
// timeline's night multiplier, tinted toward the sunrise/sunset colour when looking at the
// sun, then mixed toward the sky colour (overworld 0x78A7FF with its own night multiplier)
// by a factor that widens as the render distance shrinks. Water/lava/powder snow/blindness/
// darkness fogs are later tasks: they are further `FogEnvironment`s here, chosen by one
// function like vanilla's environment list; only the atmospheric case is implemented.
//
// The timeline tables (night fog/sky multipliers, sunrise/sunset colour keyframes) are
// re-typed from vanilla's overworld day timeline: piecewise linear between integer-tick
// keyframes over the 24000-tick day, values as plain floats (vanilla quantizes colours to
// 8 bit per channel only when packing into int, and lerps in float; the mix chain here
// stays float like vanilla's).

use crate::SkyState;

/// Overworld dimension fog colour (`DimensionTypes` overworld attributes).
pub const OVERWORLD_FOG_COLOR: [f32; 3] = [
    0xC0 as f32 / 255.0,
    0xD8 as f32 / 255.0,
    0xFF as f32 / 255.0,
];
/// Overworld sky colour (`OverworldBiomes.calculateSkyColor(0.8)` = 0x78A7FF).
pub const OVERWORLD_SKY_COLOR: [f32; 3] = [
    0x78 as f32 / 255.0,
    0xA7 as f32 / 255.0,
    0xFF as f32 / 255.0,
];

/// Which fog environment applies this frame (vanilla `FogEnvironment` list; clear air now).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FogEnvironment {
    /// Clear air: the atmospheric environment (everything below is a later task).
    Atmospheric,
    // Water, Lava, PowderSnow, Blindness, Darkness: later tasks (vanilla picks them by
    // fluid in camera / mob effects, in this priority order).
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

/// Vanilla `FogRenderer.setupFog` for the atmospheric environment (clear overworld air).
/// `view_distance_chunks` is the effective render distance (server-capped).
pub fn setup_fog(view_distance_chunks: i32) -> FogData {
    let render_distance = (view_distance_chunks.max(2) * 16) as f32;
    let span = (render_distance / 10.0).clamp(4.0, 64.0);
    FogData {
        // Overworld dimension sets neither fog_start_distance (default 0) nor
        // fog_end_distance (default 1024).
        environmental_start: 0.0,
        environmental_end: 1024.0,
        render_distance_start: render_distance - span,
        render_distance_end: render_distance,
        // sky_fog_end_distance default 512; clouds: min(cloud range option (128 chunks) *
        // 16, cloud_fog_end_distance default 2048) = 2048.
        sky_end: render_distance.min(512.0),
        cloud_end: 2048.0,
        environment: FogEnvironment::Atmospheric,
    }
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
/// multiplier (clear weather; rain/thunder darkening is a later task with weather itself).
pub fn overworld_sky_color(time_of_day: i64) -> [f32; 3] {
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

/// The clear-air fog colour (sRGB floats; vanilla `AtmosphericFogEnvironment.getBaseColor`
/// plus the void darkness term of `FogRenderer.computeFogColor`). `camera_forward` is the
/// unit look direction, `view_distance_chunks` the effective render distance, `camera_y` the
/// eye height and `world_min_y` the dimension's minimum build height.
pub fn atmospheric_fog_color(
    time_of_day: i64,
    view_distance_chunks: i32,
    camera_forward: [f32; 3],
    camera_y: f32,
    world_min_y: f32,
) -> [f32; 3] {
    let mut fog = [
        OVERWORLD_FOG_COLOR[0],
        OVERWORLD_FOG_COLOR[1],
        OVERWORLD_FOG_COLOR[2],
    ];
    let night = timeline_sample3(&FOG_COLOR_NIGHT, time_of_day);
    fog = [fog[0] * night[0], fog[1] * night[1], fog[2] * night[2]];

    // Sunrise/sunset tint when looking at the sun (render distance >= 4 chunks).
    if view_distance_chunks >= 4 {
        let side = sun_glow_side(time_of_day);
        let looking = camera_forward[0] * side;
        if looking > 0.0 {
            let glow = sunrise_sunset_color(time_of_day);
            if glow[3] > 0.0 {
                let f = looking * glow[3];
                fog = [
                    fog[0] + (glow[0] - fog[0]) * f,
                    fog[1] + (glow[1] - fog[1]) * f,
                    fog[2] + (glow[2] - fog[2]) * f,
                ];
            }
        }
    }

    // Mix toward the sky colour, more the shorter the render distance.
    let mut sky = OVERWORLD_SKY_COLOR;
    let night = timeline_sample3(&SKY_COLOR_NIGHT, time_of_day);
    sky = [sky[0] * night[0], sky[1] * night[1], sky[2] * night[2]];
    let sky_fog_end = 32f32.min(view_distance_chunks as f32);
    let mix = clamped_lerp(sky_fog_end / 32.0, 0.25, 1.0);
    let mix = 1.0 - mix.powf(0.25);
    let fog = [
        fog[0] + (sky[0] - fog[0]) * mix,
        fog[1] + (sky[1] - fog[1]) * mix,
        fog[2] + (sky[2] - fog[2]) * mix,
    ];

    // Void darkness below the world (vanilla: square of 1 - darkness multiplier).
    let onset = 32.0;
    let darkness = ((onset + world_min_y - camera_y) / onset).clamp(0.0, 1.0);
    if darkness > 0.0 {
        let brightness = (1.0 - darkness) * (1.0 - darkness);
        [
            fog[0] * brightness,
            fog[1] * brightness,
            fog[2] * brightness,
        ]
    } else {
        fog
    }
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

/// The fog colour and distances for one frame, from the sky state and camera. Returns the
/// sRGB fog colour (haste's uniforms take linear; convert at the call site) and the fog
/// distances.
pub fn frame_fog(
    sky: &SkyState,
    camera_forward: [f32; 3],
    camera_y: f32,
    world_min_y: f32,
) -> ([f32; 3], FogData) {
    // The client sets `fog_end` to the render distance in blocks (SkyState's fields are the
    // pre-fog-work API; the render distance in chunks is what the fog needs).
    let view_distance_chunks = (sky.fog_end / 16.0).round() as i32;
    let data = setup_fog(view_distance_chunks);
    let color = match data.environment {
        FogEnvironment::Atmospheric => atmospheric_fog_color(
            sky.time_of_day,
            view_distance_chunks,
            camera_forward,
            camera_y,
            world_min_y,
        ),
    };
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

    #[test]
    fn fog_distances_by_render_distance() {
        // rd 32: 512 blocks, span 512/10 = 51.2 -> start 460.8; sky end 512.
        let f = setup_fog(32);
        assert_eq!(f.render_distance_end, 512.0);
        assert_eq!(f.render_distance_start, 460.8);
        assert_eq!(f.sky_end, 512.0);
        // rd 12: 192 blocks, span 19.2 -> start 172.8.
        let f = setup_fog(12);
        assert_eq!(f.render_distance_end, 192.0);
        assert_eq!(f.render_distance_start, 172.8);
        assert_eq!(f.sky_end, 192.0);
        // rd 2: 32 blocks, span clamped up to 4 -> start 28.
        let f = setup_fog(2);
        assert_eq!(f.render_distance_end, 32.0);
        assert_eq!(f.render_distance_start, 28.0);
        assert_eq!(f.sky_end, 32.0);
        // Environmental band is the 0..1024 overworld default everywhere.
        assert_eq!(f.environmental_start, 0.0);
        assert_eq!(f.environmental_end, 1024.0);
    }

    #[test]
    fn fog_value_is_max_of_spherical_and_cylindrical_bands() {
        let f = setup_fog(12);
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
        let _ = mix;
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
}
