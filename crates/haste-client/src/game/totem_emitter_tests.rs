use super::*;

use glam::DVec3;
use haste_audio::random::LegacyRandom;

fn rng() -> LegacyRandom {
    LegacyRandom::new(0x5EED_1234)
}

#[test]
fn lives_thirty_ticks_then_stops() {
    let mut emitter = TotemEmitter::new(haste_core::EntityId(7));
    let mut rng = rng();
    let mut out = Vec::new();
    // `TrackingEmitter.tick` removes it once `life >= lifeTime`: alive for 29 ticks, gone on the 30th.
    for _ in 0..LIFE_TIME - 1 {
        assert!(emitter.tick(DVec3::ZERO, (0.6, 1.8), &mut rng, &mut out));
    }
    assert!(!emitter.tick(DVec3::ZERO, (0.6, 1.8), &mut rng, &mut out));
    assert_eq!(LIFE_TIME, 30);
}

#[test]
fn spawns_at_most_sixteen_inside_the_box_moving_up() {
    let mut emitter = TotemEmitter::new(haste_core::EntityId(7));
    let mut rng = rng();
    let pos = DVec3::new(10.0, 64.0, -3.0);
    let (w, h) = (0.6f32, 1.8f32);
    for _ in 0..50 {
        let mut out = Vec::new();
        emitter.tick(pos, (w, h), &mut rng, &mut out);
        assert!(out.len() <= 16);
        for s in &out {
            // The offsets sit in the unit sphere, and the particle is shifted by +0.2 on y.
            let v = s.offset;
            assert!(v[0] * v[0] + (v[1] - 0.2).powi(2) + v[2] * v[2] <= 1.0 + 1e-6);
            // Positions stay within a quarter of the box around the entity.
            assert!((s.pos.x - pos.x).abs() <= (w / 4.0) as f64 + 1e-9);
            assert!((s.pos.z - pos.z).abs() <= (w / 4.0) as f64 + 1e-9);
            assert!(s.pos.y >= pos.y + h as f64 * 0.25 - 1e-9);
            assert!(s.pos.y <= pos.y + h as f64 * 0.75 + 1e-9);
        }
    }
}
