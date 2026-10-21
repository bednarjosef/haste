//! Fireworks, as vanilla's `FireworkParticles`: the `Starter` a rocket's burst makes (its
//! explosions' sparks, and a flash per explosion), the `SparkParticle`s those sparks are
//! (gravity, drag, the fade towards their fade colour, the trail children and the twinkle),
//! and the `OverlayParticle` flash.
//!
//! Vanilla queues the particles a tick adds (`ParticleEngine.add` into `particlesToAdd`) and
//! appends them after the tick's loop, so new sparks are first ticked the tick after. The
//! starters and sparks here append to a pending list the engine drains the same way.
//!
//! Colours stay in sRGB, as vanilla holds them (the fade lerps there), and are converted to
//! the linear RGB the pool draws with, the same way `push` converts its colours.

use glam::{DVec3, Vec3};

use haste_core::Identifier;

use super::engine::{ParticleState, Particles};
use super::providers::ParticleBehaviour;
use super::rng::Rng;
use super::ParticleSpawn;

const PI_F32: f32 = std::f32::consts::PI;

/// `FireworkExplosion.Shape`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FireworkShape {
    SmallBall,
    LargeBall,
    Star,
    Creeper,
    Burst,
}

/// One explosion of a firework (`FireworkExplosion`): its shape, the colours its sparks pick
/// from (ARGB ints, the alpha ignored), the colours they fade to, and the trail and twinkle
/// flags.
#[derive(Clone, Debug, PartialEq)]
pub struct FireworkExplosion {
    pub shape: FireworkShape,
    pub colors: Vec<i32>,
    pub fade_colors: Vec<i32>,
    pub trail: bool,
    pub twinkle: bool,
}

/// What a firework pool particle is beyond its type's behaviour.
#[derive(Clone, Copy, Debug)]
pub(super) enum Firework {
    /// A `SparkParticle`: its colour in sRGB (the fade lerps there), the fade target, and the
    /// trail and twinkle flags.
    Spark {
        srgb: [f32; 3],
        fade: Option<[f32; 3]>,
        trail: bool,
        twinkle: bool,
    },
    /// The `OverlayParticle` flash drawn over an explosion's start.
    Flash,
}

/// A pool type a firework spawns, with its behaviour.
#[derive(Clone, Copy)]
pub(super) struct Kind {
    pub(super) index: u32,
    pub(super) behaviour: &'static ParticleBehaviour,
}

/// `DyeColor.BLACK.getFireworkColor()`: the colour of an explosion with none.
const BLACK_FIREWORK: i32 = 1_973_019;

/// `FireworkParticles.Starter.CREEPER_PARTICLE_COORDS`.
const CREEPER_COORDS: [[f64; 2]; 12] = [
    [0.0, 0.2],
    [0.2, 0.2],
    [0.2, 0.6],
    [0.6, 0.6],
    [0.6, 0.2],
    [0.2, 0.2],
    [0.2, 0.0],
    [0.4, 0.0],
    [0.4, -0.6],
    [0.2, -0.6],
    [0.2, -0.4],
    [0.0, -0.4],
];

/// `FireworkParticles.Starter.STAR_PARTICLE_COORDS`.
const STAR_COORDS: [[f64; 2]; 6] = [
    [0.0, 1.0],
    [0.3455, 0.309],
    [0.9511, 0.309],
    [0.379_591_836_734_693_9, -0.126_530_612_244_897_95],
    [0.612_244_897_959_183_7, -0.804_081_632_653_061_2],
    [0.0, -0.359_183_673_469_387_73],
];

/// `FireworkParticles.Starter`: one rocket burst. It ticks its explosions out, one every
/// other tick, and is done `lifetime` ticks in.
pub(super) struct Starter {
    pos: DVec3,
    vel: DVec3,
    explosions: Vec<FireworkExplosion>,
    life: i32,
    lifetime: i32,
    spark: Kind,
    flash: Option<Kind>,
}

impl Starter {
    /// The constructor: `explosions.len() * 2 - 1` ticks, plus 15 when any explosion twinkles.
    /// `explosions` must not be empty (an empty firework is the poof, not a starter).
    pub(super) fn new(
        pos: DVec3,
        vel: DVec3,
        explosions: &[FireworkExplosion],
        spark: Kind,
        flash: Option<Kind>,
    ) -> Starter {
        let mut lifetime = explosions.len() as i32 * 2 - 1;
        if explosions.iter().any(|e| e.twinkle) {
            lifetime += 15;
        }
        Starter {
            pos,
            vel,
            explosions: explosions.to_vec(),
            life: 0,
            lifetime,
            spark,
            flash,
        }
    }

    /// `Starter.tick`: on each even life inside the explosions, emits that explosion's sparks
    /// and the flash. `false` once the starter is done (the caller removes it).
    pub(super) fn tick(&mut self, out: &mut Vec<ParticleState>, rng: &mut Rng) -> bool {
        if self.life % 2 == 0 && self.life / 2 < self.explosions.len() as i32 {
            let explosion = &self.explosions[(self.life / 2) as usize];
            let black = [BLACK_FIREWORK];
            let colors: &[i32] = if explosion.colors.is_empty() {
                &black
            } else {
                &explosion.colors
            };
            let shot = Shot {
                colors,
                fades: &explosion.fade_colors,
                trail: explosion.trail,
                twinkle: explosion.twinkle,
            };
            let mut sparks = Sparks {
                out: &mut *out,
                rng: &mut *rng,
                kind: self.spark,
                origin: self.pos,
            };
            match explosion.shape {
                FireworkShape::SmallBall => sparks.ball(0.25, 2, &shot),
                FireworkShape::LargeBall => sparks.ball(0.5, 4, &shot),
                FireworkShape::Star => sparks.shape(0.5, &STAR_COORDS, false, &shot),
                FireworkShape::Creeper => sparks.shape(0.5, &CREEPER_COORDS, true, &shot),
                FireworkShape::Burst => sparks.burst(self.vel, &shot),
            }
            if let Some(flash) = self.flash {
                out.push(flash_particle(flash, self.pos, colors[0]));
            }
        }
        self.life += 1;
        self.life <= self.lifetime
    }
}

/// One explosion's spark settings: the colours and fades the sparks pick from, and the flags.
struct Shot<'a> {
    colors: &'a [i32],
    fades: &'a [i32],
    trail: bool,
    twinkle: bool,
}

/// The sparks a starter creates at its position (`Starter.createParticle` and the three
/// shape generators).
struct Sparks<'a> {
    out: &'a mut Vec<ParticleState>,
    rng: &'a mut Rng,
    kind: Kind,
    origin: DVec3,
}

impl Sparks<'_> {
    /// `Starter.createParticle`: one spark moving at `vel` (no jitter), its colour and fade
    /// picked at random from the explosion's lists.
    fn spark(&mut self, vel: DVec3, shot: &Shot) {
        let quad = spark_quad(self.rng);
        let lifetime = spark_lifetime(self.rng);
        let srgb = rgb(pick(shot.colors, self.rng));
        let fade = if shot.fades.is_empty() {
            None
        } else {
            Some(rgb(pick(shot.fades, self.rng)))
        };
        self.out.push(ParticleState {
            kind: self.kind.index,
            pos: self.origin.as_vec3(),
            prev_pos: self.origin.as_vec3(),
            vel: vel.as_vec3(),
            age: 0.0,
            lifetime: lifetime as f32,
            behaviour: *self.kind.behaviour,
            quad,
            color: crate::sky::linear(srgb),
            alpha: 0.99,
            portal_start: self.origin,
            block_sprite: None,
            firework: Some(Firework::Spark {
                srgb,
                fade,
                trail: shot.trail,
                twinkle: shot.twinkle,
            }),
        });
    }

    /// `createParticleBall`: a cube of `steps` out, one spark per lattice point with a
    /// random offset. Rows off the cube's faces keep only their two end layers (the
    /// `zStep += steps * 2 - 1` skip), so the ball is a shell.
    fn ball(&mut self, base: f64, steps: i32, shot: &Shot) {
        for y in -steps..=steps {
            for x in -steps..=steps {
                let mut z = -steps;
                while z <= steps {
                    let xa = x as f64 + (self.rng.next_f64() - self.rng.next_f64()) * 0.5;
                    let ya = y as f64 + (self.rng.next_f64() - self.rng.next_f64()) * 0.5;
                    let za = z as f64 + (self.rng.next_f64() - self.rng.next_f64()) * 0.5;
                    let len = (xa * xa + ya * ya + za * za).sqrt() / base
                        + self.rng.gaussian() as f64 * 0.05;
                    self.spark(DVec3::new(xa / len, ya / len, za / len), shot);
                    if y != -steps && y != steps && x != -steps && x != steps {
                        z += steps * 2 - 1;
                    }
                    z += 1;
                }
            }
        }
    }

    /// `createParticleShape`: the outline's vertices and the lines between them, swept over
    /// three rotations about the vertical axis. `flat` (the creeper face) uses a small
    /// rotation step, the star a large one.
    fn shape(&mut self, base: f64, coords: &[[f64; 2]], flat: bool, shot: &Shot) {
        let [sx, sy] = coords[0];
        self.spark(DVec3::new(sx * base, sy * base, 0.0), shot);
        let base_angle = self.rng.next_f32() * PI_F32;
        let angle_mod = if flat { 0.034 } else { 0.34 };
        for angle_step in 0..3 {
            let angle = base_angle as f64 + (angle_step as f32 * PI_F32) as f64 * angle_mod;
            let (mut ox, mut oy) = (sx, sy);
            for &[tx, ty] in &coords[1..] {
                for quarter in 1..=4 {
                    let t = quarter as f64 * 0.25;
                    let xa = lerp(t, ox, tx) * base;
                    let ya = lerp(t, oy, ty) * base;
                    let za = xa * angle.sin();
                    let xa = xa * angle.cos();
                    for flip in [-1.0, 1.0] {
                        self.spark(DVec3::new(xa * flip, ya, za * flip), shot);
                    }
                }
                ox = tx;
                oy = ty;
            }
        }
    }

    /// `createParticleBurst`: 70 sparks spreading from the rocket's velocity, halved.
    fn burst(&mut self, vel: DVec3, shot: &Shot) {
        let base_x = self.rng.gaussian() as f64 * 0.05;
        let base_z = self.rng.gaussian() as f64 * 0.05;
        for _ in 0..70 {
            let xa = vel.x * 0.5 + self.rng.gaussian() as f64 * 0.15 + base_x;
            let za = vel.z * 0.5 + self.rng.gaussian() as f64 * 0.15 + base_z;
            let ya = vel.y * 0.5 + self.rng.next_f64() * 0.5;
            self.spark(DVec3::new(xa, ya, za), shot);
        }
    }
}

/// `FlashProvider`: an `OverlayParticle` at rest, coloured by the explosion's first colour
/// (the option's alpha is opaque, and the flash's own alpha takes over on its first tick).
fn flash_particle(kind: Kind, pos: DVec3, argb: i32) -> ParticleState {
    ParticleState {
        kind: kind.index,
        pos: pos.as_vec3(),
        prev_pos: pos.as_vec3(),
        vel: Vec3::ZERO,
        age: 0.0,
        lifetime: kind.behaviour.lifetime.roll(0.0),
        behaviour: *kind.behaviour,
        quad: 0.0,
        color: crate::sky::linear(rgb(argb)),
        alpha: 1.0,
        portal_start: pos,
        block_sprite: None,
        firework: Some(Firework::Flash),
    }
}

/// One tick of a firework particle, after the engine's age step. A spark moves and drags as
/// `Particle.tick` does, then `SimpleAnimatedParticle.tick` fades it past half its life, and
/// on the even ticks of its first half it spawns a trail child. The flash's quad and alpha
/// follow its age; they are evaluated at the tick's age, where vanilla evaluates them per
/// frame at the partial tick, so a frame shows the value one tick behind.
pub(super) fn tick(p: &mut ParticleState, pending: &mut Vec<ParticleState>, rng: &mut Rng) {
    let b = p.behaviour;
    let Some(fw) = &mut p.firework else {
        return;
    };
    match fw {
        Firework::Flash => {
            // OverlayParticle.extract's alpha and getQuadSize, at age - 1.
            let s = p.age - 1.0;
            p.quad = 7.1 * (s * 0.25 * PI_F32).sin();
            p.alpha = 0.6 - s * 0.25 * 0.5;
        }
        Firework::Spark {
            srgb,
            fade,
            trail,
            twinkle,
        } => {
            // Particle.tick: gravity, move, drag.
            p.vel.y -= 0.04 * b.gravity;
            p.pos += p.vel;
            p.vel *= b.friction;
            // SimpleAnimatedParticle.tick: past half its life the alpha falls and the colour
            // moves towards the fade colour.
            let (age, life) = (p.age as i32, p.lifetime as i32);
            let half = life / 2;
            if age > half {
                p.alpha = 1.0 - (p.age - half as f32) / p.lifetime;
                if let Some(target) = *fade {
                    for k in 0..3 {
                        srgb[k] += (target[k] - srgb[k]) * 0.2;
                    }
                    p.color = crate::sky::linear(*srgb);
                }
            }
            // SparkParticle.tick's trail: a still child at this spot, starting half done, on
            // the even ticks while the parent is in its first half.
            if *trail && age < half && (age + life) % 2 == 0 {
                let quad = spark_quad(rng);
                let child_life = spark_lifetime(rng);
                pending.push(ParticleState {
                    kind: p.kind,
                    pos: p.pos,
                    prev_pos: p.pos,
                    vel: Vec3::ZERO,
                    age: (child_life / 2) as f32,
                    lifetime: child_life as f32,
                    behaviour: b,
                    quad,
                    color: p.color,
                    alpha: 0.99,
                    portal_start: p.portal_start,
                    block_sprite: None,
                    firework: Some(Firework::Spark {
                        srgb: *srgb,
                        fade: *fade,
                        trail: false,
                        twinkle: *twinkle,
                    }),
                });
            }
        }
    }
}

/// `SparkParticle.extract`'s twinkle: a twinkling spark is hidden through the odd thirds of
/// its life after the first third.
pub(super) fn hidden(p: &ParticleState) -> bool {
    match p.firework {
        Some(Firework::Spark { twinkle: true, .. }) => {
            let (age, life) = (p.age as i32, p.lifetime as i32);
            age >= life / 3 && (age + life) / 3 % 2 != 0
        }
        _ => false,
    }
}

/// `Particle`'s spawn quad half-width (`0.1 * (r * 0.5 + 0.5) * 2`), scaled 0.75 by
/// `SparkParticle`.
fn spark_quad(rng: &mut Rng) -> f32 {
    0.1 * (rng.next_f32() * 0.5 + 0.5) * 2.0 * 0.75
}

/// `SparkParticle`'s lifetime: `48 + nextInt(12)`.
fn spark_lifetime(rng: &mut Rng) -> i32 {
    48 + (rng.next_f32() * 12.0) as i32
}

/// An ARGB int's RGB as sRGB floats (the alpha ignored).
fn rgb(argb: i32) -> [f32; 3] {
    [
        ((argb >> 16) & 0xFF) as f32 / 255.0,
        ((argb >> 8) & 0xFF) as f32 / 255.0,
        (argb & 0xFF) as f32 / 255.0,
    ]
}

/// A random element of a non-empty list (`Util.getRandom`).
fn pick(list: &[i32], rng: &mut Rng) -> i32 {
    let n = list.len();
    list[((rng.next_f32() * n as f32) as usize).min(n - 1)]
}

/// `Mth.lerp`.
fn lerp(t: f64, start: f64, end: f64) -> f64 {
    start + t * (end - start)
}

impl Particles {
    /// A rocket's burst (`ClientLevel.createFireworks`, from `FireworkRocketEntity`'s status
    /// 17): the explosions' starter, ticking from the next tick like vanilla's queued add. An
    /// empty firework (no explosions) is a POOF cloud of 2 + `nextInt(3)` puffs instead.
    /// `vel` is the rocket's velocity (the burst's drift).
    pub fn firework_burst(&mut self, pos: DVec3, vel: DVec3, explosions: &[FireworkExplosion]) {
        if explosions.is_empty() {
            let count = 2 + (self.rng.next_f32() * 3.0) as i32;
            for _ in 0..count {
                let offset = [
                    self.rng.gaussian() * 0.05,
                    0.005,
                    self.rng.gaussian() * 0.05,
                ];
                let poof = ParticleSpawn::client(Identifier::minecraft("poof"), pos, offset);
                self.spawn(&poof, self.camera, None);
            }
            return;
        }
        let Some(spark) = self.firework_kind(&Identifier::minecraft("firework")) else {
            return;
        };
        let flash = self.firework_kind(&Identifier::minecraft("flash"));
        self.starters
            .push(Starter::new(pos, vel, explosions, spark, flash));
    }

    /// A firework type's pool index and behaviour; an unknown type logs once and is `None`.
    fn firework_kind(&mut self, id: &Identifier) -> Option<Kind> {
        match self.types.as_deref()?.index(id) {
            Some(index) => Some(Kind {
                index,
                behaviour: ParticleBehaviour::of(id),
            }),
            None => {
                if !self.unknown.contains(id) {
                    self.unknown.push(id.clone());
                    tracing::warn!("particle type {} has no atlas sprites; skipped", id);
                }
                None
            }
        }
    }
}
