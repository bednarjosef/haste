// --- Vanilla-number tests: the rocket's flight trail (`FireworkRocketEntity.tick`'s client branch). ---

use super::*;

/// A pool with a `firework` type, its camera parked on the rocket (the trail is limiter
/// checked against the eye like every client add).
fn pool_with_camera_at(pos: DVec3) -> Particles {
    let types = types_for_tests(&[("firework", &[[0, 0, 1, 1]])]);
    let mut particles = Particles::new(types);
    particles.camera = pos;
    particles
}

#[test]
fn trail_emits_one_firework_particle_per_tick_at_the_rocket() {
    let pos = DVec3::new(10.0, 64.0, -5.0);
    let mut particles = pool_with_camera_at(pos);
    particles.firework_rocket_trail(pos, DVec3::new(0.0, 0.5, 0.0));
    assert_eq!(particles.alive(), 1);
    assert_eq!(particles.pool[0].pos, pos.as_vec3());
}

#[test]
fn trail_falls_at_half_the_rockets_vertical_speed() {
    // `-deltaMovement.y * 0.5`: a rocket climbing at 0.5 blocks a tick leaves particles
    // drifting down at 0.25, and one falling at 0.4 leaves them rising at 0.2.
    let pos = DVec3::new(10.0, 64.0, -5.0);
    let mut particles = pool_with_camera_at(pos);
    particles.firework_rocket_trail(pos, DVec3::new(0.0, 0.5, 0.0));
    assert_eq!(particles.pool[0].vel.y, -0.25);
    particles.firework_rocket_trail(pos, DVec3::new(0.0, -0.4, 0.0));
    assert_eq!(particles.pool[1].vel.y, 0.2);
}

#[test]
fn trail_drift_on_x_and_z_is_gaussian_times_point_zero_five() {
    // `nextGaussian() * 0.05` on X and Z: the gaussian is bounded well inside 6 sigma, so the
    // drift is well inside 0.3 for a rocket at rest; a level rocket has no vertical speed.
    let pos = DVec3::new(10.0, 64.0, -5.0);
    let mut particles = pool_with_camera_at(pos);
    for _ in 0..64 {
        particles.firework_rocket_trail(pos, DVec3::ZERO);
    }
    assert_eq!(particles.alive(), 64);
    let mut moved = false;
    for p in &particles.pool {
        assert_eq!(p.vel.y, 0.0);
        assert!(p.vel.x.abs() < 0.3 && p.vel.z.abs() < 0.3);
        moved |= p.vel.x != 0.0 || p.vel.z != 0.0;
    }
    assert!(moved, "the drift is random, not all zero");
}
