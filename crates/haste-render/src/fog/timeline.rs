// Piecewise-linear overworld day timeline sampling (`KeyframeTrackSampler`) and the
// sunrise/sunset colour track. Used by the atmospheric and sky-colour code.

/// Piecewise-linear timeline sampling over the 24000-tick day: segments between consecutive
/// keyframes plus the wrap segment from the last keyframe back to the first (vanilla
/// `KeyframeTrackSampler`: at a keyframe tick the keyframe's value, between them linear by
/// tick, `ticks` taken mod the period).
pub(crate) fn timeline_lerp<T, L>(keyframes: &[(i32, T)], ticks: i64, lerp: L) -> T
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

pub(super) fn lerp3(a: [f32; 3], b: [f32; 3], alpha: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * alpha,
        a[1] + (b[1] - a[1]) * alpha,
        a[2] + (b[2] - a[2]) * alpha,
    ]
}

pub(super) fn lerp4(a: [f32; 4], b: [f32; 4], alpha: f32) -> [f32; 4] {
    [
        a[0] + (b[0] - a[0]) * alpha,
        a[1] + (b[1] - a[1]) * alpha,
        a[2] + (b[2] - a[2]) * alpha,
        a[3] + (b[3] - a[3]) * alpha,
    ]
}

pub(super) fn timeline_sample3(keyframes: &[(i32, [f32; 3])], ticks: i64) -> [f32; 3] {
    timeline_lerp(keyframes, ticks, lerp3)
}

pub(super) fn timeline_sample4(keyframes: &[(i32, [f32; 4])], ticks: i64) -> [f32; 4] {
    timeline_lerp(keyframes, ticks, lerp4)
}

/// Fog colour night multiplier (timeline `fog_color` MULTIPLY_RGB track: white by day,
/// (0.06, 0.06, 0.09) at night, ramps 11867-13670 and 22330-24000+133).
pub(super) const FOG_COLOR_NIGHT: [(i32, [f32; 3]); 4] = [
    (133, [1.0, 1.0, 1.0]),
    (11867, [1.0, 1.0, 1.0]),
    (13670, [0.06, 0.06, 0.09]),
    (22330, [0.06, 0.06, 0.09]),
];

/// Sky colour night multiplier (same shape, black at night).
pub(super) const SKY_COLOR_NIGHT: [(i32, [f32; 3]); 4] = [
    (133, [1.0, 1.0, 1.0]),
    (11867, [1.0, 1.0, 1.0]),
    (13670, [0.0, 0.0, 0.0]),
    (22330, [0.0, 0.0, 0.0]),
];

/// Cloud colour night multiplier (timeline `cloud_color` MULTIPLY_ARGB track: white by
/// day, the bluish (0.1, 0.1, 0.15) at night, same tick shape as the fog multiplier).
pub(super) const CLOUD_COLOR_NIGHT: [(i32, [f32; 3]); 4] = [
    (133, [1.0, 1.0, 1.0]),
    (11867, [1.0, 1.0, 1.0]),
    (13670, [0.1, 0.1, 0.15]),
    (22330, [0.1, 0.1, 0.15]),
];

/// Sunrise/sunset colour (timeline `sunrise_sunset_color` track, ARGB as floats). The
/// int constants are vanilla's packed ARGB keyframe values.
pub(super) const SUNRISE_SUNSET_COLOR: [(i32, [f32; 4]); 32] = [
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
pub(super) const fn argb(c: u32) -> [f32; 4] {
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
