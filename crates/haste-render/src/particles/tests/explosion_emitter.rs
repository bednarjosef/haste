//! The explosion emitter seed (`HugeExplosionSeedParticle`, a NoRenderParticle): its tick
//! spawns the visible `explosion` particles, six per tick for eight ticks, each sized by
//! the seed's age — the numbers vanilla's class carries.

use glam::Vec3;
use haste_core::view::BlockView;
use haste_core::BlockPos;

use super::*;

/// A world that answers one fixed sky/block light pair (as the snapshot tests' `FlatLight`).
struct FlatLight(u8, u8);
impl BlockView for FlatLight {
    fn block_state(&self, _p: BlockPos) -> haste_core::BlockStateId {
        haste_core::BlockStateId::AIR
    }
    fn is_chunk_loaded(&self, _p: haste_core::ChunkPos) -> bool {
        true
    }
    fn sky_light(&self, _p: BlockPos) -> u8 {
        self.0
    }
    fn block_light(&self, _p: BlockPos) -> u8 {
        self.1
    }
    fn min_y(&self) -> i32 {
        0
    }
    fn height(&self) -> i32 {
        256
    }
}

/// Spawns one seed at the fixed point and ticks it once, like the game thread does.
fn seed_and_tick(particles: &mut Particles, camera: DVec3) {
    let mut request = spawn_request("explosion_emitter", 0);
    request.pos = DVec3::new(10.0, 64.0, -5.0);
    request.offset = [0.0, 0.0, 0.0];
    request.max_speed = [0.0, 0.0, 0.0];
    particles.spawn(&request, camera, None);
    particles.tick(camera);
}

/// The seed itself: one pool entry with no atlas sprites (vanilla registers no sprite
/// set for the type), alive for eight ticks.
#[test]
fn seed_lives_eight_ticks_and_never_renders() {
    let mut particles = Particles::new(types_for_tests(&[
        ("explosion_emitter", &[]),
        ("explosion", &[[0, 0, 4, 4]]),
    ]));
    let camera = DVec3::new(10.0, 64.0, -5.0);
    let mut request = spawn_request("explosion_emitter", 0);
    request.pos = camera;
    request.offset = [0.0, 0.0, 0.0];
    request.max_speed = [0.0, 0.0, 0.0];
    particles.spawn(&request, camera, None);
    assert_eq!(particles.alive(), 1);
    // NoRenderParticle: the snapshot carries nothing for the seed.
    let mut views = Vec::new();
    particles.build_snapshot(&FlatLight(15, 0), &mut views);
    assert!(views.is_empty());
    // Eight ticks alive, gone on the ninth (vanilla: `age++ == lifetime` removes it).
    for tick in 0..8 {
        particles.tick(camera);
        assert!(
            particles.pool.iter().any(|p| p.behaviour.no_render),
            "seed alive at tick {tick}"
        );
    }
    particles.tick(camera);
    assert!(
        !particles.pool.iter().any(|p| p.behaviour.no_render),
        "seed expired after eight ticks"
    );
}

/// Six `explosion` particles per tick, 48 over the seed's life, their quad
/// `2 * (1 - size/2)` with size the tick's `age/lifetime` (0-based, so 0/8 ..= 7/8).
#[test]
fn seed_spawns_six_shrinking_explosions_per_tick() {
    let mut particles = Particles::new(types_for_tests(&[
        ("explosion_emitter", &[]),
        ("explosion", &[[0, 0, 4, 4]]),
    ]));
    let camera = DVec3::new(10.0, 64.0, -5.0);
    let mut request = spawn_request("explosion_emitter", 0);
    request.pos = camera;
    request.offset = [0.0, 0.0, 0.0];
    request.max_speed = [0.0, 0.0, 0.0];
    particles.spawn(&request, camera, None);
    let children_start = particles.alive();
    for tick in 1..=8u32 {
        particles.tick(camera);
        // The tick's six children are appended after the loop, so they sit at the tail.
        let newest = &particles.pool[particles.pool.len() - 6..];
        assert_eq!(newest.len(), 6, "six children at tick {tick}");
        // The size this tick: (tick - 1)/8, quad 2 * (1 - size * 0.5).
        let size = f64::from(tick - 1) / 8.0;
        let quad = 2.0 * (1.0 - size as f32 * 0.5);
        for p in newest {
            assert_eq!(p.behaviour.source, "HugeExplosionParticle");
            assert!(
                (p.quad - quad).abs() < 1e-6,
                "quad {} at tick {tick}",
                p.quad
            );
            // HugeExplosionParticle: life 6 + r(0..4), grey 0.4..1.0, full light, no
            // motion, and the spawn position jittered by ±4 around the seed.
            assert!((6.0..=9.0).contains(&p.lifetime), "life {}", p.lifetime);
            assert!((0.4..=1.0).contains(&p.color[0]), "grey {}", p.color[0]);
            assert!(p.vel == Vec3::ZERO, "no motion: {:?}", p.vel);
            assert!(p.behaviour.full_light);
            assert!((p.pos.x - 10.0).abs() <= 4.0 + 1e-6, "{}", p.pos.x);
            assert!((p.pos.y - 64.0).abs() <= 4.0 + 1e-6, "{}", p.pos.y);
            assert!((p.pos.z + 5.0).abs() <= 4.0 + 1e-6, "{}", p.pos.z);
        }
        // Cumulative: 48 children over the seed's life, the earliest (life 6) expiring
        // during the last ticks.
        let alive = particles.alive() - children_start;
        if tick <= 7 {
            assert_eq!(alive, 6 * tick as usize, "children after tick {tick}");
        } else {
            assert!(alive >= 47, "children after tick {tick}: {alive}");
        }
    }
}

/// The seed goes through `ClientLevel.doAddParticle`'s 32-block limiter like any
/// `level.addParticle` call: a camera 33 blocks away drops it (no emitter at all).
#[test]
fn seed_beyond_the_limiter_is_dropped() {
    let mut particles = Particles::new(types_for_tests(&[
        ("explosion_emitter", &[]),
        ("explosion", &[[0, 0, 4, 4]]),
    ]));
    // The seed within the limiter, the camera 33 blocks away: the seed itself drops.
    seed_and_tick(&mut particles, DVec3::new(43.0, 64.0, -5.0));
    assert_eq!(particles.alive(), 0, "the seed is dropped");
    // Camera back on the seed: the next tick's six children spawn.
    let mut particles = Particles::new(types_for_tests(&[
        ("explosion_emitter", &[]),
        ("explosion", &[[0, 0, 4, 4]]),
    ]));
    seed_and_tick(&mut particles, DVec3::new(10.0, 64.0, -5.0));
    assert_eq!(particles.alive(), 7);
}
