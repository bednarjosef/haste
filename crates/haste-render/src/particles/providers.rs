//! The per-type behaviour table: each registry id's numbers ported from the vanilla particle
//! class its provider builds (vanilla `ParticleResources.registerProviders` and the
//! `ParticleProvider`s), the generic `Particle` defaults for every other id.

use haste_core::Identifier;

use super::particle::{Lifetime, Motion};
use super::single_quad_particle::{Quad, QuadGrowth};

/// Per-type behaviour ported from vanilla's particle classes. Every entry names the class it
/// follows, so the numbers are traceable back to vanilla (never invented here).
///
/// Vanilla's generic base (`Particle`) is `DEFAULT`: no gravity, `friction` 0.98, a
/// `(int)(4.0 / (r * 0.9 + 0.1))` lifetime (4..=40 ticks), a `0.1 * (r * 0.5 + 0.5) * 2.0`
/// quad (0.1..=0.2 blocks half-width, 0.2..=0.4 across), white, and the packet's speed
/// inside a random direction.
#[derive(Clone, Copy, Debug)]
pub(super) struct ParticleBehaviour {
    /// The vanilla class this entry follows (`Particle` for the generic default).
    pub(super) source: &'static str,
    /// Gravity in blocks/tick²; vanilla's `Particle.tick` does `yd -= 0.04 * gravity`.
    pub(super) gravity: f32,
    /// Velocity retained per tick (`Particle.friction`).
    pub(super) friction: f32,
    /// Lifetime in ticks.
    pub(super) lifetime: Lifetime,
    /// Quad size at spawn (vanilla's `quadSize`).
    pub(super) quad: Quad,
    /// How the quad size changes over the particle's life (`getQuadSize` overrides).
    pub(super) growth: QuadGrowth,
    /// Base colour (vanilla's `rCol`/`gCol`/`bCol`, 1.0 white when unset).
    pub(super) color: [f32; 3],
    /// Spawn-time colour jitter: `rCol = randomize(low..high, color)` when `Some`, as
    /// `BaseAshSmokeParticle`'s grey and `CritParticle`'s bright grey do.
    pub(super) color_jitter: Option<(f32, f32)>,
    /// Multiplier on the start colour, as `CritParticle.MagicProvider`'s
    /// `rCol *= 0.3; gCol *= 0.8` for `enchanted_hit`.
    pub(super) color_scale: Option<[f32; 3]>,
    /// Alpha (vanilla's `alpha`, 1.0 when unset).
    pub(super) alpha: Option<f32>,
    /// Per-tick colour multiplier (`CritParticle.tick`: `gCol *= 0.96; bCol *= 0.9`).
    pub(super) color_fade: [f32; 3],
    /// Velocity handling at spawn (see [`Motion`]).
    pub(super) motion: Motion,
    /// `Particle.speedUpWhenYMotionIsBlocked` (smoke, heart, dust, note: `xd/z *= 1.1` when a
    /// vertical move is blocked).
    pub(super) speed_up_when_blocked: bool,
    /// `CritParticle` runs one tick in its constructor.
    pub(super) tick_at_spawn: bool,
    /// `NoRenderParticle` subclasses (`HugeExplosionSeedParticle`): the type has no
    /// atlas sprites and draws nothing; its tick is the behaviour's own instead.
    pub(super) no_render: bool,
    /// Full light (`HugeExplosionParticle.getLightCoords` returns 15728880).
    pub(super) full_light: bool,
    /// `PortalParticle`'s spiral path instead of the generic integration.
    pub(super) portal_motion: bool,
}

impl ParticleBehaviour {
    /// Vanilla's generic `Particle`.
    pub(super) const DEFAULT: ParticleBehaviour = ParticleBehaviour {
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
        no_render: false,
        full_light: false,
        portal_motion: false,
    };

    /// Behaviour of a registry id, `DEFAULT` for ids without an entry (mods and vanilla
    /// types we have not ported).
    /// Behaviour of a registry id, `DEFAULT` for ids without an entry (mods and vanilla
    /// types we have not ported).
    pub(super) fn of(id: &Identifier) -> &'static ParticleBehaviour {
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
            // SpellParticle.MobEffectProvider (entity_effect) and SpellParticle.Provider
            // (effect, always white — the provider ignores the option's colour): friction
            // 0.96, gravity -0.1, life (int)(8/(r*0.8+0.2)) = 8..=40, quad 0.75x, the option's
            // colour on entity_effect (a tipped arrow's trail), no collision, the spell drift
            // for velocity.
            "entity_effect" | "effect" => &ParticleBehaviour {
                source: "SpellParticle",
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
                motion: Motion::Spell,
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
            // TerrainParticle (block): gravity 1, quad halved, grey 0.6 base. The packet's
            // BlockParticleOption (its block state) is not decoded, so server-sent block
            // particles draw the behaviour's grey; the client's own (see
            // `spawn_destroy`/`spawn_crack`) carry their block's sprite and tint.
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
            // HugeExplosionSeedParticle (explosion_emitter, a NoRenderParticle): life 8,
            // no quad of its own — every tick it spawns six `explosion` particles at ±4
            // around itself, their size this tick's age/lifetime (see the engine's tick).
            "explosion_emitter" => &ParticleBehaviour {
                source: "HugeExplosionSeedParticle",
                lifetime: Lifetime::Fixed(8),
                motion: Motion::None,
                no_render: true,
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
            // FireworkParticles.SparkParticle: its lifetime, quad, colour and fade are set when
            // it is spawned (see the firework module); the gravity is its own.
            "firework" => &ParticleBehaviour {
                source: "SparkParticle",
                gravity: 0.1,
                ..ParticleBehaviour::DEFAULT
            },
            // FireworkParticles.OverlayParticle (FLASH): life 4, no physics; its quad and
            // alpha follow its age (see the firework module).
            "flash" => &ParticleBehaviour {
                source: "OverlayParticle",
                lifetime: Lifetime::Fixed(4),
                motion: Motion::None,
                ..ParticleBehaviour::DEFAULT
            },
            _ => &ParticleBehaviour::DEFAULT,
        }
    }
}
