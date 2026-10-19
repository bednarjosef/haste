//! The client's own item particles (vanilla `BreakingItemParticle`, what an
//! `ItemParticleOption` makes): eating and drinking's crumbs in front of the mouth,
//! spawned by [`crate::item_particles`] through `LivingEntity.spawnItemParticles`.

use glam::{DVec3, Vec3};
use haste_core::RegistryIndex;

use super::engine::*;
use super::providers::ParticleBehaviour;
use super::*;

impl Particles {
    /// One `BreakingItemParticle` (vanilla
    /// `BreakingItemParticle.ItemParticleProvider.createParticle`): the item's sprite (a
    /// random active layer's, `pickParticleMaterial`), gravity 1, the `SingleQuadParticle`
    /// quad halved, the 4x4 uv sub-piece of a random `uo`/`vo`, and the `Particle` speed
    /// constructor's velocity with a zero packet speed scaled by 0.1 plus the caller's
    /// (`LivingEntity.spawnItemParticles`' `d`, with its `+0.05` on y). No distance
    /// limiter: vanilla's `ParticleEngine.add` bypasses it.
    pub fn spawn_item(&mut self, item: RegistryIndex, pos: DVec3, vel: [f32; 3]) {
        if self.pool.len() >= MAX_PARTICLES {
            return;
        }
        let Some(types) = self.types.as_deref() else {
            return;
        };
        let rng = &mut self.rng;
        // The particle's own random draws, in vanilla's constructor order: lifetime,
        // speed-constructor jitter and scale, quadSize, then uo/vo.
        let lifetime = (4.0 / (rng.next_f32() * 0.9 + 0.1)).floor().max(1.0);
        let mut v = Vec3::new(
            (rng.next_f32() * 2.0 - 1.0) * 0.4,
            (rng.next_f32() * 2.0 - 1.0) * 0.4,
            (rng.next_f32() * 2.0 - 1.0) * 0.4,
        );
        let scale = (rng.next_f32() + rng.next_f32() + 1.0) * 0.15;
        let len = v.length();
        if len > 0.0 {
            v *= scale * 0.4 / len;
        }
        v.y += 0.1;
        let vel = v * 0.1 + Vec3::from(vel);
        let quad = 0.1 * (rng.next_f32() * 0.5 + 0.5) * 2.0 * 0.5;
        // The layers' rects (a random one) and the 4x4 sub-piece inside it.
        let layers = types.items.sprites(item);
        let rect = layers[(rng.next_f32() * layers.len() as f32) as usize % layers.len()];
        let [x, y, w, h] = rect;
        let (uo, vo) = (rng.next_f32() * 3.0, rng.next_f32() * 3.0);
        let sub = BlockSpriteUv {
            rect,
            uv: [
                x as f32 + w as f32 * uo / 4.0,
                y as f32 + h as f32 * vo / 4.0,
                x as f32 + w as f32 * (uo + 1.0) / 4.0,
                y as f32 + h as f32 * (vo + 1.0) / 4.0,
            ],
        };
        let pos32 = pos.as_vec3();
        let state = ParticleState {
            kind: 0,
            pos: pos32,
            prev_pos: pos32,
            vel,
            age: 0.0,
            lifetime,
            behaviour: ParticleBehaviour {
                source: "BreakingItemParticle",
                gravity: 1.0,
                ..ParticleBehaviour::DEFAULT
            },
            quad,
            color: [1.0; 3],
            alpha: 1.0,
            portal_start: pos,
            block_sprite: Some(sub),
        };
        self.pool.push(state);
    }
}
