//! Tests for `UnderwaterAmbientSoundHandler`: vanilla's chances and the one roll per tick.

use glam::DVec3;

use super::*;
use crate::sound::test_output::RecordingOutput;

fn player(under_water: bool) -> AmbientPlayer {
    AmbientPlayer {
        pos: DVec3::new(0.5, 60.0, 0.5),
        eye_y: 61.62,
        under_water,
        spectator: false,
    }
}

#[test]
fn the_rarest_addition_wins_below_each_chance() {
    assert_eq!(pick_addition(0.0), Some(ADDITIONS_ULTRA_RARE));
    assert_eq!(pick_addition(0.000_099_99), Some(ADDITIONS_ULTRA_RARE));
    assert_eq!(pick_addition(1.0E-4), Some(ADDITIONS_RARE));
    assert_eq!(pick_addition(0.000_999_9), Some(ADDITIONS_RARE));
    assert_eq!(pick_addition(0.001), Some(ADDITIONS));
    assert_eq!(pick_addition(0.009_999), Some(ADDITIONS));
    assert_eq!(pick_addition(0.01), None);
    assert_eq!(pick_addition(0.5), None);
    // Vanilla's constants.
    assert_eq!(CHANCE_PER_TICK, 0.01);
    assert_eq!(RARE_CHANCE_PER_TICK, 0.001);
    assert_eq!(ULTRA_RARE_CHANCE_PER_TICK, 1.0E-4);
    assert_eq!(MINIMUM_TICK_DELAY, 0);
}

#[test]
fn rolls_the_level_random_once_per_tick_under_water_only() {
    let mut handler = UnderwaterAmbientSoundHandler::default();
    let mut random = LegacyRandom::new(5);
    let mut mirror = random.clone();
    let mut ticking = TickingSounds::new(1);
    let mut out = RecordingOutput::default();
    // Out of the water nothing is drawn and nothing plays.
    for _ in 0..50 {
        handler.tick(0, &player(false), &mut random, &mut ticking, &mut out);
    }
    assert_eq!(random.next_long(), mirror.next_long());
    // Under water: exactly one float per tick, and the picks follow it.
    let mut expected = Vec::new();
    for _ in 0..20_000 {
        handler.tick(0, &player(true), &mut random, &mut ticking, &mut out);
        if let Some(event) = pick_addition(mirror.next_float()) {
            expected.push(event);
        }
    }
    assert_eq!(out.events(), expected);
    // Roughly one in a hundred ticks plays something.
    assert!((150..=250).contains(&expected.len()), "{}", expected.len());
    assert!(out.played.iter().all(|p| !p.looping && p.pos.is_none()));
}
