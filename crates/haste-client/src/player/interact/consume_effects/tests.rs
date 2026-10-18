//! Tests for the consumable's per-tick sound (`Consumable.emitParticlesAndSounds`).

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
