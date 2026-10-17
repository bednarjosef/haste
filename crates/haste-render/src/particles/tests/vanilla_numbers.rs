use super::*;

// --- Vanilla-number tests: the per-type constants as vanilla computes them. ---

#[test]
fn flame_matches_rising_particle() {
    let mut particles = Particles::new(types_for_tests(&[("flame", &[[0, 0, 8, 8]])]));
    spawn_fixed(&mut particles, "flame", 200);
    for p in &particles.pool {
        let b = p.behaviour;
        // RisingParticle: friction 0.96, no gravity, life (int)(8/(r*0.8+0.2))+4 = 12..=44.
        assert_eq!(b.friction, 0.96);
        assert_eq!(b.gravity, 0.0);
        assert!((12.0..=44.0).contains(&p.lifetime), "life {}", p.lifetime);
        // SingleQuadParticle's default quad, 0.1..=0.2 half-width, unchanged for flames.
        assert!((0.1..=0.2).contains(&p.quad), "quad size {}", p.quad);
        // RisingParticle jitters the spawn position by ±0.05 per axis.
        assert!((p.pos.x - 10.0).abs() <= 0.05 + 1e-6, "{}", p.pos.x);
        assert!((p.pos.y - 64.0).abs() <= 0.05 + 1e-6, "{}", p.pos.y);
        // FlameParticle.getQuadSize shrinks the quad over the life.
        assert_eq!(b.growth.scale(1.0), 0.5);
    }
}

#[test]
fn smoke_matches_base_ash_smoke_particle() {
    let mut particles = Particles::new(types_for_tests(&[("smoke", &[[0, 0, 4, 4]])]));
    spawn_fixed(&mut particles, "smoke", 200);
    for p in &particles.pool {
        let b = p.behaviour;
        // BaseAshSmokeParticle with scale 1: friction 0.96, gravity -0.1, life
        // (int)(8/(r*0.8+0.2)) = 8..=40, quadSize *= 0.75.
        assert_eq!(b.friction, 0.96);
        assert_eq!(b.gravity, -0.1);
        assert!((8.0..=40.0).contains(&p.lifetime), "life {}", p.lifetime);
        assert!((0.075..=0.15).contains(&p.quad), "quad size {}", p.quad);
        // The colour is grey, from random * 0.3.
        assert!((0.0..=0.3).contains(&p.color[0]), "grey {}", p.color[0]);
        assert_eq!(p.color[0], p.color[1]);
        assert_eq!(p.color[1], p.color[2]);
        // BaseAshSmokeParticle.getQuadSize: full size after 1/32 of the life.
        assert_eq!(b.growth.scale(0.0), 0.0);
        assert_eq!(b.growth.scale(1.0), 1.0);
    }
}

#[test]
fn large_smoke_is_smoke_at_scale_two_point_five() {
    let mut particles = Particles::new(types_for_tests(&[("large_smoke", &[[0, 0, 4, 4]])]));
    spawn_fixed(&mut particles, "large_smoke", 200);
    for p in &particles.pool {
        // LargeSmokeParticle passes scale 2.5: life 20..=100, quad 1.875x the default.
        assert!((20.0..=100.0).contains(&p.lifetime), "life {}", p.lifetime);
        assert!((0.1875..=0.375).contains(&p.quad), "quad size {}", p.quad);
    }
}

#[test]
fn crit_matches_crit_particle() {
    let mut particles = Particles::new(types_for_tests(&[("crit", &[[0, 0, 4, 4]])]));
    spawn_fixed(&mut particles, "crit", 200);
    for i in 0..particles.pool.len() {
        let b = particles.pool[i].behaviour;
        // CritParticle: gravity 0.5, friction 0.7, quad 0.75x, no collision.
        assert_eq!(b.gravity, 0.5);
        assert_eq!(b.friction, 0.7);
        // Its lifetime is max((int)(6/(r*0.8+0.6)), 1) = 4..=10.
        assert!(
            (4.0..=10.0).contains(&particles.pool[i].lifetime),
            "life {}",
            particles.pool[i].lifetime
        );
        assert!(
            (0.075..=0.15).contains(&particles.pool[i].quad),
            "quad size {}",
            particles.pool[i].quad
        );
        // Its colour starts bright grey (0.6 + r*0.3).
        assert!(
            (0.6..=0.9).contains(&particles.pool[i].color[0]),
            "grey {}",
            particles.pool[i].color[0]
        );
        // The constructor runs one tick, so the particles are already aged one.
        assert_eq!(particles.pool[i].age, 1.0);
    }
    // CritParticle.tick: gCol *= 0.96 and bCol *= 0.9 each tick, on one particle alone.
    let mut particles = Particles::new(types_for_tests(&[("crit", &[[0, 0, 4, 4]])]));
    spawn_fixed(&mut particles, "crit", 1);
    let before = particles.pool[0].color;
    particles.tick();
    let after = &particles.pool[0];
    assert!((after.color[1] - before[1] * 0.96).abs() < 1e-6);
    assert!((after.color[2] - before[2] * 0.9).abs() < 1e-6);
}

#[test]
fn portal_matches_portal_particle() {
    let mut particles = Particles::new(types_for_tests(&[("portal", &[[0, 0, 4, 4]])]));
    spawn_fixed(&mut particles, "portal", 200);
    for p in &particles.pool {
        // PortalParticle: life (int)(r*10) + 40 = 40..=49, quad 0.1*(r*0.2+0.5).
        assert!((40.0..=49.0).contains(&p.lifetime), "life {}", p.lifetime);
        assert!((0.05..=0.0700001).contains(&p.quad), "quad size {}", p.quad);
        // Purple: br*0.9, br*0.3, br with br in 0.4..=1.0.
        assert!(p.color[2] >= p.color[0]);
        assert!(p.color[0] >= p.color[1]);
        assert!((0.36..=0.9).contains(&p.color[0]), "r {}", p.color[0]);
        // getQuadSize grows towards the end of the life: 1-(1-s)².
        let b = p.behaviour;
        assert_eq!(b.growth.scale(0.0), 0.0);
        assert_eq!(b.growth.scale(0.5), 0.75);
        assert_eq!(b.growth.scale(1.0), 1.0);
    }
    // The spiral path: y falls from one block above the spawn point back to it (the
    // `(1 - a)` term), while x/z track the spawn point plus the scaled packet velocity.
    let mut particles = Particles::new(types_for_tests(&[("portal", &[[0, 0, 4, 4]])]));
    let mut request = spawn_request("portal", 0);
    request.max_speed = [0.0, 0.0, 0.0];
    particles.spawn(&request, request.pos, None);
    let start = particles.pool[0].pos;
    let lifetime = particles.pool[0].lifetime;
    let mut ticks = 0;
    // The first tick lifts the particle to (1 - 1/lifetime) blocks above the spawn point.
    particles.tick();
    ticks += 1;
    let mut last_y = particles.pool[0].pos.y;
    assert!((last_y - start.y - (1.0 - 1.0 / lifetime)).abs() < 1e-4);
    let mut last_x = f32::NAN;
    let mut last_z = f32::NAN;
    while alive(&particles) {
        let before_y = last_y;
        particles.tick();
        ticks += 1;
        if alive(&particles) {
            assert!(particles.pool[0].pos.y < before_y);
            last_y = particles.pool[0].pos.y;
            last_x = particles.pool[0].pos.x;
            last_z = particles.pool[0].pos.z;
        }
    }
    // Without a packet velocity the x/z never move.
    assert_eq!(last_x, start.x);
    assert_eq!(last_z, start.z);
    // It survives its lifetime ticks of motion and is removed on the one after
    // (`age++ >= lifetime`, so the last moving tick has age == lifetime).
    assert_eq!(ticks as f32, lifetime + 1.0);
}

#[test]
fn explosion_matches_huge_explosion_particle() {
    let mut particles = Particles::new(types_for_tests(&[("explosion", &[[0, 0, 4, 4]])]));
    let mut request = spawn_request("explosion", 0);
    // The x speed is the size: quad = 2 * (1 - size/2), size = offset * max_speed.
    request.max_speed = [0.5, 0.0, 0.0];
    request.offset = [1.0, 0.0, 0.0];
    particles.spawn(&request, request.pos, None);
    let p = &particles.pool[0];
    assert!((6.0..=9.0).contains(&p.lifetime), "life {}", p.lifetime);
    assert_eq!(p.quad, 2.0 * (1.0 - 0.5 * 0.5));
    assert!(p.behaviour.full_light);
    // It does not move.
    let pos = p.pos;
    particles.tick();
    assert_eq!(particles.pool[0].pos, pos);
}

#[test]
fn dust_scale_scales_lifetime_and_quad() {
    let mut particles = Particles::new(types_for_tests(&[("dust", &[[0, 0, 4, 4]])]));
    let mut request = spawn_request("dust", 0);
    request.dust = Some(DustOption {
        color: [1.0, 0.0, 0.0],
        scale: 2.0,
    });
    particles.spawn(&request, request.pos, None);
    let p = &particles.pool[0];
    // DustParticleBase: life (int)(8/(r*0.8+0.2) * 2) = 16..=80, quad 0.75*2 = 1.5x.
    assert!((16.0..=80.0).contains(&p.lifetime), "life {}", p.lifetime);
    assert!((0.15..=0.3).contains(&p.quad), "quad size {}", p.quad);
    // DustParticle randomises the red option colour by 0.8..=1.0 of its 0.6..=1.0 factor.
    assert!(p.color[0] > 0.0 && p.color[0] <= 1.0, "r {}", p.color[0]);
    assert_eq!(p.color[1], 0.0);
    assert_eq!(p.color[2], 0.0);
}
