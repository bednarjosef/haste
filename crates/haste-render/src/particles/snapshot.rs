//! The per-tick render snapshot (vanilla `ParticleEngine.extract` -> `QuadParticleGroup`
//! `.extractRenderState`): the game thread poses the pool once per tick under the locks it
//! already holds, and the render thread draws its own `Arc` copy without the world read
//! lock, the particle mutex or the game tick running.

use std::sync::Arc;

use glam::Vec3;
use haste_core::view::BlockView;
use haste_core::BlockPos;

use super::engine::{BlockSpriteUv, Particles};
use super::single_quad_particle::QuadGrowth;

/// One particle as the renderer sees it: everything a frame's draw needs, resolved once
/// per tick. Plain data, no pointers, so the snapshot is one `Vec` the game thread refills
/// and the render thread reads through an `Arc`.
#[derive(Clone, Copy, Debug)]
pub struct ParticleView {
    /// Position at the last tick's start (the render thread lerps from here).
    pub prev_pos: Vec3,
    /// Position at the last tick's end.
    pub pos: Vec3,
    /// Age in ticks at the tick boundary, and the lifetime, for the quad growth
    /// (`getQuadSize`); the renderer folds the partial tick in.
    pub age: f32,
    pub lifetime: f32,
    /// Quad half-width rolled at spawn (before the growth).
    pub quad: f32,
    /// Colour (`rCol`/`gCol`/`bCol`) and alpha.
    pub color: [f32; 3],
    pub alpha: f32,
    /// Sky and block light at the tick position (`Particle.getLightCoords`).
    pub sky_light: u8,
    pub block_light: u8,
    /// The sprite rect (the shader's atlas clamp) and the uv corners.
    pub rect: [u16; 4],
    pub uvs: [[f32; 2]; 4],
    /// The quad growth over the life (`getQuadSize`).
    pub growth: QuadGrowth,
}

/// A published snapshot: the immutable `Vec` the render thread holds for as long as it
/// keeps the `Arc` (one frame), plus the alive count for the debug log.
#[derive(Debug, Default)]
pub struct ParticleSnapshot {
    pub views: Vec<ParticleView>,
    /// The pool's alive count at the tick (the once-a-second debug log's denominator).
    pub alive: usize,
}

/// The double buffer between the threads: the game thread's particle tick builds the
/// snapshot into the back slot (reusing its `Vec`) and publishes it; the render thread
/// takes the `Arc` for a frame. After warmup a tick with a steady particle count
/// allocates nothing. Both slots sit behind short-held mutexes so the game thread can
/// publish through a `&Shared` and the render thread never blocks the tick.
#[derive(Default)]
pub struct ParticleSnapshots {
    /// The slot the game thread refills each tick.
    back: parking_lot::Mutex<ParticleSnapshot>,
    /// The published snapshot (`arc-swap` is not a dependency; a `Mutex<Option<Arc>>`
    /// with a lock held only for the pointer swap is the same at frame rate).
    published: parking_lot::Mutex<Option<Arc<ParticleSnapshot>>>,
}

impl ParticleSnapshots {
    /// The game thread's end-of-tick publish: `build` fills the back slot, then the slot
    /// is swapped for the previous `Arc` under the short lock. Never blocks on the
    /// render thread: the old buffer comes back for the next tick's reuse once the
    /// renderer has let go of it.
    pub fn publish(&self, build: impl FnOnce(&mut Vec<ParticleView>) -> usize) {
        let mut back = self.back.lock();
        back.views.clear();
        back.alive = build(&mut back.views);
        let published = Arc::new(std::mem::take(&mut *back));
        let old = self.published.lock().replace(published);
        // The slot's new back buffer: the old published snapshot when the renderer has
        // already dropped it (its `Vec` back to capacity), else fresh (the renderer is
        // still on the previous frame's copy).
        *back = match Arc::try_unwrap(old.unwrap_or_default()) {
            Ok(mut reused) => {
                reused.views.clear();
                reused
            }
            Err(_) => ParticleSnapshot::default(),
        };
    }

    /// The render thread's take: clones the `Arc` (one refcount bump) under the short
    /// lock. `None` before the first tick published anything.
    pub fn take(&self) -> Option<Arc<ParticleSnapshot>> {
        self.published.lock().clone()
    }
}

impl Particles {
    /// Builds this tick's snapshot (vanilla `QuadParticleGroup.extractRenderState` minus
    /// the frustum test, which is the camera's and stays with the renderer): every live
    /// particle's render pose, with its light sampled at the tick position
    /// (`Particle.getLightCoords` reads `x`/`y`/`z` — the tick position, not the
    /// interpolated one). Vanilla re-runs it every frame; here it runs once per tick,
    /// which changes nothing the player can see: a light change applies within a frame
    /// instead of the same frame.
    pub fn build_snapshot(&self, world: &dyn BlockView, out: &mut Vec<ParticleView>) {
        let Some(types) = self.types.as_deref() else {
            return;
        };
        out.reserve(self.pool.len());
        for p in &self.pool {
            if p.behaviour.no_render {
                // NoRenderParticle subclasses (the explosion emitter seed): the pool
                // entry drives the tick that spawns the visible particles.
                continue;
            }
            let block = BlockPos::new(
                p.pos.x.floor() as i32,
                p.pos.y.floor() as i32,
                p.pos.z.floor() as i32,
            );
            let light = if p.behaviour.full_light {
                // HugeExplosionParticle.getLightCoords: always full light.
                (15u8, 15u8)
            } else {
                (world.block_light(block), world.sky_light(block))
            };
            // The sprite for the particle's age (vanilla `setSpriteFromAge`), resolved
            // once per tick instead of per frame.
            let (rect, uvs) = match p.block_sprite {
                // A terrain particle: its block sprite's fixed 4x4 sub-piece.
                Some(s) => terrain_uvs(&s),
                None => {
                    let sprites = types.sprites(p.kind);
                    if sprites.is_empty() {
                        continue;
                    }
                    sprite_uvs(sprites[age_sprite(p.age, p.lifetime, sprites.len())])
                }
            };
            out.push(ParticleView {
                prev_pos: p.prev_pos,
                pos: p.pos,
                age: p.age,
                lifetime: p.lifetime,
                quad: p.quad,
                color: p.color,
                alpha: p.behaviour.alpha.unwrap_or(1.0),
                sky_light: light.1,
                block_light: light.0,
                rect,
                uvs,
                growth: p.behaviour.growth,
            });
        }
    }

    /// The game thread's end-of-tick publish into `snapshots` (see
    /// [`ParticleSnapshots::publish`]); the world is read here, where the game thread
    /// already holds it.
    pub fn publish_snapshot(&self, world: &dyn BlockView, snapshots: &ParticleSnapshots) {
        snapshots.publish(|views| {
            self.build_snapshot(world, views);
            self.alive()
        });
    }
}

/// The sprite index for a particle's age (vanilla `setSpriteFromAge`:
/// `age * sprites.len() / lifetime`, clamped to the last).
pub fn age_sprite(age: f32, lifetime: f32, len: usize) -> usize {
    (((age / lifetime) * len as f32).floor() as usize).min(len - 1)
}

/// The uv corners of an atlas sprite rect (vanilla `getU0`..`getV1`), corner order
/// bottom-left, bottom-right, top-right, top-left.
pub(super) fn sprite_uvs(rect: [u16; 4]) -> ([u16; 4], [[f32; 2]; 4]) {
    let (x, y, w, h) = (
        rect[0] as f32,
        rect[1] as f32,
        rect[2] as f32,
        rect[3] as f32,
    );
    (rect, [[x, y + h], [x + w, y + h], [x + w, y], [x, y]])
}

/// The uv corners of a terrain particle's 4x4 sub-piece (`BlockSpriteUv::uv`:
/// `[u0, v0, u1, v1]` in atlas texels).
pub(super) fn terrain_uvs(s: &BlockSpriteUv) -> ([u16; 4], [[f32; 2]; 4]) {
    (
        s.rect,
        [
            [s.uv[0], s.uv[3]],
            [s.uv[2], s.uv[3]],
            [s.uv[2], s.uv[1]],
            [s.uv[0], s.uv[1]],
        ],
    )
}
