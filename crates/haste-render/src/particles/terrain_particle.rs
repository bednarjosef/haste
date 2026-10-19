//! The client's own terrain particles (vanilla `TerrainParticle`): block breaking's destroy
//! grid, mining cracks and the sprint particle.

use glam::{DVec3, Vec3};
use haste_core::view::CollisionShapes;
use haste_core::{BlockPos, BlockStateId, Direction};

use super::engine::*;
use super::providers::ParticleBehaviour;
use super::*;

impl Particles {
    /// A breaking block's terrain particles: vanilla
    /// `ClientLevel.addDestroyBlockEffect` — a grid over every box of the state's
    /// outline shape (`ceil(width/0.25)` per axis, at least 2), one `TerrainParticle`
    /// per cell at its centre with velocity `rel - 0.5`. Spawned by our own break
    /// prediction (`MultiPlayerGameMode.destroyBlock` -> `playerWillDestroy` ->
    /// `levelEvent(2001)` on the client) and for the server's `level_event` 2001.
    /// No distance limiter: vanilla's `ParticleEngine.add` bypasses it.
    pub fn spawn_destroy(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        shapes: &dyn CollisionShapes,
        biomes: &dyn crate::tint::BiomeSource,
    ) {
        let Some(sprite) = self
            .types
            .as_deref()
            .and_then(|t| t.blocks.as_ref())
            .and_then(|b| b.sprite(state))
            .filter(|s| s.spawn)
            .copied()
        else {
            return;
        };
        let tint = self.terrain_tint(&sprite, pos, biomes);
        for box_ in shapes.outline_boxes(state) {
            let [wx, wy, wz] = [
                (box_.max.x - box_.min.x).min(1.0),
                (box_.max.y - box_.min.y).min(1.0),
                (box_.max.z - box_.min.z).min(1.0),
            ];
            let count = |w: f64| (w / 0.25).ceil().max(2.0) as i32;
            let (cx, cy, cz) = (count(wx), count(wy), count(wz));
            for xx in 0..cx {
                for yy in 0..cy {
                    for zz in 0..cz {
                        let rel = [
                            (xx as f64 + 0.5) / cx as f64,
                            (yy as f64 + 0.5) / cy as f64,
                            (zz as f64 + 0.5) / cz as f64,
                        ];
                        let p = DVec3::new(
                            pos.x as f64 + rel[0] * wx + box_.min.x,
                            pos.y as f64 + rel[1] * wy + box_.min.y,
                            pos.z as f64 + rel[2] * wz + box_.min.z,
                        );
                        let v = Vec3::new(
                            (rel[0] - 0.5) as f32,
                            (rel[1] - 0.5) as f32,
                            (rel[2] - 0.5) as f32,
                        );
                        self.push_terrain(&sprite, p, v, 1.0, None, tint);
                    }
                }
            }
        }
    }

    /// The mining crack particle: vanilla `ClientLevel.addBreakingParticles` — one
    /// `TerrainParticle` per tick on the hit face, at a random point of the state's
    /// outline bounds (0.1 inside, or 0.1 outside the hit face), velocity zeroed and
    /// `setPower(0.2)` applied, quad `scale(0.6)`. Spawned by our own mining start
    /// tick and for the server's `level_event` 2019/2020 (`addBreakingBlockEffects`).
    pub fn spawn_crack(
        &mut self,
        pos: BlockPos,
        face: Direction,
        state: BlockStateId,
        shapes: &dyn CollisionShapes,
        biomes: &dyn crate::tint::BiomeSource,
    ) {
        let Some(sprite) = self
            .types
            .as_deref()
            .and_then(|t| t.blocks.as_ref())
            .and_then(|b| b.sprite(state))
            .filter(|s| s.spawn)
            .copied()
        else {
            return;
        };
        // The outline bounds (vanilla `shape.bounds()`).
        let mut bounds_min = DVec3::new(f64::MAX, f64::MAX, f64::MAX);
        let mut bounds_max = DVec3::new(f64::MIN, f64::MIN, f64::MIN);
        for b in shapes.outline_boxes(state) {
            bounds_min = bounds_min.min(b.min);
            bounds_max = bounds_max.max(b.max);
        }
        let (bx, by, bz) = (bounds_min.x, bounds_min.y, bounds_min.z);
        let size = bounds_max - bounds_min;
        let r = self.rng.next_f64();
        let mut p = DVec3::new(
            pos.x as f64 + r * (size.x - 0.2) + 0.1 + bx,
            pos.y as f64,
            pos.z as f64,
        );
        let r = self.rng.next_f64();
        p.y = pos.y as f64 + r * (size.y - 0.2) + 0.1 + by;
        let r = self.rng.next_f64();
        p.z += r * (size.z - 0.2) + 0.1 + bz;
        // The hit face moves the particle 0.1 outside the bounds on that axis.
        match face {
            Direction::Down => p.y = pos.y as f64 + by - 0.1,
            Direction::Up => p.y = pos.y as f64 + by + size.y + 0.1,
            Direction::North => p.z = pos.z as f64 + bz - 0.1,
            Direction::South => p.z = pos.z as f64 + bz + size.z + 0.1,
            Direction::West => p.x = pos.x as f64 + bx - 0.1,
            Direction::East => p.x = pos.x as f64 + bx + size.x + 0.1,
        }
        let tint = self.terrain_tint(&sprite, pos, biomes);
        self.push_terrain(&sprite, p, Vec3::ZERO, 0.6, Some(0.2), tint);
    }

    /// The sprint particle: vanilla `Entity.spawnSprintParticle` — one block
    /// particle per tick behind a sprinting player's feet, on the block below
    /// (`getOnPosLegacy`, 0.2 below the feet), the position jittered by half the
    /// entity's width (0.6 for a player) and clamped into the support block's
    /// column when the feet left it, the velocity `movement * -4` horizontally and
    /// `1.5` up, through the `TerrainParticle` constructor.
    pub fn spawn_sprint(
        &mut self,
        feet: DVec3,
        velocity: DVec3,
        state: BlockStateId,
        shapes: &dyn CollisionShapes,
        biomes: &dyn crate::tint::BiomeSource,
    ) {
        // `getOnPosLegacy` = `getOnPos(0.2)`: the block column of the feet, 0.2 down.
        let support = BlockPos::new(
            feet.x.floor() as i32,
            (feet.y - 0.2).floor() as i32,
            feet.z.floor() as i32,
        );
        let Some(sprite) = self
            .types
            .as_deref()
            .and_then(|t| t.blocks.as_ref())
            .and_then(|b| b.sprite(state))
            .filter(|s| s.spawn)
            .copied()
        else {
            return;
        };
        let width = 0.6;
        let mut x = feet.x + (self.rng.next_f64() - 0.5) * width;
        let mut z = feet.z + (self.rng.next_f64() - 0.5) * width;
        // Clamp into the support block's column when the feet are in another.
        if (feet.x.floor() as i32) != support.x {
            x = x.clamp(support.x as f64, support.x as f64 + 1.0);
        }
        if (feet.z.floor() as i32) != support.z {
            z = z.clamp(support.z as f64, support.z as f64 + 1.0);
        }
        let pos = DVec3::new(x, feet.y + 0.1, z);
        let speed = Vec3::new((velocity.x * -4.0) as f32, 1.5, (velocity.z * -4.0) as f32);
        let tint = self.terrain_tint(&sprite, support, biomes);
        let _ = shapes;
        self.push_terrain(&sprite, pos, speed, 1.0, None, tint);
    }

    /// The tint of a terrain particle at `pos` (`TerrainParticle`'s colour source).
    /// Without biome colours the plain grey 0.6 still goes through the same
    /// linear-RGB conversion the entity pipeline's vertex colours take.
    pub(super) fn terrain_tint(
        &self,
        sprite: &crate::block_particles::BlockParticleSprite,
        pos: BlockPos,
        biomes: &dyn crate::tint::BiomeSource,
    ) -> [f32; 3] {
        self.biome_colors.as_ref().map_or_else(
            || crate::sky::linear([0.6, 0.6, 0.6]),
            |colors| crate::block_particles::terrain_tint(sprite, pos, biomes, colors),
        )
    }

    /// One `TerrainParticle` (vanilla: gravity 1, friction 0.98, life
    /// `(int)(4/(r*0.9+0.1))`, quad halved — times `quad_scale` — grey 0.6 tint, the
    /// 4x4 uv sub-piece, the speed constructor over `speed`, then optional
    /// `setPower(power)`). `quad_scale` is 1 for break particles, 0.6 for cracks.
    pub(super) fn push_terrain(
        &mut self,
        sprite: &crate::block_particles::BlockParticleSprite,
        pos: DVec3,
        speed: Vec3,
        quad_scale: f32,
        power: Option<f32>,
        tint: [f32; 3],
    ) {
        if self.pool.len() >= MAX_PARTICLES {
            return;
        }
        let rng = &mut self.rng;
        // `Particle`'s speed constructor: the speed inside a ±0.4 jitter per axis,
        // rescaled to `(r+r+1) * 0.15` length and lifted 0.1 on y.
        let mut vel = speed
            + Vec3::new(
                (rng.next_f32() * 2.0 - 1.0) * 0.4,
                (rng.next_f32() * 2.0 - 1.0) * 0.4,
                (rng.next_f32() * 2.0 - 1.0) * 0.4,
            );
        let scale = (rng.next_f32() + rng.next_f32() + 1.0) * 0.15;
        let len = vel.length();
        if len > 0.0 {
            vel *= scale * 0.4 / len;
        }
        vel.y += 0.1;
        // `setPower`: horizontal drag, y re-centred on the 0.1 lift.
        if let Some(power) = power {
            vel.x *= power;
            vel.y = (vel.y - 0.1) * power + 0.1;
            vel.z *= power;
        }
        // `SingleQuadParticle`'s quad, halved by TerrainParticle, times the caller's
        // scale (crack: 0.6).
        let quad = 0.1 * (rng.next_f32() * 0.5 + 0.5) * 2.0 * 0.5 * quad_scale;
        let lifetime = (4.0 / (rng.next_f32() * 0.9 + 0.1)).floor().max(1.0);
        // The uv sub-piece: `(uo..uo+1)/4` of the sprite, uo/vo random 0..3 (vanilla
        // `getU0/getU1/getV0/getV1` interpolate the sprite's texel space).
        let [x, y, w, h] = sprite.rect;
        let (uo, vo) = (rng.next_f32() * 3.0, rng.next_f32() * 3.0);
        let sub = BlockSpriteUv {
            rect: sprite.rect,
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
                source: "TerrainParticle",
                gravity: 1.0,
                ..ParticleBehaviour::DEFAULT
            },
            quad,
            color: tint,
            alpha: 1.0,
            portal_start: pos,
            block_sprite: Some(sub),
        };
        self.pool.push(state);
    }
}
