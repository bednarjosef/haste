// The night sky's stars (vanilla `SkyRenderer.buildStars` / `renderStars`): the brightness
// track of the overworld day timeline (`visual/star_brightness`).

/// `visual/star_brightness` keyframes of the overworld day timeline (`Timelines`, a MAXIMUM
/// modifier over the default 0), piecewise linear and wrapping over the 24000-tick day.
const STAR_BRIGHTNESS: [(i32, f32); 12] = [
    (92, 0.037),
    (627, 0.0),
    (11373, 0.0),
    (11732, 0.016),
    (11959, 0.044),
    (12399, 0.143),
    (12729, 0.258),
    (13228, 0.5),
    (22772, 0.5),
    (23032, 0.364),
    (23356, 0.225),
    (23758, 0.101),
];

/// The overworld day timeline's star brightness at a time of day (before the weather layers,
/// which set it to 0 by the rain and thunder levels).
pub fn star_brightness(time_of_day: i64) -> f32 {
    crate::fog::timeline_lerp(&STAR_BRIGHTNESS, time_of_day, |a, b, t| a + (b - a) * t)
}

#[cfg(test)]
mod tests;
