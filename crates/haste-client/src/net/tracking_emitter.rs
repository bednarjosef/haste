// The tracking emitter (vanilla `TrackingEmitter`, started by `ParticleEngine.createTrackingEmitter`):
// sixteen particles a tick around an entity's body for three ticks, each flying outward from the
// body's centre. The emitter reads its entity's position every tick, so the particles follow it.

use glam::DVec3;
use haste_audio::random::LegacyRandom;
use haste_core::{EntityId, Identifier};
use haste_render::ParticleSpawn;

use super::{EntitySink, Session};

/// Particle attempts per tick (`TrackingEmitter.tick`'s loop; attempts outside the unit sphere
/// are dropped, so a tick spawns fewer).
const ATTEMPTS_PER_TICK: u32 = 16;

/// The emitter's lifetime in ticks (`TrackingEmitter`'s default `lifeTime`, 3): it is removed
/// once its life reaches this.
const LIFE_TIME: u32 = 3;

/// The box an entity we hold no type for is drawn with (a standing player's box, `0.6 x 1.8`).
const FALLBACK_WIDTH: f32 = 0.6;
const FALLBACK_HEIGHT: f32 = 1.8;

/// An entity's feet position and box, as the emitter reads them (`Entity.getX(scale)` is
/// `x + width * scale`, `getY(scale)` is `y + height * scale`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    /// The feet position.
    pub pos: DVec3,
    /// The bounding box's width.
    pub width: f32,
    /// The bounding box's height.
    pub height: f32,
}

/// One tracking emitter (vanilla `TrackingEmitter`): the entity it follows, the particle type
/// it spawns and its life in ticks.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackingEmitter {
    /// The entity the emitter follows.
    pub target: EntityId,
    /// The particle type (`minecraft:crit` or `minecraft:enchanted_hit`).
    pub kind: Identifier,
    /// Ticks run so far.
    pub life: u32,
    /// The entity's body as last read; kept when the entity leaves the store (vanilla keeps
    /// reading the removed entity's last position).
    body: Body,
}

impl TrackingEmitter {
    /// Whether the emitter is still alive (`TrackingEmitter.isAlive`).
    pub fn alive(&self) -> bool {
        self.life < LIFE_TIME
    }

    /// `TrackingEmitter.tick`: sixteen attempts, each drawing three floats in `[-1, 1]`, kept
    /// when the offset lies in the unit sphere. A kept particle is placed at `getX(xa / 4)`,
    /// `getY(0.5 + ya / 4)`, `getZ(za / 4)` and flies with `(xa, ya + 0.2, za)`.
    pub fn tick(&mut self, rng: &mut LegacyRandom, out: &mut Vec<ParticleSpawn>) {
        for _ in 0..ATTEMPTS_PER_TICK {
            let xa = f64::from(rng.next_float() * 2.0 - 1.0);
            let ya = f64::from(rng.next_float() * 2.0 - 1.0);
            let za = f64::from(rng.next_float() * 2.0 - 1.0);
            if xa * xa + ya * ya + za * za > 1.0 {
                continue;
            }
            let b = self.body;
            let pos = DVec3::new(
                b.pos.x + f64::from(b.width) * (xa / 4.0),
                b.pos.y + f64::from(b.height) * (0.5 + ya / 4.0),
                b.pos.z + f64::from(b.width) * (za / 4.0),
            );
            let velocity = [xa as f32, (ya + 0.2) as f32, za as f32];
            out.push(ParticleSpawn::client(self.kind.clone(), pos, velocity));
        }
        self.life += 1;
    }
}

impl Session {
    /// `ParticleEngine.createTrackingEmitter`: a new emitter on `target` (no emitter when the
    /// entity is not in the store). Its constructor ticks it once, so its first particles spawn
    /// now; it is kept while alive.
    pub(super) fn create_tracking_emitter<E: EntitySink + ?Sized>(
        &mut self,
        target: EntityId,
        kind: Identifier,
        entities: &E,
    ) {
        let Some(body) = self.emitter_body(target, entities) else {
            return;
        };
        let mut emitter = TrackingEmitter {
            target,
            kind,
            life: 0,
            body,
        };
        emitter.tick(&mut self.tracking_rng, &mut self.particle_spawns);
        if emitter.alive() {
            self.tracking_emitters.push(emitter);
        }
    }

    /// The tracking emitters' tick (vanilla `ParticleEngine.tick`): each reads its entity's body
    /// again, spawns its particles and is removed once it is spent.
    pub(super) fn tick_tracking_emitters<E: EntitySink + ?Sized>(&mut self, entities: &E) {
        if self.tracking_emitters.is_empty() {
            return;
        }
        for i in 0..self.tracking_emitters.len() {
            let target = self.tracking_emitters[i].target;
            if let Some(body) = self.emitter_body(target, entities) {
                self.tracking_emitters[i].body = body;
            }
            self.tracking_emitters[i].tick(&mut self.tracking_rng, &mut self.particle_spawns);
        }
        self.tracking_emitters.retain(TrackingEmitter::alive);
    }

    /// The body an emitter reads for `id`: our own player from the local player's state, anything
    /// else from the store with its type's box. `None` when the store does not hold the entity.
    fn emitter_body<E: EntitySink + ?Sized>(&self, id: EntityId, entities: &E) -> Option<Body> {
        if id == self.entity_id {
            return Some(Body {
                pos: self.player.state.pos,
                width: FALLBACK_WIDTH,
                height: FALLBACK_HEIGHT,
            });
        }
        let (pos, _, _) = entities.pose(id)?;
        let (width, height) = entities
            .entity_type_info(id)
            .map(|(_, size)| size)
            .unwrap_or((FALLBACK_WIDTH, FALLBACK_HEIGHT));
        Some(Body { pos, width, height })
    }
}

/// A fresh unseeded generator for the emitters' offsets (vanilla's `Particle.random`, a
/// `RandomSource.create()` per particle; one generator here serves them all).
pub(super) fn new_random() -> LegacyRandom {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0x5eed);
    LegacyRandom::new(seed)
}

#[cfg(test)]
mod tests;
