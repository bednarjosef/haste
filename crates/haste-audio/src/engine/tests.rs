//! Tests of the silent engine (`crate::engine`).

use super::*;
use crate::sounds::SoundVariant;
use haste_core::Identifier;

fn variant(path: &str, attenuation_distance: u32) -> SoundVariant {
    variant_with(path, attenuation_distance, 1.0, 1.0)
}

fn variant_with(path: &str, attenuation_distance: u32, volume: f32, pitch: f32) -> SoundVariant {
    SoundVariant {
        id: Identifier::new("minecraft", path.trim_end_matches(".ogg")),
        path: path.to_string(),
        volume,
        pitch,
        weight: 1,
        stream: false,
        attenuation_distance,
        preload: false,
    }
}

/// Plays one sound of `event`, in play order, like the output stage does.
fn play_event(engine: &mut SilentEngine, variant: &SoundVariant, source: SoundSource, event: &str) {
    engine.play(variant, source, None, 1.0, 1.0, Arc::from(event));
}

/// Sum of all gains, as a cheap "everything is silent" check.
fn total_gains(engine: &SilentEngine) -> (f32, f32) {
    engine
        .gains()
        .fold((0.0, 0.0), |(l, r), (gl, gr)| (l + gl, r + gr))
}

#[test]
fn non_positional_sound_is_full_gain_and_centred() {
    let mut engine = SilentEngine::new();
    let sound = variant("sounds/dig/stone1.ogg", 16);
    play_event(
        &mut engine,
        &sound,
        SoundSource::Block,
        "minecraft:block.stone.break",
    );
    engine.tick();
    assert_eq!(engine.playing(), 1);
    // Full gain, centred: each channel gets gain / sqrt(2).
    let (l, r) = engine.gains().next().unwrap();
    assert!((l - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
    assert_eq!(l, r);
}

#[test]
fn master_volume_zero_silences_everything() {
    let mut engine = SilentEngine::new();
    engine.set_listener(DVec3::new(0.0, 0.0, 0.0), 0.0, 0.0);
    engine.play(
        &variant("sounds/a.ogg", 16),
        SoundSource::Block,
        Some(DVec3::new(2.0, 0.0, 0.0)),
        1.0,
        1.0,
        Arc::from("minecraft:a"),
    );
    engine.play(
        &variant("sounds/b.ogg", 16),
        SoundSource::Ui,
        None,
        1.0,
        1.0,
        Arc::from("minecraft:b"),
    );
    engine.set_volume(SoundSource::Master, 0.0);
    engine.tick();
    assert_eq!(engine.playing(), 2);
    let (l, r) = total_gains(&engine);
    assert_eq!((l, r), (0.0, 0.0));
}

#[test]
fn per_source_volume_scales_only_that_source() {
    let mut engine = SilentEngine::new();
    play_event(
        &mut engine,
        &variant("sounds/a.ogg", 16),
        SoundSource::Block,
        "minecraft:a",
    );
    play_event(
        &mut engine,
        &variant("sounds/b.ogg", 16),
        SoundSource::Ui,
        "minecraft:b",
    );
    engine.set_volume(SoundSource::Block, 0.25);
    engine.tick();
    let gains: Vec<(f32, f32)> = engine.gains().collect();
    let norm = |(l, r): (f32, f32)| (l * l + r * r).sqrt();
    assert!((norm(gains[0]) - 0.25).abs() < 1e-5);
    assert!((norm(gains[1]) - 1.0).abs() < 1e-5);
}

#[test]
fn playing_in_counts_per_source() {
    let mut engine = SilentEngine::new();
    play_event(
        &mut engine,
        &variant("sounds/a.ogg", 16),
        SoundSource::Block,
        "minecraft:a",
    );
    play_event(
        &mut engine,
        &variant("sounds/b.ogg", 16),
        SoundSource::Block,
        "minecraft:b",
    );
    play_event(
        &mut engine,
        &variant("sounds/c.ogg", 16),
        SoundSource::Music,
        "minecraft:c",
    );
    assert_eq!(engine.playing(), 3);
    assert_eq!(engine.playing_in(SoundSource::Block), 2);
    assert_eq!(engine.playing_in(SoundSource::Music), 1);
    assert_eq!(engine.playing_in(SoundSource::Ui), 0);
    // Master is not a source of its own: it scales the others.
    assert_eq!(engine.playing_in(SoundSource::Master), 0);
}

#[test]
fn is_event_playing_follows_the_event_not_the_source() {
    let mut engine = SilentEngine::new();
    play_event(
        &mut engine,
        &variant("sounds/dig/stone1.ogg", 16),
        SoundSource::Block,
        "minecraft:block.stone.break",
    );
    assert!(engine.is_event_playing("minecraft:block.stone.break"));
    // The same event without the namespace, the way `sounds.json` keys it.
    assert!(engine.is_event_playing("block.stone.break"));
    assert!(!engine.is_event_playing("minecraft:block.stone.place"));
    // A sound in another source does not answer for this event.
    play_event(
        &mut engine,
        &variant("sounds/music/game.ogg", 16),
        SoundSource::Music,
        "minecraft:music.game",
    );
    assert!(engine.is_event_playing("minecraft:block.stone.break"));
    assert!(engine.is_event_playing("minecraft:music.game"));
}

#[test]
fn stop_by_source() {
    let mut engine = SilentEngine::new();
    play_event(
        &mut engine,
        &variant("sounds/a.ogg", 16),
        SoundSource::Block,
        "minecraft:a",
    );
    play_event(
        &mut engine,
        &variant("sounds/b.ogg", 16),
        SoundSource::Ui,
        "minecraft:b",
    );
    play_event(
        &mut engine,
        &variant("sounds/c.ogg", 16),
        SoundSource::Block,
        "minecraft:c",
    );
    engine.stop(Some(SoundSource::Block), None);
    assert_eq!(engine.playing(), 1);
    engine.tick();
    // Only the Ui sound survives.
    assert!((total_gains(&engine).0).abs() > 0.0);
}

#[test]
fn stop_by_event_only() {
    let mut engine = SilentEngine::new();
    play_event(
        &mut engine,
        &variant("sounds/a.ogg", 16),
        SoundSource::Block,
        "minecraft:a",
    );
    play_event(
        &mut engine,
        &variant("sounds/b.ogg", 16),
        SoundSource::Block,
        "minecraft:b",
    );
    engine.stop(None, Some("minecraft:a"));
    assert_eq!(engine.playing(), 1);
    // The surviving sound is b's, not a's.
    assert!(engine.is_event_playing("minecraft:b"));
    assert!(!engine.is_event_playing("minecraft:a"));
}

#[test]
fn stop_by_event_matches_the_namespaced_name() {
    let mut engine = SilentEngine::new();
    play_event(
        &mut engine,
        &variant("sounds/dig/stone1.ogg", 16),
        SoundSource::Block,
        "minecraft:block.stone.break",
    );
    // The event name without the namespace, the way a stop packet's identifier or
    // sounds.json key spells it, stops the same sound.
    engine.stop(None, Some("block.stone.break"));
    assert_eq!(engine.playing(), 0);
}

#[test]
fn stop_by_source_and_event() {
    let mut engine = SilentEngine::new();
    play_event(
        &mut engine,
        &variant("sounds/a.ogg", 16),
        SoundSource::Block,
        "minecraft:a",
    );
    play_event(
        &mut engine,
        &variant("sounds/b.ogg", 16),
        SoundSource::Ui,
        "minecraft:a",
    );
    play_event(
        &mut engine,
        &variant("sounds/c.ogg", 16),
        SoundSource::Block,
        "minecraft:b",
    );
    // Both filters must match: the Block sound of event a goes, the Ui one of the same
    // event stays, as does the Block sound of the other event.
    engine.stop(Some(SoundSource::Block), Some("minecraft:a"));
    assert_eq!(engine.playing(), 2);
    assert_eq!(engine.playing_in(SoundSource::Ui), 1);
    assert_eq!(engine.playing_in(SoundSource::Block), 1);
    assert!(engine.is_event_playing("minecraft:a"), "the Ui a survives");
    assert!(engine.is_event_playing("minecraft:b"));
}

#[test]
fn stop_by_neither_stops_everything() {
    let mut engine = SilentEngine::new();
    play_event(
        &mut engine,
        &variant("sounds/a.ogg", 16),
        SoundSource::Block,
        "minecraft:a",
    );
    play_event(
        &mut engine,
        &variant("sounds/b.ogg", 16),
        SoundSource::Music,
        "minecraft:music.game",
    );
    engine.stop(None, None);
    assert_eq!(engine.playing(), 0);
}

#[test]
fn stop_by_unknown_event_stops_nothing() {
    let mut engine = SilentEngine::new();
    play_event(
        &mut engine,
        &variant("sounds/a.ogg", 16),
        SoundSource::Block,
        "minecraft:a",
    );
    engine.stop(None, Some("minecraft:not.a.real.event"));
    assert_eq!(engine.playing(), 1);
}

#[test]
fn positional_sound_out_of_range_after_listener_moves_is_silent() {
    let mut engine = SilentEngine::new();
    let sound = variant("sounds/dig/stone1.ogg", 16);
    engine.set_listener(DVec3::new(0.0, 0.0, 0.0), 0.0, 0.0);
    engine.play(
        &sound,
        SoundSource::Block,
        Some(DVec3::new(4.0, 0.0, 0.0)),
        1.0,
        1.0,
        Arc::from("minecraft:block.stone.break"),
    );
    engine.tick();
    let (l, r) = engine.gains().next().unwrap();
    assert!((l * l + r * r).sqrt() > 0.7, "in range should be loud");

    // Walk away: the same sound is now out of range.
    engine.set_listener(DVec3::new(1000.0, 0.0, 0.0), 0.0, 0.0);
    engine.tick();
    assert_eq!(engine.playing(), 1);
    let (l, r) = engine.gains().next().unwrap();
    assert_eq!((l, r), (0.0, 0.0));
}

#[test]
fn listener_move_scales_gain_without_replay() {
    let mut engine = SilentEngine::new();
    let sound = variant("sounds/a.ogg", 16);
    engine.set_listener(DVec3::ZERO, 0.0, 0.0);
    engine.play(
        &sound,
        SoundSource::Block,
        Some(DVec3::new(8.0, 0.0, 0.0)),
        1.0,
        1.0,
        Arc::from("minecraft:a"),
    );
    engine.tick();
    let (l, r) = engine.gains().next().unwrap();
    // Linear falloff with reference distance 1: 1 - (8 - 1)/(16 - 1) = 8/15.
    assert!(
        ((l * l + r * r).sqrt() - 8.0 / 15.0).abs() < 1e-5,
        "8 of 16 blocks"
    );
}

#[test]
fn volume_two_sound_is_audible_at_distance_24() {
    // volume 2.0 doubles the audible range to 32, so 24 blocks is still in range.
    let mut engine = SilentEngine::new();
    engine.set_listener(DVec3::ZERO, 0.0, 0.0);
    engine.play(
        &variant("sounds/a.ogg", 16),
        SoundSource::Block,
        Some(DVec3::new(24.0, 0.0, 0.0)),
        2.0,
        1.0,
        Arc::from("minecraft:a"),
    );
    engine.tick();
    let (l, r) = engine.gains().next().unwrap();
    // gain = min(2, 1) * (1 - (24 - 1)/(32 - 1)) = 8/31, split over both channels.
    let norm = (l * l + r * r).sqrt();
    assert!((norm - 8.0 / 31.0).abs() < 1e-5, "norm {norm}");
    assert!(l <= 1.0 && r <= 1.0);
}

#[test]
fn variant_volume_scales_the_gain() {
    // A variant with volume 0.5 played at volume 1.0 has half the gain of a
    // volume-1.0 variant at the same spot.
    let mut loud = SilentEngine::new();
    loud.set_listener(DVec3::ZERO, 0.0, 0.0);
    loud.play(
        &variant_with("sounds/a.ogg", 16, 1.0, 1.0),
        SoundSource::Block,
        Some(DVec3::new(4.0, 0.0, 0.0)),
        1.0,
        1.0,
        Arc::from("minecraft:a"),
    );
    loud.tick();
    let mut quiet = SilentEngine::new();
    quiet.set_listener(DVec3::ZERO, 0.0, 0.0);
    quiet.play(
        &variant_with("sounds/a.ogg", 16, 0.5, 1.0),
        SoundSource::Block,
        Some(DVec3::new(4.0, 0.0, 0.0)),
        1.0,
        1.0,
        Arc::from("minecraft:a"),
    );
    quiet.tick();
    let norm = |e: &SilentEngine| {
        let (l, r) = e.gains().next().unwrap();
        (l * l + r * r).sqrt()
    };
    // Falloff 1 - (4 - 1)/(16 - 1) = 0.8 for the loud variant, half that for the quiet one.
    assert!((norm(&loud) - 0.8).abs() < 1e-5);
    assert!((norm(&quiet) - 0.4).abs() < 1e-5);
}

#[test]
fn variant_pitch_multiplies_and_clamps() {
    // Variant pitch 1.5 played at pitch 1.5: 2.25 clamps to 2.0.
    let mut engine = SilentEngine::new();
    engine.play(
        &variant_with("sounds/a.ogg", 16, 1.0, 1.5),
        SoundSource::Block,
        None,
        1.0,
        1.5,
        Arc::from("minecraft:a"),
    );
    assert_eq!(engine.pitches().next().unwrap(), 2.0);
}

#[test]
fn remove_drops_the_sound_at_the_index() {
    let mut engine = SilentEngine::new();
    for (index, path) in ["sounds/a.ogg", "sounds/b.ogg", "sounds/c.ogg"]
        .iter()
        .enumerate()
    {
        let event: Arc<str> = format!("minecraft:s{index}").into();
        engine.play(
            &variant(path, 16),
            SoundSource::Block,
            None,
            1.0,
            1.0,
            event,
        );
    }
    engine.remove(1);
    assert_eq!(engine.playing(), 2);
    // The first and third sounds survive with their gains.
    engine.tick();
    assert!(total_gains(&engine).0 > 0.0);
}

#[test]
fn zero_volume_sound_is_not_started() {
    // Vanilla's SoundEngine.play: a sound whose whole volume is 0 is skipped — it is not
    // tracked and never plays. Master 0, source volume 0 or a play volume of 0 all do it.
    let mut engine = SilentEngine::new();
    engine.set_volume(SoundSource::Master, 0.0);
    assert_eq!(
        engine.play(
            &variant("sounds/a.ogg", 16),
            SoundSource::Block,
            None,
            1.0,
            1.0,
            Arc::from("minecraft:a"),
        ),
        super::PlayResult::NotStarted
    );
    let mut engine = SilentEngine::new();
    assert_eq!(
        engine.play(
            &variant("sounds/a.ogg", 16),
            SoundSource::Block,
            None,
            0.0,
            1.0,
            Arc::from("minecraft:a"),
        ),
        super::PlayResult::NotStarted
    );
    // A variant volume of 0 counts too (volume * variant volume, clamped).
    assert_eq!(
        engine.play(
            &variant_with("sounds/b.ogg", 16, 0.0, 1.0),
            SoundSource::Block,
            None,
            1.0,
            1.0,
            Arc::from("minecraft:b"),
        ),
        super::PlayResult::NotStarted
    );
    assert_eq!(engine.playing(), 0);
}

#[test]
fn zero_volume_music_starts_silently() {
    // The music exception: a volume-0 music track starts (silently) and is tracked, so a
    // later volume change can raise it without restarting the track.
    let mut engine = SilentEngine::new();
    engine.set_volume(SoundSource::Music, 0.0);
    assert_eq!(
        engine.play(
            &variant("sounds/music/game/calm1.ogg", 16),
            SoundSource::Music,
            None,
            1.0,
            1.0,
            Arc::from("minecraft:music.game"),
        ),
        super::PlayResult::StartedSilently
    );
    assert_eq!(engine.playing(), 1);
    engine.tick();
    assert_eq!(engine.gains().next().unwrap(), (0.0, 0.0));
    // The volume coming back raises the same track, with no restart.
    engine.set_volume(SoundSource::Music, 1.0);
    engine.tick();
    assert_eq!(engine.playing(), 1);
    let (l, r) = engine.gains().next().unwrap();
    assert!((l - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5, "l {l}");
    assert!((r - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5, "r {r}");
}

#[test]
fn master_source_takes_the_master_volume_once() {
    // Options.getFinalSoundSourceVolume: the master source's own volume is the master
    // volume, every other source's is master * source, and SoundEngine clamps it to 0..=1.
    let mut volumes = SourceVolumes::default();
    volumes.set(SoundSource::Master, 0.5);
    volumes.set(SoundSource::Block, 0.5);
    assert_eq!(volumes.effective(SoundSource::Master), 0.5);
    assert_eq!(volumes.effective(SoundSource::Block), 0.25);
    volumes.set(SoundSource::Master, 2.0);
    volumes.set(SoundSource::Block, 1.0);
    assert_eq!(volumes.effective(SoundSource::Block), 1.0);
}
