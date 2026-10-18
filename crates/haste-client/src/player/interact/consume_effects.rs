//! The per-tick effects of a consumable being used (player workstream): vanilla
//! `ItemStack.onUseTick` -> `Consumable.shouldEmitParticlesAndSounds` /
//! `Consumable.emitParticlesAndSounds`, driven from `LivingEntity.updateUsingItem` every
//! game tick while a `minecraft:consumable` item is in use.
//!
//! The sound comes from the stack's component (`Consumable.sound`, the generic eat sound
//! by default), the volume and pitch from the component's animation, and the item
//! particles from `Consumable.hasConsumeParticles` (`LivingEntity.spawnItemParticles`).
//! The client plays its own sound: `Player.playSound` passes the player as the excluded
//! listener, so the server never sends the acting player their own eat/drink sound. The
//! final 16-particle burst and the finishing sound in `Consumable.onConsume` are
//! server-driven (the client's `updateUsingItem` skips `completeUsingItem` on the client),
//! so they are not duplicated here.

use haste_audio::random::LegacyRandom;
use haste_core::RegistryIndex;
use haste_data::Consumable;

use super::UseAnimation;
use crate::item_particles::{spawn_item_particles, EntityPose, ItemParticleRequest};
use crate::sound::{PlayRequest, SoundPos, SoundRef, SoundRequest};

/// `SoundSource.PLAYERS`, the category `Player.getSoundSource` returns (the ordinal in
/// vanilla's `SoundSource` order, `crate::sound::SOURCES`).
const CATEGORY_PLAYERS: i32 = 7;

/// `Consumable.shouldEmitParticlesAndSounds`: past 21.875% of the use and every fourth
/// remaining tick.
pub fn should_emit(consume_ticks: i32, remaining: i32) -> bool {
    let item_used_for_ticks = consume_ticks - remaining;
    let wait_ticks_before_use_effects = (consume_ticks as f32 * 0.21875) as i32;
    item_used_for_ticks > wait_ticks_before_use_effects && remaining % 4 == 0
}

/// The consumable's per-tick effects: a `LivingEntity.getRandom()` the volume, pitch and
/// item-particle draws come from (vanilla seeds each entity's `RandomSource` with a
/// unique seed).
pub struct ConsumeEffects {
    random: LegacyRandom,
}

impl Default for ConsumeEffects {
    fn default() -> Self {
        Self {
            random: LegacyRandom::new(unseeded()),
        }
    }
}

impl ConsumeEffects {
    /// `Consumable.emitParticlesAndSounds` (the 5-particle call from `onUseTick`): the
    /// four random draws (always all of them, whatever the animation) give the eat volume
    /// and pitch or the drink pitch, then the item particles spawn (the component's
    /// `has_consume_particles`, vanilla `spawnItemParticles(stack, 5)`), then the
    /// component's sound plays at the player. `pose` carries the entity's position, eye
    /// height and rotations the particle math reads. `item` is the used stack's registry
    /// index; `None` (an empty stack) spawns no particles, like vanilla's empty-stack
    /// guard.
    pub fn emit(
        &mut self,
        consumable: &Consumable,
        item: Option<RegistryIndex>,
        pose: EntityPose,
        requests: &mut Vec<SoundRequest>,
        particles: &mut Vec<ItemParticleRequest>,
    ) {
        let eat_volume = if self.random.next(1) != 0 { 0.5 } else { 1.0 };
        let eat_pitch = 1.0 + 0.2 * (self.random.next_float() - self.random.next_float());
        let drink_pitch = 0.9 + self.random.next_float() * 0.1;
        // The volume and pitch rules follow the component's animation (the drink takes
        // the fixed volume and its own pitch range), whatever sound it names.
        let drink = UseAnimation::from_id(consumable.animation) == UseAnimation::Drink;
        let volume = if drink { 0.5 } else { eat_volume };
        let pitch = if drink { drink_pitch } else { eat_pitch };
        if consumable.has_consume_particles {
            if let Some(item) = item {
                spawn_item_particles(&mut self.random, item, 5, pose, particles);
            }
        }
        let seed = self.random.next_long();
        requests.push(SoundRequest::Play(PlayRequest {
            // `Consumable.sound`: an unknown id plays nothing and logs once (the sound
            // request channel drops ids `sounds.json` does not know).
            sound: SoundRef::Inline(consumable.sound.to_string()),
            category: CATEGORY_PLAYERS,
            pos: SoundPos::Fixed(pose.pos),
            volume,
            pitch,
            seed,
        }));
    }
}

/// A seed for an unseeded generator (vanilla's `RandomSource.create()` takes a unique seed
/// per instance): the clock's nanoseconds.
fn unseeded() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0x5eed_e470)
}

#[cfg(test)]
mod tests;
