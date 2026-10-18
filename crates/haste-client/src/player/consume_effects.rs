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

use super::interact::UseAnimation;
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
mod tests {
    use super::*;

    /// A bread's `minecraft:consumable` duration (`Consumable.consumeTicks`, 1.6 s).
    const BREAD_TICKS: i32 = 32;

    #[test]
    fn emits_only_past_the_wait_and_on_every_fourth_remaining_tick() {
        // waitTicksBeforeUseEffects = (int)(32 * 0.21875) = 7: itemUsedForTicks > 7 and
        // remaining % 4 == 0. The start tick (remaining 32) and the first ticks are silent.
        assert!(!should_emit(BREAD_TICKS, 32), "start tick");
        assert!(!should_emit(BREAD_TICKS, 28), "used 4 <= 7");
        assert!(!should_emit(BREAD_TICKS, 25), "used 7 <= 7");
        assert!(should_emit(BREAD_TICKS, 24), "used 8 > 7 and 24 % 4 == 0");
        // Remaining ticks that are not multiples of 4 never emit.
        for remaining in 0..=32 {
            let used = BREAD_TICKS - remaining;
            let wait = (BREAD_TICKS as f32 * 0.21875) as i32;
            assert_eq!(
                should_emit(BREAD_TICKS, remaining),
                used > wait && remaining % 4 == 0,
                "remaining {remaining}"
            );
        }
        // Every emit during one bread use: remaining 24, 20, 16, 12, 8, 4 (six sounds over
        // the 32-tick use; `remaining` 0 is the formula's answer but vanilla's
        // `updateUsingItem` only reaches it for a tick if the server's flag lags).
        let emitted: Vec<i32> = (0..=BREAD_TICKS)
            .filter(|&r| should_emit(BREAD_TICKS, r))
            .collect();
        assert_eq!(emitted, [0, 4, 8, 12, 16, 20, 24]);
    }

    #[test]
    fn a_zero_duration_consumable_never_emits() {
        // consumeTicks 0: itemUsedForTicks is 0, wait is 0, 0 > 0 is false.
        for remaining in 0..=4 {
            assert!(!should_emit(0, remaining), "remaining {remaining}");
        }
    }

    #[test]
    fn eat_volume_and_pitch_are_vanilla_ranges() {
        // eatVolume is 0.5 or 1.0; eatPitch = 1.0 + 0.2*(f - f) is within [0.8, 1.2].
        let mut fx = ConsumeEffects::default();
        let pos = DVec3::ZERO;
        for _ in 0..1000 {
            let mut reqs = Vec::new();
            let particles = fx.emit(UseAnimation::Eat, None, pos, &mut reqs);
            assert!(particles, "eat spawns consume particles");
            let SoundRequest::Play(p) = &reqs[0] else {
                panic!("not a play request");
            };
            assert!(
                p.volume == 0.5 || p.volume == 1.0,
                "eat volume {}",
                p.volume
            );
            assert!((0.8..=1.2).contains(&p.pitch), "eat pitch {}", p.pitch);
            assert_eq!(
                p.sound,
                SoundRef::Inline("minecraft:entity.generic.eat".into())
            );
            assert_eq!(p.category, CATEGORY_PLAYERS);
        }
    }

    #[test]
    fn drink_volume_and_pitch_are_vanilla_ranges() {
        // Drink volume is always 0.5; drinkPitch = 0.9 + f*0.1 is within [0.9, 1.0).
        let mut fx = ConsumeEffects::default();
        let pos = DVec3::ZERO;
        let mut saw_honey = false;
        for _ in 0..1000 {
            let mut reqs = Vec::new();
            let particles = fx.emit(UseAnimation::Drink, None, pos, &mut reqs);
            assert!(!particles, "drink spawns no consume particles");
            let SoundRequest::Play(p) = &reqs[0] else {
                panic!("not a play request");
            };
            assert_eq!(p.volume, 0.5);
            assert!((0.9..1.0).contains(&p.pitch), "drink pitch {}", p.pitch);
            assert_eq!(
                p.sound,
                SoundRef::Inline("minecraft:entity.generic.drink".into())
            );
            // The honey bottle's one override.
            let mut honey = Vec::new();
            fx.emit(
                UseAnimation::Drink,
                Some(&Identifier::minecraft("honey_bottle")),
                pos,
                &mut honey,
            );
            let SoundRequest::Play(h) = &honey[0] else {
                panic!("not a play request");
            };
            if h.sound == SoundRef::Inline("minecraft:item.honey_bottle.drink".into()) {
                saw_honey = true;
            }
        }
        assert!(saw_honey, "the honey bottle names its own drink sound");
    }
}
