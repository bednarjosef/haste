//! `TotemParticle` (the `totem_of_undying` type): a `SimpleAnimatedParticle` that the
//! totem's pop spawns from the tracking emitter. Its colour is rolled in its constructor
//! (a quarter gold, the rest green), and it fades out over the second half of its life.

use super::rng::Rng;

/// `TotemParticle`'s colour roll (its constructor): one in four is the gold branch
/// `(0.6 + 0.2r, 0.6 + 0.3r, 0.2r)`, the rest the green `(0.1 + 0.2r, 0.4 + 0.3r, 0.2r)`.
pub(super) fn totem_color(rng: &mut Rng) -> [f32; 3] {
    if rng.next_f32() < 0.25 {
        let r = rng.next_f32();
        [
            0.6 + r * 0.2,
            0.6 + rng.next_f32() * 0.3,
            rng.next_f32() * 0.2,
        ]
    } else {
        let r = rng.next_f32();
        [
            0.1 + r * 0.2,
            0.4 + rng.next_f32() * 0.3,
            rng.next_f32() * 0.2,
        ]
    }
}

/// `SimpleAnimatedParticle.tick`'s alpha: untouched while `age <= lifetime / 2`, then
/// `1 - (age - lifetime / 2) / lifetime`.
pub(super) fn fade_alpha(age: f32, lifetime: f32) -> Option<f32> {
    let half = lifetime / 2.0;
    (age > half).then(|| 1.0 - (age - half) / lifetime)
}

#[cfg(test)]
#[path = "totem_tests.rs"]
mod tests;
