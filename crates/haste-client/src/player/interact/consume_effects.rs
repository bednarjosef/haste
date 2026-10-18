//! The per-tick effects of a consumable being used (player workstream): vanilla
//! `ItemStack.onUseTick` -> `Consumable.shouldEmitParticlesAndSounds` /
//! `Consumable.emitParticlesAndSounds`, driven from `LivingEntity.updateUsingItem` every
//! game tick while a `minecraft:consumable` item is in use.
//!
//! The client plays its own sound: `Player.playSound` passes the player as the excluded
//! listener, so the server never sends the acting player their own eat/drink sound. The
//! final 16-particle burst and the finishing sound in `Consumable.onConsume` are
//! server-driven (the client's `updateUsingItem` skips `completeUsingItem` on the client),
//! so they are not duplicated here.
//!
//! The item particles the consume spawns (`user.spawnItemParticles`) have no spawner in the
//! renderer yet (the `minecraft:item` particle type has no atlas sprites and no item-stack
//! option is decoded); `emit` reports whether vanilla would spawn them.

use glam::DVec3;
use haste_audio::random::LegacyRandom;
use haste_core::Identifier;

use super::UseAnimation;
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

/// The consumable's per-tick sound state: a `LivingEntity.getRandom()` the volume and pitch
/// draws come from (vanilla seeds each entity's `RandomSource` with a unique seed).
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
    /// `Consumable.emitParticlesAndSounds`'s sound half: vanilla's four draws (always all of
    /// them, whatever the animation) give the eat volume and pitch or the drink pitch, then
    /// the sound plays at the player. Returns whether vanilla would also spawn the consume
    /// particles (`hasConsumeParticles`, true for the eat animation).
    pub fn emit(
        &mut self,
        animation: UseAnimation,
        item: Option<&Identifier>,
        pos: DVec3,
        requests: &mut Vec<SoundRequest>,
    ) -> bool {
        let eat_volume = if self.random.next(1) != 0 { 0.5 } else { 1.0 };
        let eat_pitch = 1.0 + 0.2 * (self.random.next_float() - self.random.next_float());
        let drink_pitch = 0.9 + self.random.next_float() * 0.1;
        let drink = animation == UseAnimation::Drink;
        let volume = if drink { 0.5 } else { eat_volume };
        let pitch = if drink { drink_pitch } else { eat_pitch };
        // `Consumables.defaultFood` has consume particles, `defaultDrink` does not; the
        // component data the client decodes carries neither `sound` nor
        // `has_consume_particles`, so the animation decides and the honey bottle's one
        // sound override is named here.
        let particles = !drink;
        let event = if drink {
            if item.is_some_and(|i| i.path() == "honey_bottle") {
                "minecraft:item.honey_bottle.drink"
            } else {
                "minecraft:entity.generic.drink"
            }
        } else {
            "minecraft:entity.generic.eat"
        };
        let seed = self.random.next_long();
        requests.push(SoundRequest::Play(PlayRequest {
            sound: SoundRef::Inline(event.to_owned()),
            category: CATEGORY_PLAYERS,
            pos: SoundPos::Fixed(pos),
            volume,
            pitch,
            seed,
        }));
        particles
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
