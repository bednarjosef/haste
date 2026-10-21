//! The totem's pop particles: vanilla `TrackingEmitter` with `ParticleTypes.TOTEM_OF_UNDYING`
//! (`ClientPacketListener.handleEntityEvent` status 35 adds it to the entity). Each game tick
//! it spawns 16 attempts inside the entity's box and lives for 30 ticks.

use glam::DVec3;
use haste_audio::random::LegacyRandom;
use haste_core::Identifier;
use haste_render::ParticleSpawn;

/// `TrackingEmitter`'s default `lifeTime` as the totem's status 35 sets it: 30 ticks.
pub const LIFE_TIME: u32 = 30;

/// The attempts per tick (`TrackingEmitter.tick`'s loop).
const ATTEMPTS: u32 = 16;

/// One tracking emitter. It follows its entity: the position is read from the store each
/// tick, so the particles move with it.
#[derive(Clone, Copy, Debug)]
pub struct TotemEmitter {
    pub entity: haste_core::EntityId,
    /// Ticks run so far (`TrackingEmitter.life`).
    life: u32,
}

impl TotemEmitter {
    pub fn new(entity: haste_core::EntityId) -> TotemEmitter {
        TotemEmitter { entity, life: 0 }
    }

    /// One `TrackingEmitter.tick`: each attempt draws a point in the cube `[-1, 1]^3`, skips
    /// it outside the unit sphere, and emits a totem particle at the entity's box (`getX(xa/4)`,
    /// `getY(0.5 + ya/4)`, `getZ(za/4)`) moving at `(xa, ya + 0.2, za)`. Returns `false` once
    /// the emitter has lived its `LIFE_TIME`: the caller drops it then.
    pub fn tick(
        &mut self,
        pos: DVec3,
        (width, height): (f32, f32),
        rng: &mut LegacyRandom,
        out: &mut Vec<ParticleSpawn>,
    ) -> bool {
        let kind = Identifier::minecraft("totem_of_undying");
        for _ in 0..ATTEMPTS {
            let xa = rng.next_float() as f64 * 2.0 - 1.0;
            let ya = rng.next_float() as f64 * 2.0 - 1.0;
            let za = rng.next_float() as f64 * 2.0 - 1.0;
            if xa * xa + ya * ya + za * za > 1.0 {
                continue;
            }
            let at = DVec3::new(
                pos.x + width as f64 * (xa / 4.0),
                pos.y + height as f64 * (0.5 + ya / 4.0),
                pos.z + width as f64 * (za / 4.0),
            );
            out.push(ParticleSpawn::client(
                kind.clone(),
                at,
                [xa as f32, (ya + 0.2) as f32, za as f32],
            ));
        }
        self.life += 1;
        self.life < LIFE_TIME
    }
}

#[cfg(test)]
#[path = "totem_emitter_tests.rs"]
mod tests;
