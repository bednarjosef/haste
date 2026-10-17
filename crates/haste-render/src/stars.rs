// The night sky's stars (vanilla `SkyRenderer.buildStars` / `renderStars`): the star field
// from vanilla's seeded random and the brightness track of the overworld day timeline
// (`visual/star_brightness`).

use glam::{Mat3, Vec3};

use crate::java_random::JavaRandom;

/// Vanilla's star seed and attempt count (`SkyRenderer.buildStars`).
const STAR_SEED: i64 = 10842;
const STAR_ATTEMPTS: usize = 1500;

/// The star quads in the celestial frame (sun-angle rotation applied by the shader), four
/// corners each in vanilla's vertex order. Follows `SkyRenderer.buildStars`: a direction and
/// a size from `nextFloat`, rejected outside the shell 0.01 < |d|^2 < 1, pushed out to radius
/// 100, then a spin from `nextDouble`; the quad faces the origin (`rotateTowards(-centre,
/// +Y)`) and turns by `-spin` about its own Z.
pub fn star_quads() -> Vec<[[f32; 3]; 4]> {
    let mut random = JavaRandom::new(STAR_SEED);
    let mut out = Vec::with_capacity(STAR_ATTEMPTS);
    for _ in 0..STAR_ATTEMPTS {
        let x = random.next_float() * 2.0 - 1.0;
        let y = random.next_float() * 2.0 - 1.0;
        let z = random.next_float() * 2.0 - 1.0;
        let size = 0.15 + random.next_float() * 0.1;
        let len2 = x * x + y * y + z * z;
        if len2 <= 0.010_000_001 || len2 >= 1.0 {
            continue;
        }
        let centre = Vec3::new(x, y, z) * ((1.0 / len2.sqrt()) * 100.0);
        let spin = (random.next_double() * std::f32::consts::PI as f64 * 2.0) as f32;
        let basis = towards(-centre, Vec3::Y) * Mat3::from_rotation_z(-spin);
        let corner = |sx: f32, sy: f32| (basis * Vec3::new(sx, sy, 0.0) + centre).to_array();
        out.push([
            corner(size, -size),
            corner(size, size),
            corner(-size, size),
            corner(-size, -size),
        ]);
    }
    out
}

/// JOML's `Matrix3f.rotateTowards(dir, up)` on the identity: the basis whose columns are
/// left = normalize(up x dir), up' = dir x left and dir (normalized), mapping +Z onto `dir`.
fn towards(dir: Vec3, up: Vec3) -> Mat3 {
    let dir = dir * (1.0 / dir.length());
    let left = up.cross(dir);
    let left = left * (1.0 / left.length());
    let up = dir.cross(left);
    Mat3::from_cols(left, up, dir)
}

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
