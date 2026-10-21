//! Tests for the ticking sound instances: the underwater loop's fades and the sub sounds'
//! cut-offs against vanilla's numbers, and the list's play/hold/release/stop rules.

use super::*;
use crate::sound::test_output::RecordingOutput;
use crate::sound::under_liquid_ambient_sound_instance::{
    UnderLiquidAmbientSoundInstance, AMBIENT_UNDERWATER_LOOP, FADE_DURATION,
};
use crate::sound::under_liquid_sub_sound::UnderLiquidSubSound;

fn under(player: u32, under_water: bool) -> TickingPlayer {
    TickingPlayer {
        player,
        under_water,
    }
}

fn underwater_loop() -> TickableInstance {
    TickableInstance::UnderLiquidAmbient(UnderLiquidAmbientSoundInstance::underwater(0))
}

#[test]
fn underwater_loop_plays_looping_ambient_relative_at_volume_one() {
    let mut out = RecordingOutput::default();
    let mut ticking = TickingSounds::new(1);
    ticking.play(underwater_loop(), false, &mut out);
    let p = &out.played[0];
    assert_eq!(p.event, AMBIENT_UNDERWATER_LOOP);
    assert_eq!(p.source, SoundSource::Ambient);
    assert_eq!(p.pos, None);
    assert_eq!(p.volume, 1.0);
    assert!(p.looping);
}

#[test]
fn underwater_loop_fades_in_over_40_ticks_and_out_twice_as_fast() {
    let mut out = RecordingOutput::default();
    let mut ticking = TickingSounds::new(1);
    ticking.play(underwater_loop(), false, &mut out);
    let h = out.handle(0);
    // Under water: one fortieth per tick, capped at full.
    for t in 1..=45 {
        ticking.tick(&under(0, true), &mut out);
        let want = (t.min(FADE_DURATION) as f32 / 40.0).min(1.0);
        assert_eq!(out.volume(h), Some(want), "tick {t} under water");
    }
    // Out of water: two fortieths per tick from the cap; at 0 it is silent but still
    // playing, the tick after that takes the fade below 0, and the next one stops it.
    for t in 1..=20 {
        ticking.tick(&under(0, false), &mut out);
        assert_eq!(
            out.volume(h),
            Some((40 - 2 * t) as f32 / 40.0),
            "tick {t} out"
        );
    }
    ticking.tick(&under(0, false), &mut out);
    assert_eq!(out.volume(h), Some(0.0));
    assert!(out.stopped.is_empty());
    ticking.tick(&under(0, false), &mut out);
    assert_eq!(out.stopped, vec![h]);
    assert!(ticking.is_empty());
}

#[test]
fn underwater_loop_fades_back_in_when_reentering_in_time() {
    let mut out = RecordingOutput::default();
    let mut ticking = TickingSounds::new(1);
    ticking.play(underwater_loop(), false, &mut out);
    let h = out.handle(0);
    for _ in 0..10 {
        ticking.tick(&under(0, true), &mut out);
    }
    for _ in 0..3 {
        ticking.tick(&under(0, false), &mut out);
    }
    assert_eq!(out.volume(h), Some(4.0 / 40.0));
    ticking.tick(&under(0, true), &mut out);
    assert_eq!(out.volume(h), Some(5.0 / 40.0));
    assert!(out.stopped.is_empty());
}

#[test]
fn a_removed_player_stops_its_underwater_sounds() {
    let mut out = RecordingOutput::default();
    let mut ticking = TickingSounds::new(1);
    ticking.play(underwater_loop(), false, &mut out);
    let sub = UnderLiquidSubSound::new(0, "minecraft:ambient.underwater.loop.additions");
    ticking.play(TickableInstance::UnderLiquidSub(sub), false, &mut out);
    ticking.tick(&under(0, true), &mut out);
    assert!(out.stopped.is_empty());
    // A respawn's new player (generation 1): the old one is removed.
    ticking.tick(&under(1, true), &mut out);
    assert_eq!(out.stopped, vec![out.handle(0), out.handle(1)]);
    assert!(ticking.is_empty());
}

#[test]
fn sub_sounds_stop_on_surfacing_and_leave_when_played_through() {
    let mut out = RecordingOutput::default();
    let mut ticking = TickingSounds::new(1);
    let event = "minecraft:ambient.underwater.loop.additions.rare";
    let id = ticking.play(
        TickableInstance::UnderLiquidSub(UnderLiquidSubSound::new(0, event)),
        false,
        &mut out,
    );
    assert!(!out.played[0].looping);
    ticking.tick(&under(0, true), &mut out);
    assert!(ticking.is_playing(id));
    ticking.tick(&under(0, false), &mut out);
    assert_eq!(out.stopped, vec![out.handle(0)]);
    // A second one that plays through before the player surfaces: off the list, never
    // stopped.
    let id = ticking.play(
        TickableInstance::UnderLiquidSub(UnderLiquidSubSound::new(0, event)),
        false,
        &mut out,
    );
    out.finish(out.handle(1));
    ticking.tick(&under(0, true), &mut out);
    assert!(!ticking.is_playing(id));
    assert_eq!(out.stopped.len(), 1);
    assert!(ticking.is_empty());
}

#[test]
fn an_instance_that_did_not_start_never_ticks() {
    let mut out = RecordingOutput {
        refuse: true,
        ..Default::default()
    };
    let mut ticking = TickingSounds::new(1);
    let dropped = ticking.play(underwater_loop(), false, &mut out);
    assert!(ticking.get(dropped).is_none());
    // A held one stays reachable for its handler, silent and unticked.
    let held = ticking.play(underwater_loop(), true, &mut out);
    ticking.tick(&under(0, false), &mut out);
    ticking.tick(&under(0, false), &mut out);
    assert!(!ticking.get(held).unwrap().is_stopped());
    assert!(!ticking.is_playing(held));
    ticking.release(held);
    assert!(ticking.is_empty());
}

#[test]
fn stop_all_stops_the_sounds_but_not_the_instances() {
    let mut out = RecordingOutput::default();
    let mut ticking = TickingSounds::new(1);
    let held = ticking.play(underwater_loop(), true, &mut out);
    ticking.play(underwater_loop(), false, &mut out);
    ticking.request_stop_all();
    ticking.tick(&under(0, true), &mut out);
    assert_eq!(out.stopped, vec![out.handle(0), out.handle(1)]);
    // The held instance stays, unstopped and no longer ticking; the other is gone.
    assert_eq!(ticking.len(), 1);
    assert!(!ticking.get(held).unwrap().is_stopped());
    assert!(!ticking.is_playing(held));
    // The request is carried out once.
    ticking.tick(&under(0, true), &mut out);
    assert_eq!(out.stopped.len(), 2);
}

#[test]
fn every_instance_gets_its_own_unseeded_seed() {
    let mut out = RecordingOutput::default();
    let mut ticking = TickingSounds::new(99);
    ticking.play(underwater_loop(), false, &mut out);
    ticking.play(underwater_loop(), false, &mut out);
    let mut seeds = LegacyRandom::new(99);
    assert_eq!(out.played[0].seed, seeds.next_long());
    assert_eq!(out.played[1].seed, seeds.next_long());
}
