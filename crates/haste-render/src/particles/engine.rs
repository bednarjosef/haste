//! The particle pool: spawning from `level_particles` and the 20 Hz tick (vanilla
//! `ParticleEngine`, `Particle.tick`).

use std::sync::Arc;

use glam::{DVec3, Vec3};
use haste_core::{BlockPos, Identifier};

use super::particle::Motion;
use super::providers::ParticleBehaviour;
use super::resources::ParticleTypes;
use super::rng::Rng;
use super::*;

/// One live particle (a pool slot). Positions are world-space f32; the renderer subtracts
/// the camera in f64 before narrowing.
pub(super) struct ParticleState {
    /// Index into `ParticleTypes::types`.
    pub(super) kind: u32,
    pub(super) pos: Vec3,
    pub(super) prev_pos: Vec3,
    pub(super) vel: Vec3,
    /// Age and lifetime in ticks.
    pub(super) age: f32,
    pub(super) lifetime: f32,
    /// The packet speed inside a ±0.4 jitter per axis, rescaled to `(r+r+1) * 0.15` and
    /// lifted by 0.1 on y (`Particle`'s speed constructor).
    pub(super) behaviour: ParticleBehaviour,
    /// Spawn-time quad half-width in blocks (vanilla's `quadSize`).
    pub(super) quad: f32,
    /// Spawn-time colour jitter already applied, so the renderer multiplies once.
    pub(super) color: [f32; 3],
    /// Start position for the spiral path (`PortalParticle`).
    pub(super) portal_start: DVec3,
    /// Terrain-particle override: the block sprite (clamping rect) and its 4x4
    /// sub-piece (uv offsets `uo`/`vo` of vanilla `TerrainParticle`), in place of
    /// the type's sprite list.
    pub(super) block_sprite: Option<BlockSpriteUv>,
}

/// A terrain particle's texture: the sprite rect (for the shader's clamp) and the
/// sub-piece's uv corners in atlas texels (`[u0, v0, u1, v1]`).
#[derive(Clone, Copy, Debug)]
pub(super) struct BlockSpriteUv {
    pub(super) rect: [u16; 4],
    pub(super) uv: [f32; 4],
}

/// The particle pool (game state, no GPU): the game thread spawns into it from
/// `level_particles` and ticks it at 20 Hz; the render thread reads it each frame (short
/// lock). Fixed capacity, no allocation per particle or per frame after the pool warms up.
pub struct Particles {
    pub(super) types: Option<Arc<ParticleTypes>>,
    /// Alive particles; expiry swaps the last in, so this stays compact.
    pub(super) pool: Vec<ParticleState>,
    /// Types logged as unknown already (log once per type).
    pub(super) unknown: Vec<Identifier>,
    /// Biome colours for terrain-particle tints (set once at join; `None` leaves
    /// them white, which only headless runs without a pool ever see).
    pub(super) biome_colors: Option<crate::tint::BiomeColors>,
    pub(super) rng: Rng,
}

impl Particles {
    /// An empty pool drawing types from `types`.
    pub fn new(types: Arc<ParticleTypes>) -> Particles {
        Particles {
            types: Some(types),
            pool: Vec::new(),
            unknown: Vec::new(),
            biome_colors: None,
            rng: Rng(0x853C_49E6_748F_EA9B),
        }
    }

    /// The biome colours terrain particles tint with (the joined server's synced
    /// biome registry; `BiomeColors::new` from the same climates the renderer gets).
    pub fn set_biome_colors(&mut self, colors: crate::tint::BiomeColors) {
        self.biome_colors = Some(colors);
    }

    /// Number of live particles.
    pub fn alive(&self) -> usize {
        self.pool.len()
    }

    /// Drops every particle (dimension change).
    pub fn clear(&mut self) {
        self.pool.clear();
    }

    /// Spawns per the packet's semantics (see the module docs). Unknown types are skipped,
    /// logged once per type. `camera` is the eye position the vanilla distance limiter
    /// measures from. `biomes` resolves terrain-particle tints (the world the game thread
    /// holds); `None` leaves the grey base.
    pub fn spawn(
        &mut self,
        s: &ParticleSpawn,
        camera: DVec3,
        biomes: Option<&dyn crate::tint::BiomeSource>,
    ) {
        let Some(types) = self.types.as_deref() else {
            return;
        };
        // A BlockParticleOption replaces the type's sprite and colour (vanilla
        // `TerrainParticle.createTerrainParticle`); it resolves through the block table,
        // not the type's atlas sprites (a `minecraft:block` type has none).
        let terrain = s
            .block_state
            .filter(|_| {
                matches!(
                    s.kind.path(),
                    "block" | "block_marker" | "falling_dust" | "dust_pillar" | "block_crumble"
                )
            })
            .and_then(|state| types.blocks.as_ref().and_then(|b| b.sprite(state)))
            .filter(|sprite| sprite.spawn)
            .copied();
        let kind = if terrain.is_some() {
            None
        } else {
            match types.index(&s.kind) {
                Some(k) => Some(k),
                None => {
                    if !self.unknown.contains(&s.kind) {
                        self.unknown.push(s.kind.clone());
                        tracing::warn!("particle type {} has no atlas sprites; skipped", s.kind);
                    }
                    return;
                }
            }
        };
        // `ClientLevel.doAddParticle`: the limiter drops spawns more than 32 blocks from the
        // camera, unless the packet overrides it (explosions do). Vanilla checks the final
        // (jittered) position of every spawned particle, so this is per particle below.
        let within_limiter =
            |pos: DVec3| s.override_limiter || pos.distance_squared(camera) <= SPAWN_DISTANCE_SQ;
        let behaviour = ParticleBehaviour::of(&s.kind);
        let tint = terrain.map_or([1.0; 3], |sprite| match biomes {
            Some(b) => {
                let f = s.pos.floor().as_ivec3();
                let block = BlockPos::new(f.x, f.y, f.z);
                self.terrain_tint(&sprite, block, b)
            }
            None => [0.6; 3],
        });
        if s.count <= 0 {
            // One particle at the position, velocity = offset * max speed.
            if !within_limiter(s.pos) {
                return;
            }
            let speed = Vec3::from(s.offset) * Vec3::from(s.max_speed);
            match (kind, terrain) {
                (Some(kind), _) => self.push(kind, behaviour, s.pos, speed, s.dust),
                (None, Some(sprite)) => self.push_terrain(&sprite, s.pos, speed, 1.0, None, tint),
                (None, None) => {}
            }
        } else {
            for _ in 0..s.count {
                if self.pool.len() >= MAX_PARTICLES {
                    break;
                }
                let (pos, speed) = match s.randomization {
                    0 => (
                        DVec3::new(
                            s.pos.x + (self.rng.gaussian() * s.offset[0]) as f64,
                            s.pos.y + (self.rng.gaussian() * s.offset[1]) as f64,
                            s.pos.z + (self.rng.gaussian() * s.offset[2]) as f64,
                        ),
                        Vec3::new(
                            self.rng.gaussian() * s.max_speed[0],
                            self.rng.gaussian() * s.max_speed[1],
                            self.rng.gaussian() * s.max_speed[2],
                        ),
                    ),
                    2 => (
                        DVec3::new(
                            s.pos.x + self.rng.next_f64() * s.offset[0] as f64,
                            s.pos.y + self.rng.next_f64() * s.offset[1] as f64,
                            s.pos.z + self.rng.next_f64() * s.offset[2] as f64,
                        ),
                        Vec3::new(
                            s.max_speed[0] * self.rng.next_f32(),
                            s.max_speed[1] * self.rng.next_f32(),
                            s.max_speed[2] * self.rng.next_f32(),
                        ),
                    ),
                    // 1: uniform spread, the packet speed as the velocity.
                    _ => (
                        DVec3::new(
                            s.pos.x + self.rng.next_f64() * s.offset[0] as f64,
                            s.pos.y + self.rng.next_f64() * s.offset[1] as f64,
                            s.pos.z + self.rng.next_f64() * s.offset[2] as f64,
                        ),
                        Vec3::from(s.max_speed),
                    ),
                };
                if within_limiter(pos) {
                    match (kind, terrain) {
                        (Some(kind), _) => self.push(kind, behaviour, pos, speed, s.dust),
                        (None, Some(sprite)) => {
                            self.push_terrain(&sprite, pos, speed, 1.0, None, tint)
                        }
                        (None, None) => {}
                    }
                }
            }
        }
    }

    pub(super) fn push(
        &mut self,
        kind: u32,
        behaviour: &ParticleBehaviour,
        pos: DVec3,
        speed: Vec3,
        dust: Option<DustOption>,
    ) {
        if self.pool.len() >= MAX_PARTICLES {
            return;
        }
        let dust_scale = dust.map_or(1.0, |d| d.scale);
        let mut quad = behaviour
            .quad
            .half_width(self.rng.next_f32(), self.rng.next_f32(), speed.x);
        // `DustParticleBase` scales the lifetime and the quad by the packet's dust scale.
        let mut lifetime = behaviour.lifetime.roll(self.rng.next_f32());
        if behaviour.source == "DustParticleBase" {
            lifetime *= dust_scale;
            quad *= dust_scale;
        }
        let color = spawn_color(behaviour, dust, &mut self.rng);
        // `Particle`'s speed constructor: the packet speed inside a ±0.4 jitter per axis,
        // rescaled to `(r+r+1) * 0.15` and lifted by 0.1 on y.
        let jittered = {
            let mut v = speed
                + Vec3::new(
                    (self.rng.next_f32() * 2.0 - 1.0) * 0.4,
                    (self.rng.next_f32() * 2.0 - 1.0) * 0.4,
                    (self.rng.next_f32() * 2.0 - 1.0) * 0.4,
                );
            let scale = (self.rng.next_f32() + self.rng.next_f32() + 1.0) * 0.15;
            let len = v.length();
            if len > 0.0 {
                v *= scale * 0.4 / len;
            }
            v.y += 0.1;
            v
        };
        // `RisingParticle` jitters the spawn position itself by ±0.05 per axis (flames).
        let pos32 = if let Motion::Rising { spread, .. } = behaviour.motion {
            Vec3::new(
                pos.x as f32 + (self.rng.next_f32() - self.rng.next_f32()) * spread,
                pos.y as f32 + (self.rng.next_f32() - self.rng.next_f32()) * spread,
                pos.z as f32 + (self.rng.next_f32() - self.rng.next_f32()) * spread,
            )
        } else {
            pos.as_vec3()
        };
        let vel = match behaviour.motion {
            Motion::Jitter { keep, add } => jittered * keep + speed * add,
            Motion::Rising { keep, .. } => jittered * keep + speed,
            Motion::Scaled { keep, jitter } => {
                speed * keep
                    + Vec3::new(
                        (self.rng.next_f32() * 2.0 - 1.0) * jitter,
                        (self.rng.next_f32() * 2.0 - 1.0) * jitter,
                        (self.rng.next_f32() * 2.0 - 1.0) * jitter,
                    )
            }
            Motion::Keep => speed,
            Motion::None => Vec3::ZERO,
            Motion::Drop { splash } => {
                if splash && speed.y == 0.0 && (speed.x != 0.0 || speed.z != 0.0) {
                    // SplashParticle: a horizontal packet speed replaces the drop's drift.
                    Vec3::new(speed.x, 0.1, speed.z)
                } else {
                    Vec3::new(
                        speed.x * 0.3,
                        self.rng.next_f32() * 0.2 + 0.1,
                        speed.z * 0.3,
                    )
                }
            }
            Motion::Damped { keep, lift } => {
                Vec3::new(speed.x * keep, speed.y * keep + lift, speed.z * keep)
            }
        };
        let state = ParticleState {
            kind,
            pos: pos32,
            prev_pos: pos32,
            vel,
            age: 0.0,
            lifetime,
            behaviour: *behaviour,
            quad,
            color,
            portal_start: pos,
            block_sprite: None,
        };
        self.pool.push(state);
        if behaviour.tick_at_spawn {
            let last = self.pool.len() - 1;
            self.tick_one(last);
        }
    }

    /// One 20 Hz step: expire, then move. The renderer interpolates between `prev_pos` and
    /// `pos` within the tick. Vanilla's `Particle.tick` order: `yd -= 0.04 * gravity`, move,
    /// then `velocity *= friction`; `BubbleParticle`/`WaterDropParticle`/`SuspendedTownParticle`
    /// override that with their own orders, modelled here by `behaviour`.
    pub fn tick(&mut self) {
        let mut i = 0;
        while i < self.pool.len() {
            if self.tick_one(i) {
                i += 1;
            } else {
                // Swap the last alive particle in and retest this slot.
                let last = self.pool.len() - 1;
                self.pool.swap(i, last);
                self.pool.truncate(last);
            }
        }
    }

    /// Steps one particle; `false` when it expired and was removed (by the caller).
    pub(super) fn tick_one(&mut self, i: usize) -> bool {
        let p = &mut self.pool[i];
        p.age += 1.0;
        if p.age > p.lifetime {
            return false;
        }
        p.prev_pos = p.pos;
        let b = p.behaviour;
        if b.portal_motion {
            // PortalParticle.tick: pos = start + vel * (-p + 2p²) with p = age/lifetime,
            // and y lifted by the remaining fraction; no gravity, no friction.
            let s = p.age / p.lifetime;
            let arc = 1.0 + s - 2.0 * s * s;
            p.pos = p.portal_start.as_vec3() + p.vel * arc;
            p.pos.y += 1.0 - s;
            return true;
        }
        if b.source == "BubbleParticle" {
            // BubbleParticle.tick lifts 0.002 before the move, then drags 0.85; it has no
            // gravity of its own (WaterDropParticle's gravity field reads as 0.04 here).
            p.vel.y += 0.002;
            p.pos += p.vel;
            p.vel *= 0.85;
            return true;
        }
        // WaterDropParticle.tick (rain) and SplashParticle.tick (splash) subtract their own
        // gravity (0.06 resp. 0.04) before the move, then drag; the generic `Particle.tick`
        // subtracts `0.04 * gravity`.
        if b.source == "WaterDropParticle" || b.source == "SplashParticle" {
            p.vel.y -= b.gravity;
            p.pos += p.vel;
            p.vel *= b.friction;
            return true;
        }
        p.vel.y -= 0.04 * b.gravity;
        p.pos += p.vel;
        if b.speed_up_when_blocked && p.pos.y == p.prev_pos.y {
            // Particle.tick: a blocked y move speeds the horizontal drift up.
            p.vel.x *= 1.1;
            p.vel.z *= 1.1;
        }
        p.vel *= b.friction;
        // Per-tick colour fade (`CritParticle.tick`'s yellowing).
        for k in 0..3 {
            p.color[k] *= b.color_fade[k];
        }
        true
    }
}

/// The spawn colour: the behaviour's base, its spawn-time jitter (`BaseAshSmokeParticle`'s
/// grey, `CritParticle`'s bright grey), the `dust` option colour, and finally any scale
/// (`enchanted_hit`).
pub(super) fn spawn_color(
    behaviour: &ParticleBehaviour,
    dust: Option<DustOption>,
    rng: &mut Rng,
) -> [f32; 3] {
    let mut color = behaviour.color;
    if let Some((low, high)) = behaviour.color_jitter {
        let jitter = low + rng.next_f32() * (high - low);
        color = [color[0] * jitter, color[1] * jitter, color[2] * jitter];
    }
    if behaviour.source == "DustParticleBase" {
        // DustParticle randomises the option colour by (0.8 + 0.2r) per channel, times a
        // shared 0.6 + 0.4r brightness.
        let base = dust.map_or([1.0; 3], |d| d.color);
        let factor = rng.next_f32() * 0.4 + 0.6;
        color = [
            (rng.next_f32() * 0.2 + 0.8) * base[0] * factor,
            (rng.next_f32() * 0.2 + 0.8) * base[1] * factor,
            (rng.next_f32() * 0.2 + 0.8) * base[2] * factor,
        ];
    }
    if let Some(scale) = behaviour.color_scale {
        color = [
            color[0] * scale[0],
            color[1] * scale[1],
            color[2] * scale[2],
        ];
    }
    color
}
