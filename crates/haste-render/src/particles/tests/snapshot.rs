//! Tests for the per-tick render snapshot and the render thread's frustum cull.

use super::particle_renderer::point_visible;
use super::*;

use haste_core::view::BlockView;
use haste_core::BlockPos;

/// A world that answers one fixed sky/block light pair (the engine's light sampling
/// only needs a value per block).
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

/// The snapshot carries every pool particle's pose, light and sprite.
#[test]
fn snapshot_poses_light_and_sprites() {
    let mut particles = Particles::new(types_for_tests(&[("flame", &[[0, 0, 8, 8]])]));
    spawn_fixed(&mut particles, "flame", 3);
    particles.tick(DVec3::new(10.0, 64.0, -5.0));
    let mut views = Vec::new();
    particles.build_snapshot(&FlatLight(11, 3), &mut views);
    assert_eq!(views.len(), 3);
    for (v, p) in views.iter().zip(&particles.pool) {
        assert_eq!(v.prev_pos, p.prev_pos);
        assert_eq!(v.pos, p.pos);
        assert_eq!(v.age, p.age);
        assert_eq!(v.lifetime, p.lifetime);
        assert_eq!(v.quad, p.quad);
        // The sampled light, not the pool's UNSAMPLED placeholder.
        assert_eq!((v.block_light, v.sky_light), (3, 11));
        // The flame sprite rect with vanilla's uv corner order.
        assert_eq!(v.rect, [0, 0, 8, 8]);
        assert_eq!(v.uvs[0], [0.0, 8.0]);
        assert_eq!(v.uvs[1], [8.0, 8.0]);
        assert_eq!(v.uvs[2], [8.0, 0.0]);
        assert_eq!(v.uvs[3], [0.0, 0.0]);
    }
}

/// Full-light particles (explosions) ignore the sampled light, like
/// `HugeExplosionParticle.getLightCoords`.
#[test]
fn snapshot_full_light_particles_stay_full() {
    let mut particles = Particles::new(types_for_tests(&[("explosion", &[[0, 0, 4, 4]])]));
    let mut request = spawn_request("explosion", 0);
    request.max_speed = [0.5, 0.0, 0.0];
    request.offset = [1.0, 0.0, 0.0];
    particles.spawn(&request, request.pos, None);
    let mut views = Vec::new();
    particles.build_snapshot(&FlatLight(2, 1), &mut views);
    assert_eq!(views.len(), 1);
    assert_eq!((views[0].block_light, views[0].sky_light), (15, 15));
}

/// A multi-sprite type animates through its sprite list with age (`setSpriteFromAge`).
#[test]
fn snapshot_picks_the_age_sprite() {
    let sprites = &[[0, 0, 4, 4], [4, 0, 4, 4], [8, 0, 4, 4]];
    let mut particles = Particles::new(types_for_tests(&[("smoke", sprites)]));
    spawn_fixed(&mut particles, "smoke", 1);
    // Default Particle life 4..=40: tick until the age covers the sprite list.
    let lifetime = particles.pool[0].lifetime;
    let mut last_rect = None;
    for _ in 0..lifetime as usize + 1 {
        if !alive(&particles) {
            break;
        }
        let mut views = Vec::new();
        particles.build_snapshot(&FlatLight(15, 0), &mut views);
        if let Some(v) = views.first() {
            last_rect = Some(v.rect);
            let index = crate::particles::snapshot::age_sprite(v.age, v.lifetime, sprites.len());
            assert_eq!(v.rect, sprites[index]);
        }
        particles.tick(DVec3::new(10.0, 64.0, -5.0));
    }
    // The last surviving sprite is the list's last.
    assert_eq!(last_rect, Some(sprites[2]));
}

/// The publish/take handover: a take sees the last tick's data, a second publish
/// reuses the buffer the renderer dropped, and the alive count rides along.
#[test]
fn snapshot_double_buffer_publishes_and_reuses() {
    let mut particles = Particles::new(types_for_tests(&[("smoke", &[[0, 0, 4, 4]])]));
    let snapshots = ParticleSnapshots::default();
    assert!(snapshots.take().is_none(), "nothing published yet");
    particles.publish_snapshot(&FlatLight(15, 0), &snapshots);
    {
        let taken = snapshots.take().expect("published");
        assert_eq!(taken.alive, 0);
        assert!(taken.views.is_empty());
    }
    spawn_fixed(&mut particles, "smoke", 2);
    particles.tick(DVec3::new(10.0, 64.0, -5.0));
    particles.publish_snapshot(&FlatLight(15, 0), &snapshots);
    let taken = snapshots.take().expect("published");
    assert_eq!(taken.alive, 2);
    assert_eq!(taken.views.len(), 2);
    // The next publish takes the dropped buffer back (same allocation, no growth).
    particles.publish_snapshot(&FlatLight(15, 0), &snapshots);
    let again = snapshots.take().expect("published");
    assert_eq!(again.views.len(), 2);
}

/// The per-particle frustum cull (vanilla `QuadParticleGroup.extractRenderState`'s
/// `frustum.pointInFrustum`): a point in front of a forward-looking camera passes, one
/// behind it does not; the 3-block cull-camera offset keeps just-behind points alive.
#[test]
fn frustum_cull_matches_vanilla_point_test() {
    use crate::renderer::{Camera, Renderer};
    let camera = Camera {
        pos: glam::DVec3::new(0.0, 64.0, 0.0),
        yaw: 0.0,
        pitch: 0.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: Default::default(),
    };
    let aspect = 16.0 / 9.0;
    let planes = Renderer::frustum_planes(&Renderer::view_projection(
        &Camera {
            pos: glam::DVec3::ZERO,
            ..camera
        },
        aspect,
        None,
    ));
    // Yaw 0 pitch 0 looks along +z (Minecraft's convention at the renderer).
    let forward = glam::Vec3::new(0.0, 0.0, 1.0);
    let shift = forward * 3.0;
    let visible = |world: glam::Vec3| point_visible(&planes, world - camera.pos.as_vec3() + shift);
    // In front of the camera (into the screen at yaw 0): drawn.
    assert!(visible(glam::Vec3::new(0.0, 64.0, 10.0)));
    // Behind the camera: culled.
    assert!(!visible(glam::Vec3::new(0.0, 64.0, -10.0)));
    // Within the 3-block cull-camera offset behind the eye: still drawn (vanilla's
    // `offset(-3.0F)` keeps particles just behind the camera visible).
    assert!(visible(glam::Vec3::new(0.0, 64.0, -2.0)));
    // Far off to the side (outside the horizontal fov): culled.
    assert!(!visible(glam::Vec3::new(100.0, 64.0, 10.0)));
}
