//! Tests for the consumable's per-tick sound and item particles
//! (`Consumable.emitParticlesAndSounds`).

use super::*;
use glam::DVec3;
use haste_core::Identifier;
use haste_data::Consumable;

/// A pose helper: the feet position, the eye height and the look.
fn pose(pos: DVec3, eye_y: f64, look: (f32, f32)) -> EntityPose {
    EntityPose { pos, eye_y, look }
}

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
    // Every emit during one bread use: remaining 24, 20, 16, 12, 8, 4 (six emits over
    // the 32-tick use, each 5 item particles and one sound; the 16-particle burst and
    // the finishing sound on the last tick are `Consumable.onConsume`'s, which the
    // server drives). `remaining` 0 is the formula's answer but vanilla's
    // `updateUsingItem` only reaches it for a tick if the server's flag lags.
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
        let mut particles = Vec::new();
        fx.emit(
            &Consumable::default_food(),
            Some(RegistryIndex(3)),
            pose(pos, 1.62, (0.0, 0.0)),
            &mut reqs,
            &mut particles,
        );
        assert_eq!(
            particles.len(),
            5,
            "five crumbs per emit (onUseTick's count)"
        );
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
    for _ in 0..1000 {
        let mut reqs = Vec::new();
        let mut particles = Vec::new();
        fx.emit(
            &Consumable::default_drink(),
            Some(RegistryIndex(3)),
            pose(pos, 1.62, (0.0, 0.0)),
            &mut reqs,
            &mut particles,
        );
        assert!(particles.is_empty(), "the default drink has no particles");
        let SoundRequest::Play(p) = &reqs[0] else {
            panic!("not a play request");
        };
        assert_eq!(p.volume, 0.5);
        assert!((0.9..1.0).contains(&p.pitch), "drink pitch {}", p.pitch);
        assert_eq!(
            p.sound,
            SoundRef::Inline("minecraft:entity.generic.drink".into())
        );
    }
}

/// The honey bottle: a drink whose component names its own sound
/// (`Consumables.HONEY_BOTTLE`) — the sound comes from the component, never the item name.
#[test]
fn the_sound_comes_from_the_component_not_the_item() {
    let mut fx = ConsumeEffects::default();
    let honey = Consumable {
        consume_seconds: 2.0,
        sound: Identifier::minecraft("item.honey_bottle.drink"),
        ..Consumable::default_drink()
    };
    let mut reqs = Vec::new();
    let mut particles = Vec::new();
    fx.emit(
        &honey,
        Some(RegistryIndex(3)),
        pose(DVec3::ZERO, 1.62, (0.0, 0.0)),
        &mut reqs,
        &mut particles,
    );
    let SoundRequest::Play(p) = &reqs[0] else {
        panic!("not a play request");
    };
    assert_eq!(
        p.sound,
        SoundRef::Inline("minecraft:item.honey_bottle.drink".into())
    );
    // Still a drink: volume 0.5, no particles.
    assert_eq!(p.volume, 0.5);
    assert!(particles.is_empty());
}

/// A stack whose component disables particles: no crumbs, the sound still plays (an
/// unknown sound id plays nothing and logs once on the sound channel, not here).
#[test]
fn a_component_without_particles_spawns_none() {
    let mut fx = ConsumeEffects::default();
    let silent = Consumable {
        has_consume_particles: false,
        ..Consumable::default_food()
    };
    let mut reqs = Vec::new();
    let mut particles = Vec::new();
    fx.emit(
        &silent,
        Some(RegistryIndex(3)),
        pose(DVec3::ZERO, 1.62, (0.0, 0.0)),
        &mut reqs,
        &mut particles,
    );
    assert!(particles.is_empty());
    assert_eq!(reqs.len(), 1);
    // An unknown modded sound id still goes out as a request (dropped by the sound
    // channel, logged once there).
    let modded = Consumable {
        sound: Identifier::new("mod", "custom_eat"),
        ..Consumable::default_food()
    };
    let mut reqs = Vec::new();
    fx.emit(
        &modded,
        Some(RegistryIndex(3)),
        pose(DVec3::ZERO, 1.62, (0.0, 0.0)),
        &mut reqs,
        &mut particles,
    );
    let SoundRequest::Play(p) = &reqs[0] else {
        panic!("not a play request");
    };
    assert_eq!(p.sound, SoundRef::Inline("mod:custom_eat".into()));
    assert_eq!(p.category, CATEGORY_PLAYERS);
    assert_eq!(p.pos, SoundPos::Fixed(DVec3::ZERO));
}

#[test]
fn crumbs_sit_in_front_of_the_mouth_at_the_eye() {
    let mut fx = ConsumeEffects::default();
    let pos = DVec3::new(10.0, 64.0, -5.0);
    let eye_y = 65.62;
    for _ in 0..200 {
        let mut reqs = Vec::new();
        let mut particles = Vec::new();
        fx.emit(
            &Consumable::default_food(),
            Some(RegistryIndex(3)),
            pose(pos, eye_y, (0.0, 0.0)),
            &mut reqs,
            &mut particles,
        );
        assert_eq!(particles.len(), 5);
        for r in &particles {
            assert_eq!(r.item, RegistryIndex(3));
            // 0.6 in front on z (yaw 0 and pitch 0 leave the offset unrotated), x
            // within +-0.15 and y 0.3..0.9 below the eye.
            assert!((r.pos.z - (-5.0 + 0.6)).abs() < 1e-6, "z {:?}", r.pos);
            assert!((r.pos.x - 10.0).abs() <= 0.15 + 1e-6, "x {:?}", r.pos);
            let below = eye_y - r.pos.y;
            assert!(
                (0.3..=0.9).contains(&below),
                "y {:?} below the eye {below}",
                r.pos
            );
            // `d` with the +0.05 lift: y in [0.15, 0.25], x within +-0.05, z 0.
            assert!((0.15..=0.25).contains(&r.vel[1]), "vy {:?}", r.vel);
            assert!(r.vel[0].abs() <= 0.05 + 1e-6, "vx {:?}", r.vel);
            assert_eq!(r.vel[2], 0.0);
        }
    }
}

/// Vanilla's `Vec3.xRot`/`yRot` of the offsets: looking straight down (pitch 90) puts
/// the crumbs 0.6 below the eye, looking up (pitch -90) 0.6 above.
#[test]
fn crumbs_follow_the_look() {
    let mut fx = ConsumeEffects::default();
    let pos = DVec3::new(0.0, 64.0, 0.0);
    for (pitch, dy) in [(90.0f32, -0.6f64), (-90.0, 0.6)] {
        let mut reqs = Vec::new();
        let mut particles = Vec::new();
        fx.emit(
            &Consumable::default_food(),
            Some(RegistryIndex(3)),
            pose(pos, 65.62, (0.0, pitch)),
            &mut reqs,
            &mut particles,
        );
        assert_eq!(particles.len(), 5);
        for r in &particles {
            // The 0.6 z offset rotated to y exactly; the y offset becomes z (the x
            // offset stays horizontal).
            assert!(
                (r.pos.y - (65.62 + dy)).abs() <= 1e-6,
                "y {:?} (pitch {pitch})",
                r.pos
            );
        }
    }
}

/// An empty stack spawns no particles (vanilla's `spawnItemParticles` empty guard); the
/// sound still plays.
#[test]
fn an_empty_stack_spawns_no_particles() {
    let mut fx = ConsumeEffects::default();
    let mut reqs = Vec::new();
    let mut particles = Vec::new();
    fx.emit(
        &Consumable::default_food(),
        None,
        pose(DVec3::ZERO, 1.62, (0.0, 0.0)),
        &mut reqs,
        &mut particles,
    );
    assert!(particles.is_empty());
    assert_eq!(reqs.len(), 1);
}
