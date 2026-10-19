// The `explode` packet (vanilla `ClientPacketListener.handleExplosion`): the sound it
// carries (played as a local sound at the centre, volume 4), the explosion particle
// itself, the block particles handed to the tracker (`ClientLevel
// .trackExplosionEffects`, see [`super::explosion_tracker`]) and the player knockback
// added to our own velocity (`Entity.pushFromExplosion` -> `push`).

use haste_core::Identifier;
use haste_protocol::ClientboundPlay;

use super::explosion_tracker::TrackedParticle;
use super::particles::{block_state_option, dust_option};
use super::Session;

/// `SoundSource.BLOCKS`'s index in vanilla's enum (the sound packets' category VarInt).
const BLOCKS: i32 = 4;

impl Session {
    /// The `explode` packet; returns the packet when it is not one.
    pub(super) fn apply_explosion(&mut self, p: ClientboundPlay) -> Option<ClientboundPlay> {
        let ClientboundPlay::Explode(p) = p else {
            return Some(p);
        };
        // The local sound at the centre: volume 4, pitch `(1 + (r - r) * 0.2) * 0.7`
        // with the seed drawn after the two pitch draws, all from the level's random
        // (one shared random here, as in vanilla).
        if p.play_sound {
            let (r1, r2) = (
                self.explosion_rng.next_float(),
                self.explosion_rng.next_float(),
            );
            let seed = self.explosion_rng.next_long();
            self.sound_requests.push(crate::sound::SoundRequest::Play(
                crate::sound::PlayRequest {
                    sound: crate::sound::SoundRef::of(&p.sound),
                    category: BLOCKS,
                    pos: crate::sound::SoundPos::Fixed(p.center),
                    volume: 4.0,
                    pitch: explosion_sound_pitch(r1, r2),
                    seed,
                },
            ));
        }
        // The explosion particle at the centre with speed (1, 0, 0) (the seed type
        // ignores it; other types the server names read it as their own).
        if let Some(kind) = self.particle_kind(p.particle.kind) {
            self.particle_spawns.push(haste_render::ParticleSpawn {
                kind,
                pos: p.center,
                offset: [1.0, 0.0, 0.0],
                max_speed: [1.0, 0.0, 0.0],
                count: 0,
                randomization: 0,
                override_limiter: false,
                dust: None,
                block_state: None,
            });
        }
        // `trackExplosionEffects`: the block particles wait for the level tick, which
        // spawns them weighted by the block count. The names resolve against the
        // connection's table here, where the packet is the only borrower.
        let names = self.particle_names.clone();
        let particles = p
            .block_particles
            .iter()
            .map(|entry| {
                let kind = particle_kind(&names, entry.particle.kind);
                TrackedParticle {
                    kind: kind.clone(),
                    scaling: entry.scaling,
                    speed: entry.speed,
                    weight: entry.weight,
                    dust: kind
                        .as_ref()
                        .and_then(|k| dust_option(k, &entry.particle.data)),
                    block_state: kind
                        .as_ref()
                        .and_then(|k| block_state_option(k, &entry.particle.data)),
                }
            })
            .collect();
        self.explosions
            .track(p.center, p.radius, p.block_count, particles);
        // `pushFromExplosion`: the impulse is only added when it is finite (vanilla's
        // `push` checks), and it lands on the velocity the next tick's move starts from.
        if let Some(impulse) = p.player_knockback {
            if let Some(velocity) = pushed_velocity(self.player.state.velocity, impulse) {
                self.player.state.velocity = velocity;
                tracing::info!(
                    "explosion knockback ({:.3}, {:.3}, {:.3}), velocity now ({:.3}, {:.3}, {:.3})",
                    impulse.x,
                    impulse.y,
                    impulse.z,
                    velocity.x,
                    velocity.y,
                    velocity.z
                );
            }
        }
        None
    }

    /// The particle type a wire id names (`minecraft:particle_type`'s registry, the
    /// connection's static table); `None` for ids outside it.
    fn particle_kind(&self, kind: i32) -> Option<Identifier> {
        particle_kind(&self.particle_names, kind)
    }
}

/// The particle type a wire id names in the connection's table; `None` for ids outside
/// it (a modded type the client has no name for).
fn particle_kind(names: &[Identifier], kind: i32) -> Option<Identifier> {
    names
        .get(usize::try_from(kind).ok().unwrap_or(usize::MAX))
        .cloned()
}

/// The explosion sound's pitch: `(1 + (r1 - r2) * 0.2) * 0.7`.
fn explosion_sound_pitch(r1: f32, r2: f32) -> f32 {
    (1.0 + (r1 - r2) * 0.2) * 0.7
}

/// The knockback's impulse as `Entity.push` applies it: the current velocity plus the
/// impulse, only when every component is finite.
fn pushed_velocity(velocity: glam::DVec3, impulse: glam::DVec3) -> Option<glam::DVec3> {
    impulse.is_finite().then_some(velocity + impulse)
}

#[cfg(test)]
mod tests;
