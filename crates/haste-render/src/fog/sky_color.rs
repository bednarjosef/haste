// Overworld sky-dome colour and the sun's glow side.

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
    let night = super::timeline_sample3(&super::SKY_COLOR_NIGHT, time_of_day);
    [
        super::OVERWORLD_SKY_COLOR[0] * night[0],
        super::OVERWORLD_SKY_COLOR[1] * night[1],
        super::OVERWORLD_SKY_COLOR[2] * night[2],
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
