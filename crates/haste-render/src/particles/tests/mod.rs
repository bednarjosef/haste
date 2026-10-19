mod explosion_emitter;
mod item_particle;
mod snapshot;
mod terrain_particle;
mod vanilla_numbers;

use std::sync::Arc;

use haste_core::view::CollisionShapes;
use haste_core::BlockPos;

use super::engine::*;
use super::particle_renderer::*;
use super::resources::*;
use super::rng::*;
use super::*;

/// A type table with hand-written sprite lists (no assets needed).
fn types_for_tests(entries: &[(&str, &[[u16; 4]])]) -> Arc<ParticleTypes> {
    let mut types = ParticleTypes {
        types: Vec::new(),
        by_name: Default::default(),
        blocks: None,
        items: Default::default(),
        pixels: Vec::new(),
        size: 0,
    };
    for (name, sprites) in entries {
        types.types.push(ParticleType {
            name: Identifier::minecraft(name),
            sprites: sprites.to_vec(),
        });
        types
            .by_name
            .insert(Identifier::minecraft(name), (types.types.len() - 1) as u32);
    }
    Arc::new(types)
}

fn spawn_request(kind: &str, count: i32) -> ParticleSpawn {
    ParticleSpawn {
        kind: Identifier::minecraft(kind),
        pos: DVec3::new(10.0, 64.0, -5.0),
        offset: [0.5, 0.0, 0.25],
        max_speed: [0.1, 0.0, 0.05],
        count,
        randomization: 0,
        override_limiter: false,
        dust: None,
        block_state: None,
    }
}

/// Spawns `count` particles of `kind` at a fixed point (no packet spread) with a camera
/// parked on the spot, so the vanilla 32-block limiter and the packet's own gaussian
/// spread do not mix into the numbers under test.
fn spawn_fixed(particles: &mut Particles, kind: &str, count: i32) {
    let mut request = spawn_request(kind, count);
    request.pos = DVec3::new(10.0, 64.0, -5.0);
    request.offset = [0.0, 0.0, 0.0];
    request.max_speed = [0.0, 0.0, 0.0];
    particles.spawn(&request, request.pos, None);
}

/// Spawns with a 3-sigma gaussian spread from the packet, as `handleParticleEvent` does.
fn spawn_spread(particles: &mut Particles, kind: &str, count: i32) {
    let mut request = spawn_request(kind, count);
    request.pos = DVec3::new(10.0, 64.0, -5.0);
    particles.spawn(&request, request.pos, None);
}

/// True while the pool still has particles.
fn alive(particles: &Particles) -> bool {
    particles.alive() > 0
}

/// A types table whose block table maps every state to one 16x16 sprite (stone-like:
/// spawns terrain particles, untinted), for the block-particle tests.
fn types_with_blocks() -> Arc<ParticleTypes> {
    let mut types = ParticleTypes {
        types: Vec::new(),
        by_name: Default::default(),
        blocks: Some(crate::block_particles::BlockParticleTable::uniform(
            crate::block_particles::BlockParticleSprite {
                rect: [0, 0, 16, 16],
                tint: crate::tint::TintKind::None,
                spawn: true,
                upper_half_tint_below: false,
            },
        )),
        items: Default::default(),
        pixels: Vec::new(),
        size: 0,
    };
    types.types.push(ParticleType {
        name: Identifier::minecraft("flame"),
        sprites: vec![[0, 0, 8, 8]],
    });
    types.by_name.insert(Identifier::minecraft("flame"), 0);
    Arc::new(types)
}

/// One full-cube outline box (state 0), the shape stone has.
fn full_cube_shapes() -> impl CollisionShapes {
    struct FullCube;
    impl CollisionShapes for FullCube {
        fn collision_boxes(&self, _s: BlockStateId) -> &[haste_core::Aabb] {
            &[]
        }
        fn outline_boxes(&self, _s: BlockStateId) -> &[haste_core::Aabb] {
            const FULL: &[haste_core::Aabb] = &[haste_core::Aabb {
                min: DVec3::ZERO,
                max: DVec3::new(1.0, 1.0, 1.0),
            }];
            FULL
        }
    }
    FullCube
}

/// A biome source that knows one biome everywhere.
struct OneBiome;
impl crate::tint::BiomeSource for OneBiome {
    fn biome(&self, _pos: BlockPos) -> haste_core::BiomeId {
        haste_core::BiomeId(0)
    }
}

#[test]
fn count_zero_spawns_one_at_the_position() {
    let mut particles = Particles::new(types_for_tests(&[("flame", &[[0, 0, 8, 8]])]));
    particles.spawn(
        &spawn_request("flame", 0),
        DVec3::new(10.0, 64.0, -5.0),
        None,
    );
    assert_eq!(particles.alive(), 1);
    let p = &particles.pool[0];
    assert_eq!(p.prev_pos, p.pos);
    // RisingParticle keeps 1 % of the packet velocity inside the random direction.
    assert!(p.vel.length() > 0.0);
    // The position is the packet's, jittered by RisingParticle's ±0.05.
    assert!((p.pos.x - 10.0).abs() <= 0.05 + 1e-6);
    assert!((p.pos.y - 64.0).abs() <= 0.05 + 1e-6);
    assert!((p.pos.z + 5.0).abs() <= 0.05 + 1e-6);
}

#[test]
fn count_n_spreads_within_three_sigmas() {
    let mut particles = Particles::new(types_for_tests(&[("portal", &[[0, 0, 4, 4]])]));
    spawn_spread(&mut particles, "portal", 50);
    assert_eq!(particles.alive(), 50);
    for p in &particles.pool {
        // The gaussian approximations stay within ±3 sigmas.
        for (a, o) in p.pos.to_array().iter().zip([10.0, 64.0, -5.0]) {
            assert!((a - o).abs() <= 3.0 * 0.5 + 1e-5, "pos {:?}", p.pos);
        }
        // PortalParticle keeps the packet speed verbatim, so its velocity is exactly the
        // gaussian spread.
        for (v, m) in p.vel.to_array().iter().zip([0.1, 0.0, 0.05]) {
            assert!(v.abs() <= 3.0 * m + 1e-5, "vel {:?}", p.vel);
        }
    }
}

#[test]
fn pool_caps_at_max_particles() {
    let mut particles = Particles::new(types_for_tests(&[("smoke", &[[0, 0, 4, 4]])]));
    spawn_spread(&mut particles, "smoke", (MAX_PARTICLES * 2) as i32);
    assert_eq!(particles.alive(), MAX_PARTICLES);
}

#[test]
fn particles_expire_within_their_lifetime() {
    let mut particles = Particles::new(types_for_tests(&[("flame", &[[0, 0, 8, 8]])]));
    spawn_fixed(&mut particles, "flame", 1);
    let lifetime = particles.pool[0].lifetime as usize;
    // Alive through the lifetime ticks...
    for _ in 0..lifetime {
        particles.tick(DVec3::new(10.0, 64.0, -5.0));
    }
    assert_eq!(particles.alive(), 1);
    // ...expired one tick later.
    particles.tick(DVec3::new(10.0, 64.0, -5.0));
    assert_eq!(particles.alive(), 0);
}

#[test]
fn unknown_types_are_skipped_and_logged_once() {
    let mut particles = Particles::new(types_for_tests(&[("flame", &[[0, 0, 8, 8]])]));
    spawn_spread(&mut particles, "glow", 10);
    assert_eq!(particles.alive(), 0);
    assert_eq!(particles.unknown.len(), 1);
    // The same unknown type does not log a second time.
    spawn_spread(&mut particles, "glow", 10);
    assert_eq!(particles.unknown.len(), 1);
    // Types with an empty sprite list count as unknown too.
    spawn_fixed(&mut particles, "flame", 1);
    assert_eq!(particles.alive(), 1);
}

#[test]
fn spawns_beyond_the_vanilla_limiter_are_dropped() {
    let mut particles = Particles::new(types_for_tests(&[("smoke", &[[0, 0, 4, 4]])]));
    let mut request = spawn_request("smoke", 0);
    // 33 blocks on the x axis: `ClientLevel.doAddParticle` drops it.
    request.pos = DVec3::new(33.0, 0.0, 0.0);
    particles.spawn(&request, DVec3::ZERO, None);
    assert_eq!(particles.alive(), 0);
    // The packet can override the limiter.
    let mut request = spawn_request("smoke", 0);
    request.pos = DVec3::new(33.0, 0.0, 0.0);
    request.override_limiter = true;
    particles.spawn(&request, DVec3::ZERO, None);
    assert_eq!(particles.alive(), 1);
    // 31 blocks survives the limiter.
    let mut request = spawn_request("smoke", 0);
    request.pos = DVec3::new(31.0, 0.0, 0.0);
    particles.spawn(&request, DVec3::ZERO, None);
    assert_eq!(particles.alive(), 2);
}

#[test]
fn tick_moves_with_gravity_and_drag() {
    let mut particles = Particles::new(types_for_tests(&[("flame", &[[0, 0, 8, 8]])]));
    spawn_fixed(&mut particles, "flame", 1);
    let p = &particles.pool[0];
    // FlameParticle: no gravity, friction 0.96 (RisingParticle).
    assert_eq!(p.behaviour.gravity, 0.0);
    assert_eq!(p.behaviour.friction, 0.96);
    let vel = p.vel;
    let start = p.pos;
    particles.tick(DVec3::new(10.0, 64.0, -5.0));
    let p = &particles.pool[0];
    assert_eq!(p.prev_pos, start);
    assert_eq!(p.pos, start + vel);
    assert_eq!(p.vel, vel * 0.96);
}

#[test]
fn gaussian_is_bounded() {
    let mut rng = Rng(0x853C_49E6_748F_EA9B);
    for _ in 0..1000 {
        let g = rng.gaussian();
        assert!((-3.0..=3.0).contains(&g), "{g}");
    }
    let mut rng = Rng(1);
    for _ in 0..1000 {
        let g = rng.gaussian();
        assert!((-3.0..=3.0).contains(&g), "{g}");
    }
}

#[test]
fn billboard_axes_stay_orthogonal_at_all_pitches() {
    for pitch in [-90.0, -45.0, 0.0, 45.0, 90.0] {
        let (right, up) = billboard_axes(30.0, pitch);
        assert!(right.is_normalized(), "{right:?}");
        assert!(up.is_normalized(), "{up:?}");
        assert!(right.cross(up).length() > 0.99);
    }
}
