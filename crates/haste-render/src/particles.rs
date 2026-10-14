//! Server-sent particles (`level_particles`), ported from vanilla 26.3's particle classes
//! (`net.minecraft.client.particle`): a fixed-capacity pool simulated at 20 Hz on the game
//! thread ([`Particles`]), drawn as camera-facing quads textured from the particle atlas in
//! one batched draw inside the main pass ([`ParticleRenderer`], reusing the entity shader:
//! sprite rects by textureLoad, vertex light colour, fog).
//!
//! Per-type behaviour lives in the [`ParticleBehaviour`] table: gravity, friction, lifetime,
//! quad size and its growth over the particle's life, colour, alpha and spawn-time motion.
//! Each entry names the vanilla class it follows (`FlameParticle` for `flame`,
//! `BaseAshSmokeParticle` subclass `SmokeParticle` for `smoke`, ...). Registry ids without
//! an entry use vanilla's generic `Particle` defaults, which is also what mods and vanilla
//! types we have not ported get.
//!
//! `level_particles` semantics follow `ClientPacketListener.handleParticleEvent`:
//! count 0 spawns one particle at the position with velocity `offset * max_speed`; count > 0
//! spawns that many at `pos + dist * gaussian` with velocity `gaussian * max_speed` (or
//! uniform, when the packet's randomization mode says so). `ClientLevel.doAddParticle`
//! drops spawns beyond `sqrt(1024.0)` = 32 blocks of the camera unless the packet overrides
//! the limiter.
//!
//! Not modelled (each a separate later step): block collisions (`Particle.move` with
//! `hasPhysics`, so no `onGround` drag and no `speedUpWhenYMotionIsBlocked` trigger beyond
//! an exactly zero y step), block/item particles (they draw from the block and item atlases
//! in vanilla), particle options other than `dust` (block states, trail colors, geyser),
//! the 16384-particle cap with the distance squash (`ParticleGroup`), roll (billboards keep
//! facing the camera), and the per-type light boosts `FlameParticle`/`PortalParticle` add.

use std::sync::Arc;

use anyhow::Result;
use glam::{DVec3, Vec3};
use haste_assets::{Assets, ParticleAtlas};
use haste_core::view::BlockView;
use haste_core::{BlockPos, Identifier};

use crate::entity::EntityVertex;
use crate::renderer::Camera;

/// Pool capacity (vanilla caps its engine around 16384 with a distance squash we do not
/// model; extra spawns are dropped).
pub const MAX_PARTICLES: usize = 4096;

/// Sky-light value meaning "not sampled yet" (resampled on the first frame).
const UNSAMPLED: u8 = 255;

/// How far from the camera `level_particles` spawns survive (`ClientLevel.doAddParticle`:
/// `distanceToSqr(x, y, z) > 1024.0`).
const SPAWN_DISTANCE_SQ: f64 = 1024.0;

/// A particle spawn request from `level_particles`: the resolved type name and the packet's
/// spawn parameters. The game thread queues these, the shared pool consumes them.
#[derive(Clone, Debug)]
pub struct ParticleSpawn {
    /// Particle type (`minecraft:flame`).
    pub kind: Identifier,
    /// Spawn position.
    pub pos: DVec3,
    /// Offset per axis (spread box for count > 0, direction for count 0).
    pub offset: [f32; 3],
    /// Max speed per axis.
    pub max_speed: [f32; 3],
    /// Count (0: one particle at `pos` with velocity `offset * max_speed`).
    pub count: i32,
    /// Packet randomization mode (0 gaussian, 1 uniform, 2 uniform with speed).
    pub randomization: u32,
    /// The packet overrides `ClientLevel`'s distance limiter.
    pub override_limiter: bool,
    /// `minecraft:dust` option data: ARGB colour and size scale (vanilla's
    /// `DustParticleOptions`), the one particle option whose payload we read.
    pub dust: Option<DustOption>,
}

/// `DustParticleOptions`: the colour `DustParticle` tints its quad with, and the size scale
/// (`DustParticleBase` multiplies `quadSize` and `lifetime` with it).
#[derive(Clone, Copy, Debug)]
pub struct DustOption {
    /// Linear RGB colour (vanilla unpacks the packet's ARGB int, alpha ignored).
    pub color: [f32; 3],
    /// Size scale (`0.2..=4.0` in vanilla's codec, 1.0 in practice).
    pub scale: f32,
}

/// Per-type behaviour ported from vanilla's particle classes. Every entry names the class it
/// follows, so the numbers are traceable back to vanilla (never invented here).
///
/// Vanilla's generic base (`Particle`) is `DEFAULT`: no gravity, `friction` 0.98, a
/// `(int)(4.0 / (r * 0.9 + 0.1))` lifetime (4..=40 ticks), a `0.1 * (r * 0.5 + 0.5) * 2.0`
/// quad (0.1..=0.2 blocks half-width, 0.2..=0.4 across), white, and the packet's speed
/// inside a random direction.
#[derive(Clone, Copy, Debug)]
struct ParticleBehaviour {
    /// The vanilla class this entry follows (`Particle` for the generic default).
    source: &'static str,
    /// Gravity in blocks/tick²; vanilla's `Particle.tick` does `yd -= 0.04 * gravity`.
    gravity: f32,
    /// Velocity retained per tick (`Particle.friction`).
    friction: f32,
    /// Lifetime in ticks.
    lifetime: Lifetime,
    /// Quad size at spawn (vanilla's `quadSize`).
    quad: Quad,
    /// How the quad size changes over the particle's life (`getQuadSize` overrides).
    growth: QuadGrowth,
    /// Base colour (vanilla's `rCol`/`gCol`/`bCol`, 1.0 white when unset).
    color: [f32; 3],
    /// Spawn-time colour jitter: `rCol = randomize(low..high, color)` when `Some`, as
    /// `BaseAshSmokeParticle`'s grey and `CritParticle`'s bright grey do.
    color_jitter: Option<(f32, f32)>,
    /// Multiplier on the start colour, as `CritParticle.MagicProvider`'s
    /// `rCol *= 0.3; gCol *= 0.8` for `enchanted_hit`.
    color_scale: Option<[f32; 3]>,
    /// Alpha (vanilla's `alpha`, 1.0 when unset).
    alpha: Option<f32>,
    /// Per-tick colour multiplier (`CritParticle.tick`: `gCol *= 0.96; bCol *= 0.9`).
    color_fade: [f32; 3],
    /// Velocity handling at spawn (see [`Motion`]).
    motion: Motion,
    /// `Particle.speedUpWhenYMotionIsBlocked` (smoke, heart, dust, note: `xd/z *= 1.1` when a
    /// vertical move is blocked).
    speed_up_when_blocked: bool,
    /// `CritParticle` runs one tick in its constructor.
    tick_at_spawn: bool,
    /// Full light (`HugeExplosionParticle.getLightCoords` returns 15728880).
    full_light: bool,
    /// `PortalParticle`'s spiral path instead of the generic integration.
    portal_motion: bool,
}

impl ParticleBehaviour {
    /// Vanilla's generic `Particle`.
    const DEFAULT: ParticleBehaviour = ParticleBehaviour {
        source: "Particle",
        gravity: 0.0,
        friction: 0.98,
        lifetime: Lifetime::Rolled {
            base: 4.0,
            low: 0.1,
            scale: 1.0,
            add: 0.0,
            min: 4.0,
        },
        quad: Quad::Default {
            scale: 1.0,
            lo: 1.0,
            hi: 1.0,
        },
        growth: QuadGrowth::Constant,
        color: [1.0, 1.0, 1.0],
        color_jitter: None,
        color_scale: None,
        alpha: Some(1.0),
        color_fade: [1.0, 1.0, 1.0],
        motion: Motion::Jitter {
            keep: 1.0,
            add: 0.0,
        },
        speed_up_when_blocked: false,
        tick_at_spawn: false,
        full_light: false,
        portal_motion: false,
    };

    /// Behaviour of a registry id, `DEFAULT` for ids without an entry (mods and vanilla
    /// types we have not ported).
    /// Behaviour of a registry id, `DEFAULT` for ids without an entry (mods and vanilla
    /// types we have not ported).
    fn of(id: &Identifier) -> &'static ParticleBehaviour {
        let name: &str = id.path();
        match name {
            // FlameParticle (RisingParticle): friction 0.96, no gravity, life
            // (int)(8/(r*0.8+0.2)) + 4 = 12..=44, quad shrinking as 1 - s²/2, the spawn
            // position jittered by ±0.05 and the velocity kept at 1 % of the packet's.
            "flame" | "copper_fire_flame" | "soul_fire_flame" => &ParticleBehaviour {
                source: "FlameParticle",
                friction: 0.96,
                lifetime: Lifetime::Rolled {
                    base: 8.0,
                    low: 0.2,
                    scale: 1.0,
                    add: 4.0,
                    min: 12.0,
                },
                growth: QuadGrowth::ShrinkLate,
                motion: Motion::Rising {
                    keep: 0.01,
                    spread: 0.05,
                },
                ..ParticleBehaviour::DEFAULT
            },
            // small_flame is FlameParticle.SmallFlameProvider: the same class, quad halved.
            "small_flame" => &ParticleBehaviour {
                source: "FlameParticle.SmallFlameProvider",
                friction: 0.96,
                lifetime: Lifetime::Rolled {
                    base: 8.0,
                    low: 0.2,
                    scale: 1.0,
                    add: 4.0,
                    min: 12.0,
                },
                quad: Quad::Default {
                    scale: 0.5,
                    lo: 1.0,
                    hi: 1.0,
                },
                growth: QuadGrowth::ShrinkLate,
                motion: Motion::Rising {
                    keep: 0.01,
                    spread: 0.05,
                },
                ..ParticleBehaviour::DEFAULT
            },
            // SmokeParticle (BaseAshSmokeParticle, scale 1): friction 0.96, gravity -0.1,
            // life (int)(8/(r*0.8+0.2)) = 8..=40, quad 0.75x growing to full size, grey
            // colour jitter, speedUpWhenYMotionIsBlocked.
            "smoke" => &ParticleBehaviour {
                source: "SmokeParticle",
                gravity: -0.1,
                friction: 0.96,
                lifetime: Lifetime::Rolled {
                    base: 8.0,
                    low: 0.2,
                    scale: 1.0,
                    add: 0.0,
                    min: 1.0,
                },
                quad: Quad::Default {
                    scale: 0.75,
                    lo: 1.0,
                    hi: 1.0,
                },
                growth: QuadGrowth::GrowEarly,
                color_jitter: Some((0.0, 0.3)),
                motion: Motion::Jitter {
                    keep: 0.1,
                    add: 0.0,
                },
                speed_up_when_blocked: true,
                ..ParticleBehaviour::DEFAULT
            },
            // LargeSmokeParticle is SmokeParticle with scale 2.5 (quad 1.875x, life *2.5).
            "large_smoke" => &ParticleBehaviour {
                source: "LargeSmokeParticle",
                gravity: -0.1,
                friction: 0.96,
                lifetime: Lifetime::Rolled {
                    base: 8.0,
                    low: 0.2,
                    scale: 2.5,
                    add: 0.0,
                    min: 1.0,
                },
                quad: Quad::Default {
                    scale: 1.875,
                    lo: 1.0,
                    hi: 1.0,
                },
                growth: QuadGrowth::GrowEarly,
                color_jitter: Some((0.0, 0.3)),
                motion: Motion::Jitter {
                    keep: 0.1,
                    add: 0.0,
                },
                speed_up_when_blocked: true,
                ..ParticleBehaviour::DEFAULT
            },
            // PlayerCloudParticle (cloud): friction 0.96, life (int)(8/(r*0.8+0.3))*2.5
            // = 17..=65, quad 1.875x, grey-white jitter, no collision.
            "cloud" => &ParticleBehaviour {
                source: "PlayerCloudParticle",
                friction: 0.96,
                lifetime: Lifetime::Rolled {
                    base: 8.0,
                    low: 0.3,
                    scale: 2.5,
                    add: 0.0,
                    min: 1.0,
                },
                quad: Quad::Default {
                    scale: 1.875,
                    lo: 1.0,
                    hi: 1.0,
                },
                growth: QuadGrowth::GrowEarly,
                color_jitter: Some((0.7, 1.0)),
                motion: Motion::Jitter {
                    keep: 0.1,
                    add: 0.0,
                },
                ..ParticleBehaviour::DEFAULT
            },
            // ExplodeParticle (poof): gravity -0.1, friction 0.9, life 16/(r*0.8+0.2)+2 =
            // 18..=82, quad 0.1*(r*r*6+1), bright grey.
            "poof" => &ParticleBehaviour {
                source: "ExplodeParticle",
                gravity: -0.1,
                friction: 0.9,
                lifetime: Lifetime::Rolled {
                    base: 16.0,
                    low: 0.2,
                    scale: 1.0,
                    add: 2.0,
                    min: 18.0,
                },
                quad: Quad::Formula {
                    size: 0.1,
                    lo: 1.0,
                    hi: 6.0,
                },
                color_jitter: Some((0.7, 1.0)),
                motion: Motion::Scaled {
                    keep: 1.0,
                    jitter: 0.05,
                },
                ..ParticleBehaviour::DEFAULT
            },
            // HeartParticle: friction 0.86, no gravity, life 16, quad 1.5x, the jittered
            // velocity damped to 1 % with +0.1 on y, no collision.
            "heart" => &ParticleBehaviour {
                source: "HeartParticle",
                friction: 0.86,
                lifetime: Lifetime::Fixed(16),
                quad: Quad::Default {
                    scale: 1.5,
                    lo: 1.0,
                    hi: 1.0,
                },
                growth: QuadGrowth::GrowEarly,
                motion: Motion::Damped {
                    keep: 0.01,
                    lift: 0.1,
                },
                speed_up_when_blocked: true,
                ..ParticleBehaviour::DEFAULT
            },
            // CritParticle: gravity 0.5, friction 0.7, life max(6/(r*0.8+0.6), 1) = 4..=10,
            // quad 0.75x, bright grey that yellows with age, no collision, one constructor
            // tick.
            "crit" | "damage_indicator" => &ParticleBehaviour {
                source: "CritParticle",
                gravity: 0.5,
                friction: 0.7,
                lifetime: Lifetime::Rolled {
                    base: 6.0,
                    low: 0.6,
                    scale: 1.0,
                    add: 0.0,
                    min: 1.0,
                },
                quad: Quad::Default {
                    scale: 0.75,
                    lo: 1.0,
                    hi: 1.0,
                },
                growth: QuadGrowth::GrowEarly,
                color_jitter: Some((0.6, 0.9)),
                motion: Motion::Jitter {
                    keep: 0.1,
                    add: 0.4,
                },
                tick_at_spawn: true,
                color_fade: [1.0, 0.96, 0.9],
                ..ParticleBehaviour::DEFAULT
            },
            // enchanted_hit is CritParticle.MagicProvider (rCol 0.3x, gCol 0.8x).
            "enchanted_hit" => &ParticleBehaviour {
                source: "CritParticle.MagicProvider",
                gravity: 0.5,
                friction: 0.7,
                lifetime: Lifetime::Rolled {
                    base: 6.0,
                    low: 0.6,
                    scale: 1.0,
                    add: 0.0,
                    min: 1.0,
                },
                quad: Quad::Default {
                    scale: 0.75,
                    lo: 1.0,
                    hi: 1.0,
                },
                growth: QuadGrowth::GrowEarly,
                color_jitter: Some((0.6, 0.9)),
                color_scale: Some([0.3, 0.8, 1.0]),
                motion: Motion::Jitter {
                    keep: 0.1,
                    add: 0.4,
                },
                tick_at_spawn: true,
                color_fade: [1.0, 0.96, 0.9],
                ..ParticleBehaviour::DEFAULT
            },
            // SuspendedTownParticle (happy_villager): friction 0.99, life 20/(r*0.8+0.2) =
            // 20..=100, the jittered velocity kept at 2 %, white.
            "happy_villager" => &ParticleBehaviour {
                source: "SuspendedTownParticle",
                friction: 0.99,
                lifetime: Lifetime::Rolled {
                    base: 20.0,
                    low: 0.2,
                    scale: 1.0,
                    add: 0.0,
                    min: 1.0,
                },
                motion: Motion::Jitter {
                    keep: 0.02,
                    add: 0.0,
                },
                ..ParticleBehaviour::DEFAULT
            },
            // BubbleParticle: life 8/(r*0.8+0.2) = 8..=40, velocity 0.2x plus a ±0.02
            // jitter, quad scaled by r*0.6+0.2, rising 0.002/tick with friction 0.85.
            "bubble" => &ParticleBehaviour {
                source: "BubbleParticle",
                friction: 0.85,
                lifetime: Lifetime::Rolled {
                    base: 8.0,
                    low: 0.2,
                    scale: 1.0,
                    add: 0.0,
                    min: 1.0,
                },
                quad: Quad::Default {
                    scale: 1.0,
                    lo: 0.2,
                    hi: 0.8,
                },
                growth: QuadGrowth::GrowEarly,
                motion: Motion::Scaled {
                    keep: 0.2,
                    jitter: 0.02,
                },
                ..ParticleBehaviour::DEFAULT
            },
            // SplashParticle (WaterDropParticle): gravity 0.04, friction 0.98, life
            // 8/(r*0.8+0.2), y speed 0.1 plus the packet's x/z when only y is free.
            "splash" => &ParticleBehaviour {
                source: "SplashParticle",
                gravity: 0.04,
                lifetime: Lifetime::Rolled {
                    base: 8.0,
                    low: 0.2,
                    scale: 1.0,
                    add: 0.0,
                    min: 1.0,
                },
                quad: Quad::Default {
                    scale: 1.0,
                    lo: 0.2,
                    hi: 0.8,
                },
                motion: Motion::Drop { splash: true },
                ..ParticleBehaviour::DEFAULT
            },
            // WaterDropParticle (rain): gravity 0.06, friction 0.98, life 8/(r*0.8+0.2),
            // y speed 0.1..0.3, x/z speed 0.3x.
            "rain" => &ParticleBehaviour {
                source: "WaterDropParticle",
                gravity: 0.06,
                lifetime: Lifetime::Rolled {
                    base: 8.0,
                    low: 0.2,
                    scale: 1.0,
                    add: 0.0,
                    min: 1.0,
                },
                motion: Motion::Drop { splash: false },
                ..ParticleBehaviour::DEFAULT
            },
            // DustParticleBase (dust): friction 0.96, life 8/(r*0.8+0.2)*scale, quad
            // 0.75*scale, the option colour randomised by (0.8 + 0.2r) * (0.6 + 0.4r),
            // speedUpWhenYMotionIsBlocked.
            "dust" => &ParticleBehaviour {
                source: "DustParticleBase",
                friction: 0.96,
                lifetime: Lifetime::Rolled {
                    base: 8.0,
                    low: 0.2,
                    scale: 1.0,
                    add: 0.0,
                    min: 1.0,
                },
                quad: Quad::Default {
                    scale: 0.75,
                    lo: 1.0,
                    hi: 1.0,
                },
                growth: QuadGrowth::GrowEarly,
                color_jitter: Some((0.6, 1.0)),
                motion: Motion::Jitter {
                    keep: 0.1,
                    add: 0.0,
                },
                speed_up_when_blocked: true,
                ..ParticleBehaviour::DEFAULT
            },
            // TerrainParticle (block): gravity 1, quad halved, grey 0.6 base (block tint
            // not modelled: block particles draw from the block atlas in vanilla).
            "block" | "block_marker" | "falling_dust" | "dust_pillar" | "block_crumble" => {
                &ParticleBehaviour {
                    source: "TerrainParticle",
                    gravity: 1.0,
                    quad: Quad::Default {
                        scale: 0.5,
                        lo: 1.0,
                        hi: 1.0,
                    },
                    color: [0.6, 0.6, 0.6],
                    motion: Motion::Jitter {
                        keep: 1.0,
                        add: 0.0,
                    },
                    ..ParticleBehaviour::DEFAULT
                }
            }
            // BreakingItemParticle (item): gravity 1, quad halved, velocity 0.1x plus the
            // packet speed (ItemParticle draws from the item atlas in vanilla).
            "item" | "item_slime" | "item_cobweb" | "item_snowball" => &ParticleBehaviour {
                source: "BreakingItemParticle",
                gravity: 1.0,
                quad: Quad::Default {
                    scale: 0.5,
                    lo: 1.0,
                    hi: 1.0,
                },
                motion: Motion::Scaled {
                    keep: 0.1,
                    jitter: 0.0,
                },
                ..ParticleBehaviour::DEFAULT
            },
            // HugeExplosionParticle (explosion): life 6 + r(0..4), quad 2*(1 - size/2) with
            // size from the packet's x speed, grey 0.4..1.0, full light, no motion.
            "explosion" => &ParticleBehaviour {
                source: "HugeExplosionParticle",
                friction: 1.0,
                lifetime: Lifetime::Rand { base: 6, max: 4 },
                quad: Quad::Explosion,
                color_jitter: Some((0.4, 1.0)),
                motion: Motion::None,
                full_light: true,
                ..ParticleBehaviour::DEFAULT
            },
            // NoteParticle: friction 0.66, life 6, quad 1.5x, the jittered velocity damped
            // to 1 % with +0.2 on y, speedUpWhenYMotionIsBlocked.
            "note" => &ParticleBehaviour {
                source: "NoteParticle",
                friction: 0.66,
                lifetime: Lifetime::Fixed(6),
                quad: Quad::Default {
                    scale: 1.5,
                    lo: 1.0,
                    hi: 1.0,
                },
                growth: QuadGrowth::GrowEarly,
                motion: Motion::Damped {
                    keep: 0.01,
                    lift: 0.2,
                },
                speed_up_when_blocked: true,
                ..ParticleBehaviour::DEFAULT
            },
            // PortalParticle: life r*10 + 40, quad 0.1*(r*0.2+0.5) growing as 1-(1-s)²,
            // purple (br*0.9, br*0.3, br), the spiral path in its own tick.
            "portal" => &ParticleBehaviour {
                source: "PortalParticle",
                lifetime: Lifetime::Rand { base: 40, max: 10 },
                quad: Quad::Formula {
                    size: 0.1,
                    lo: 0.5,
                    hi: 0.2,
                },
                growth: QuadGrowth::Portal,
                color: [0.9, 0.3, 1.0],
                color_jitter: Some((0.4, 1.0)),
                motion: Motion::Keep,
                portal_motion: true,
                ..ParticleBehaviour::DEFAULT
            },
            _ => &ParticleBehaviour::DEFAULT,
        }
    }
}

/// Lifetime in ticks, as the vanilla classes compute it.
#[derive(Clone, Copy, Debug)]
enum Lifetime {
    /// `(int)(base / (r * (1.0 - low) + low)) * scale + add`, at least `min` ticks: the
    /// `(int)(base / (r * jitter + low))` form every vanilla particle class uses.
    Rolled {
        base: f32,
        low: f32,
        scale: f32,
        add: f32,
        min: f32,
    },
    /// A fixed number of ticks.
    Fixed(u32),
    /// `(int)(r * max) + base` (`PortalParticle`, `HugeExplosionParticle`).
    Rand { base: u32, max: u32 },
}

impl Lifetime {
    /// Rolls this lifetime with `r`, never below one tick.
    fn roll(self, r: f32) -> f32 {
        match self {
            Lifetime::Rolled {
                base,
                low,
                scale,
                add,
                min,
            } => {
                let rolled = (base / (r * (1.0 - low) + low)).floor();
                (rolled * scale + add).max(min).max(1.0)
            }
            Lifetime::Fixed(ticks) => ticks as f32,
            Lifetime::Rand { base, max } => (base + (r * max as f32) as u32) as f32,
        }
    }
}

/// Quad size at spawn (vanilla's `quadSize`), as the half-width in blocks.
#[derive(Clone, Copy, Debug)]
enum Quad {
    /// `SingleQuadParticle`'s default `0.1 * (r * 0.5 + 0.5) * 2.0` (0.1..=0.2 half-width),
    /// times `scale`, re-randomised by a `lo + r * (hi - lo)` factor (`BubbleParticle`'s
    /// `r * 0.6 + 0.2`, where the default size itself is also random).
    Default { scale: f32, lo: f32, hi: f32 },
    /// `size * (r * hi + lo)`, where a class assigns `quadSize` outright (`PortalParticle`'s
    /// `0.1 * (r * 0.2 + 0.5)`).
    Formula { size: f32, lo: f32, hi: f32 },
    /// `HugeExplosionParticle`: `2.0 * (1.0 - size * 0.5)` with `size` from the packet.
    Explosion,
}

impl Quad {
    /// The quad's half-width in blocks. `r` is the default's random, `r2` the entry's
    /// factor, `speed_x` the packet's x max speed (the explosion size carrier).
    fn half_width(self, r: f32, r2: f32, speed_x: f32) -> f32 {
        match self {
            Quad::Default { scale, lo, hi } => {
                0.1 * (r * 0.5 + 0.5) * 2.0 * scale * (lo + r2 * (hi - lo))
            }
            Quad::Formula { size, lo, hi } => size * (r * hi + lo),
            Quad::Explosion => 2.0 * (1.0 - speed_x * 0.5),
        }
    }
}

/// How the quad size changes over the particle's life (`getQuadSize` overrides).
#[derive(Clone, Copy, Debug)]
enum QuadGrowth {
    /// Unchanged (`SingleQuadParticle.getQuadSize`).
    Constant,
    /// `clamp((age + a) / lifetime * 32, 0, 1)`: full size after 1/32 of the life
    /// (`BaseAshSmokeParticle`, `CritParticle`, `HeartParticle`, `NoteParticle`,
    /// `DustParticleBase`, `PlayerCloudParticle`).
    GrowEarly,
    /// `FlameParticle`: `quadSize * (1 - s²/2)` with `s = (age + a) / lifetime`.
    ShrinkLate,
    /// `PortalParticle`: `quadSize * (1 - (1 - s)²)` with `s = age / lifetime`.
    Portal,
}

impl QuadGrowth {
    /// The multiplier for a normalized age `s` in 0..=1.
    fn scale(self, s: f32) -> f32 {
        match self {
            QuadGrowth::Constant => 1.0,
            QuadGrowth::GrowEarly => (s * 32.0).clamp(0.0, 1.0),
            QuadGrowth::ShrinkLate => 1.0 - s * s * 0.5,
            QuadGrowth::Portal => 1.0 - (1.0 - s) * (1.0 - s),
        }
    }
}

/// Spawn-time velocity handling, as the vanilla classes' constructors do it. `speed` is the
/// packet's velocity (`offset * max_speed` for count 0, `gaussian * max_speed` for count > 0)
/// and `jittered` is `Particle`'s speed constructor: `speed` plus a ±0.4 random offset per
/// axis, rescaled to `(r+r+1) * 0.15` and lifted by 0.1 on y.
#[derive(Clone, Copy, Debug)]
enum Motion {
    /// `jittered`, then the packet speed scaled by `keep` and added back (`SmokeParticle`
    /// 0.1, `CritParticle` 0.1 and its `xa * 0.4`, `SuspendedTownParticle` 0.02,
    /// `DustParticleBase` 0.1, `PlayerCloudParticle` 0.1, `TerrainParticle` 1.0).
    Jitter { keep: f32, add: f32 },
    /// `RisingParticle` (flames): `jittered * keep + speed`, and the spawn position jittered
    /// by ±`spread` per axis.
    Rising { keep: f32, spread: f32 },
    /// The packet speed alone, plus a ±`jitter` random offset per axis (`BubbleParticle`
    /// 0.02, `ExplodeParticle` 0.05, `BreakingItemParticle` 0).
    Scaled { keep: f32, jitter: f32 },
    /// The packet speed verbatim (`PortalParticle`).
    Keep,
    /// No velocity at all (`HugeExplosionParticle`).
    None,
    /// `WaterDropParticle` (rain): x/z speed 0.3x, y speed `r * 0.2 + 0.1`. `SplashParticle`
    /// (`splash` set) replaces that with the packet's x/z speed and a fixed 0.1 on y when it
    /// has no y speed of its own.
    Drop { splash: bool },
    /// The jittered velocity damped to `keep`, with a fixed `lift` on y: `HeartParticle`
    /// (0.01, +0.1) and `NoteParticle` (0.01, +0.2).
    Damped { keep: f32, lift: f32 },
}

/// The sprite list of one particle type: rectangles in the particle atlas, in file order.
struct ParticleType {
    name: Identifier,
    sprites: Vec<[u16; 4]>,
}

/// Every particle type that has a particle json (name -> atlas sprite rectangles), plus the
/// packed atlas pixels. Built once from the registries' particle type ids; the game thread
/// resolves spawn names against it, the renderer uploads its pixels as one texture.
pub struct ParticleTypes {
    types: Vec<ParticleType>,
    by_name: ahash::HashMap<Identifier, u32>,
    pixels: Vec<u8>,
    size: u32,
}

impl ParticleTypes {
    /// Packs the sprites of `ids` into one atlas (types without a definition or with no
    /// placed sprites contribute nothing: block/item particles are drawn from block/item
    /// textures in vanilla, none of that here).
    pub fn build(assets: &mut Assets, ids: &[Identifier]) -> Result<ParticleTypes> {
        let atlas = ParticleAtlas::build(assets, ids)?;
        let mut types = Vec::new();
        for id in ids {
            let sprites: Vec<[u16; 4]> = atlas
                .sprites_of(id)
                .iter()
                .map(|r| r.pixel_rect.map(|v| v as u16))
                .collect();
            if !sprites.is_empty() {
                types.push(ParticleType {
                    name: id.clone(),
                    sprites,
                });
            }
        }
        let by_name = types
            .iter()
            .enumerate()
            .map(|(i, t)| (t.name.clone(), i as u32))
            .collect();
        Ok(ParticleTypes {
            types,
            by_name,
            pixels: atlas.pixels().to_vec(),
            size: atlas.size(),
        })
    }

    /// Index of a particle type, `None` when it has no atlas sprites.
    fn index(&self, name: &Identifier) -> Option<u32> {
        self.by_name.get(name).copied()
    }

    fn sprites(&self, i: u32) -> &[[u16; 4]] {
        &self.types[i as usize].sprites
    }

    /// The packed atlas RGBA pixels and its side length (for the texture upload).
    pub fn atlas(&self) -> (&[u8], u32) {
        (&self.pixels, self.size)
    }
}

/// One live particle (a pool slot). Positions are world-space f32; the renderer subtracts
/// the camera in f64 before narrowing.
struct ParticleState {
    /// Index into `ParticleTypes::types`.
    kind: u32,
    pos: Vec3,
    prev_pos: Vec3,
    vel: Vec3,
    /// Age and lifetime in ticks.
    age: f32,
    lifetime: f32,
    /// The packet speed inside a ±0.4 jitter per axis, rescaled to `(r+r+1) * 0.15` and
    /// lifted by 0.1 on y (`Particle`'s speed constructor).
    behaviour: ParticleBehaviour,
    /// Spawn-time quad half-width in blocks (vanilla's `quadSize`).
    quad: f32,
    /// Spawn-time colour jitter already applied, so the renderer multiplies once.
    color: [f32; 3],
    /// Start position for the spiral path (`PortalParticle`).
    portal_start: DVec3,
    /// Block whose light is cached (resampled when the particle leaves it).
    light_at: BlockPos,
    sky_light: u8,
    block_light: u8,
}

/// The particle pool (game state, no GPU): the game thread spawns into it from
/// `level_particles` and ticks it at 20 Hz; the render thread reads it each frame (short
/// lock). Fixed capacity, no allocation per particle or per frame after the pool warms up.
pub struct Particles {
    types: Option<Arc<ParticleTypes>>,
    /// Alive particles; expiry swaps the last in, so this stays compact.
    pool: Vec<ParticleState>,
    /// Types logged as unknown already (log once per type).
    unknown: Vec<Identifier>,
    rng: Rng,
}

impl Particles {
    /// An empty pool drawing types from `types`.
    pub fn new(types: Arc<ParticleTypes>) -> Particles {
        Particles {
            types: Some(types),
            pool: Vec::new(),
            unknown: Vec::new(),
            rng: Rng(0x853C_49E6_748F_EA9B),
        }
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
    /// measures from.
    pub fn spawn(&mut self, s: &ParticleSpawn, camera: DVec3) {
        let Some(types) = self.types.as_deref() else {
            return;
        };
        let Some(kind) = types.index(&s.kind) else {
            if !self.unknown.contains(&s.kind) {
                self.unknown.push(s.kind.clone());
                tracing::warn!("particle type {} has no atlas sprites; skipped", s.kind);
            }
            return;
        };
        // `ClientLevel.doAddParticle`: the limiter drops spawns more than 32 blocks from the
        // camera, unless the packet overrides it (explosions do). Vanilla checks the final
        // (jittered) position of every spawned particle, so this is per particle below.
        let within_limiter =
            |pos: DVec3| s.override_limiter || pos.distance_squared(camera) <= SPAWN_DISTANCE_SQ;
        let behaviour = ParticleBehaviour::of(&s.kind);
        if s.count <= 0 {
            // One particle at the position, velocity = offset * max speed.
            if !within_limiter(s.pos) {
                return;
            }
            let speed = Vec3::from(s.offset) * Vec3::from(s.max_speed);
            self.push(kind, behaviour, s.pos, speed, s.dust);
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
                    self.push(kind, behaviour, pos, speed, s.dust);
                }
            }
        }
    }

    fn push(
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
            light_at: BlockPos::new(
                pos.x.floor() as i32,
                pos.y.floor() as i32,
                pos.z.floor() as i32,
            ),
            sky_light: UNSAMPLED,
            block_light: 0,
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
    fn tick_one(&mut self, i: usize) -> bool {
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
fn spawn_color(behaviour: &ParticleBehaviour, dust: Option<DustOption>, rng: &mut Rng) -> [f32; 3] {
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

/// xorshift64*, deterministically seeded (tests rely on the repetition).
struct Rng(u64);

impl Rng {
    /// Uniform f32 in [0, 1).
    fn next_f32(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / 16_777_216.0
    }

    /// Uniform f64 in [0, 1) (`RandomSource.nextDouble`).
    fn next_f64(&mut self) -> f64 {
        self.next_f32() as f64
    }

    /// Approximate standard gaussian: the sum of three uniforms in (-1, 1) (variance 1,
    /// range ±3), standing in for vanilla's `Random.nextGaussian`.
    fn gaussian(&mut self) -> f32 {
        self.next_f32() * 2.0 - 1.0 + (self.next_f32() * 2.0 - 1.0) + (self.next_f32() * 2.0 - 1.0)
    }
}

/// Draws the pool: camera-facing quads over the particle atlas, one draw call for all
/// particles. The vertex/pipeline shape is the entity renderer's (same shader, textureLoad
/// sprite rects, vertex light colour, fog); particles differ in culling (billboards face
/// the camera) and depth (blended, no depth write).
pub struct ParticleRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: Option<wgpu::BindGroup>,
    vertex_buffer: wgpu::Buffer,
    capacity: usize,
    vertices: Vec<EntityVertex>,
    /// Frames since the last debug log of the drawn count.
    log_clock: u32,
    pub drawn: u32,
}

impl ParticleRenderer {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        target_format: wgpu::TextureFormat,
        camera_layout: &wgpu::BindGroupLayout,
        lightmap_layout: &wgpu::BindGroupLayout,
    ) -> ParticleRenderer {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particle"),
            source: wgpu::ShaderSource::Wgsl(include_str!("entity.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particle-pipeline-layout"),
            bind_group_layouts: &[Some(camera_layout), Some(&layout), Some(lightmap_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particle"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(EntityVertex::LAYOUT)],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // Billboards are seen from whichever side faces the camera.
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let capacity = 4096;
        let vertex_buffer = make_vertex_buffer(&device, capacity);
        ParticleRenderer {
            device,
            queue,
            pipeline,
            layout,
            bind_group: None,
            vertex_buffer,
            capacity,
            vertices: Vec::new(),
            log_clock: 0,
            drawn: 0,
        }
    }

    /// Uploads the atlas of `types` as the particle texture (one texture for all types).
    pub fn set_types(&mut self, types: &Arc<ParticleTypes>) {
        let (pixels, size) = types.atlas();
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("particle-atlas"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 4),
                rows_per_image: Some(size),
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );
        self.bind_group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle-bind"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(
                    &texture.create_view(&Default::default()),
                ),
            }],
        }));
    }

    /// Poses every particle as a camera-facing quad into the vertex buffer (positions
    /// interpolated within the tick; light resampled when a particle leaves its block).
    /// The camera bind group of the main pass is at group 0; roll is ignored (billboards
    /// do not tilt with the hurt tilt).
    pub fn prepare(
        &mut self,
        particles: &mut Particles,
        camera: Camera,
        partial: f32,
        world: &dyn BlockView,
    ) {
        self.vertices.clear();
        self.drawn = 0;
        // Debug visibility: how many particles the frame poses (once a second).
        self.log_clock += 1;
        let log = self.log_clock.is_multiple_of(60);
        let types = particles.types.clone();
        let Some(types) = types.as_deref() else {
            return;
        };
        let (right, up) = billboard_axes(camera.yaw, camera.pitch);
        for p in &mut particles.pool {
            let pos = p.prev_pos.lerp(p.pos, partial);
            // Light at the particle's position, resampled when it crosses a block boundary.
            let block = BlockPos::new(
                pos.x.floor() as i32,
                pos.y.floor() as i32,
                pos.z.floor() as i32,
            );
            if p.sky_light == UNSAMPLED || block != p.light_at {
                p.light_at = block;
                p.sky_light = world.sky_light(block);
                p.block_light = world.block_light(block);
            }
            let sprites = types.sprites(p.kind);
            if sprites.is_empty() {
                continue;
            }
            // The sprite for the particle's age (multi-sprite types animate over the list,
            // like vanilla's `setSpriteFromAge`).
            let index = ((p.age / p.lifetime) * sprites.len() as f32).floor() as usize;
            let rect = sprites[index.min(sprites.len() - 1)];
            let light = if p.behaviour.full_light {
                // HugeExplosionParticle.getLightCoords: always full light.
                (15u8, 15u8)
            } else {
                (p.block_light, p.sky_light)
            };
            let alpha = p.behaviour.alpha.unwrap_or(1.0);
            // The light is applied by the shader from the light texture, not multiplied into
            // the colour here (that would light twice).
            let color = [p.color[0], p.color[1], p.color[2], alpha];
            let rel = (pos.as_dvec3() - camera.pos).as_vec3();
            // The quad size the behaviour rolled at spawn, grown for its age
            // (`SingleQuadParticle.getQuadSize`).
            let growth = p.behaviour.growth.scale((p.age + partial) / p.lifetime);
            let half = p.quad * growth;
            let r = right * half;
            let u = up * half;
            let corners = [rel - r - u, rel + r - u, rel + r + u, rel - r + u];
            let (x, y, w, h) = (
                rect[0] as f32,
                rect[1] as f32,
                rect[2] as f32,
                rect[3] as f32,
            );
            let uvs = [[x, y + h], [x + w, y + h], [x + w, y], [x, y]];
            let rect16 = rect;
            for i in [0usize, 1, 2, 0, 2, 3] {
                self.vertices.push(EntityVertex {
                    pos: corners[i].to_array(),
                    uv: uvs[i],
                    color,
                    rect: rect16,
                    light: [light.0 as u16, light.1 as u16, 0, 0],
                });
            }
            self.drawn += 1;
        }
        if log && (self.drawn > 0 || particles.alive() > 0) {
            tracing::debug!(
                "particles: {} drawn of {} alive, {} vertices",
                self.drawn,
                particles.alive(),
                self.vertices.len()
            );
        }
        if self.vertices.len() > self.capacity {
            self.capacity = self.vertices.len().next_power_of_two();
            self.vertex_buffer = make_vertex_buffer(&self.device, self.capacity);
        }
        if !self.vertices.is_empty() {
            self.queue
                .write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.vertices));
        }
    }

    /// Draws into the main pass after the entities (camera bind group must be set at
    /// group 0). One draw call for every particle.
    pub fn draw<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) -> u32 {
        let Some(bind) = &self.bind_group else {
            return 0;
        };
        if self.vertices.is_empty() {
            return 0;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(1, bind, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..self.vertices.len() as u32, 0..1);
        1
    }
}

fn make_vertex_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("particle-vertices"),
        size: (capacity * std::mem::size_of::<EntityVertex>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Billboard axes for a camera look (yaw/pitch in degrees, Minecraft's convention):
/// screen-right horizontal, up perpendicular to the look. Never degenerate (right and look
/// are orthogonal unit vectors).
fn billboard_axes(yaw: f32, pitch: f32) -> (Vec3, Vec3) {
    let yaw = yaw.to_radians();
    let pitch = pitch.to_radians();
    let look = Vec3::new(
        -pitch.cos() * yaw.sin(),
        -pitch.sin(),
        pitch.cos() * yaw.cos(),
    );
    let right = Vec3::new(-yaw.cos(), 0.0, -yaw.sin());
    let up = right.cross(look).normalize();
    (right, up)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A type table with hand-written sprite lists (no assets needed).
    fn types_for_tests(entries: &[(&str, &[[u16; 4]])]) -> Arc<ParticleTypes> {
        let mut types = ParticleTypes {
            types: Vec::new(),
            by_name: Default::default(),
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
        particles.spawn(&request, request.pos);
    }

    /// Spawns with a 3-sigma gaussian spread from the packet, as `handleParticleEvent` does.
    fn spawn_spread(particles: &mut Particles, kind: &str, count: i32) {
        let mut request = spawn_request(kind, count);
        request.pos = DVec3::new(10.0, 64.0, -5.0);
        particles.spawn(&request, request.pos);
    }

    /// True while the pool still has particles.
    fn alive(particles: &Particles) -> bool {
        particles.alive() > 0
    }

    #[test]
    fn count_zero_spawns_one_at_the_position() {
        let mut particles = Particles::new(types_for_tests(&[("flame", &[[0, 0, 8, 8]])]));
        particles.spawn(&spawn_request("flame", 0), DVec3::new(10.0, 64.0, -5.0));
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
            particles.tick();
        }
        assert_eq!(particles.alive(), 1);
        // ...expired one tick later.
        particles.tick();
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
        particles.spawn(&request, DVec3::ZERO);
        assert_eq!(particles.alive(), 0);
        // The packet can override the limiter.
        let mut request = spawn_request("smoke", 0);
        request.pos = DVec3::new(33.0, 0.0, 0.0);
        request.override_limiter = true;
        particles.spawn(&request, DVec3::ZERO);
        assert_eq!(particles.alive(), 1);
        // 31 blocks survives the limiter.
        let mut request = spawn_request("smoke", 0);
        request.pos = DVec3::new(31.0, 0.0, 0.0);
        particles.spawn(&request, DVec3::ZERO);
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
        particles.tick();
        let p = &particles.pool[0];
        assert_eq!(p.prev_pos, start);
        assert_eq!(p.pos, start + vel);
        assert_eq!(p.vel, vel * 0.96);
    }

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
        particles.spawn(&request, request.pos);
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
        particles.spawn(&request, request.pos);
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
        particles.spawn(&request, request.pos);
        let p = &particles.pool[0];
        // DustParticleBase: life (int)(8/(r*0.8+0.2) * 2) = 16..=80, quad 0.75*2 = 1.5x.
        assert!((16.0..=80.0).contains(&p.lifetime), "life {}", p.lifetime);
        assert!((0.15..=0.3).contains(&p.quad), "quad size {}", p.quad);
        // DustParticle randomises the red option colour by 0.8..=1.0 of its 0.6..=1.0 factor.
        assert!(p.color[0] > 0.0 && p.color[0] <= 1.0, "r {}", p.color[0]);
        assert_eq!(p.color[1], 0.0);
        assert_eq!(p.color[2], 0.0);
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
}
