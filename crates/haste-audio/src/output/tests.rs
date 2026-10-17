//! Tests of the audio output and the `SoundSystem` facade (`crate::output`).

use super::*;
use crate::engine::SoundSource;
use crate::sounds::SoundEvents;
use glam::DVec3;

fn sound(samples: Vec<Sample>, channels: u16) -> DecodedSound {
    DecodedSound {
        samples: Arc::new(samples),
        channels,
        sample_rate: 44_100,
    }
}

#[test]
fn lru_cache_evicts_the_least_recently_used() {
    let mut cache = LruCache::new(2);
    cache.insert("a".into(), sound(vec![0.1], 1));
    cache.insert("b".into(), sound(vec![0.2], 1));
    assert_eq!(cache.paths(), vec!["a", "b"]);
    // Touch "a": "b" is now the least recently used and gets evicted by "c".
    assert!(cache.get("a").is_some());
    cache.insert("c".into(), sound(vec![0.3], 1));
    assert_eq!(cache.paths(), vec!["a", "c"]);
    assert!(cache.get("b").is_none());
    // Re-inserting an existing path refreshes it without duplicating.
    cache.insert("a".into(), sound(vec![0.4], 1));
    assert_eq!(cache.paths(), vec!["c", "a"]);
}

#[test]
fn cached_samples_average_channels_to_mono() {
    let decoded = sound(vec![0.2, 0.6, 0.4, 0.8], 2);
    let mut cached = CachedSamples::new(&decoded);
    assert_eq!(cached.channels().get(), 1);
    assert_eq!(cached.sample_rate().get(), 44_100);
    assert_eq!(cached.next(), Some(0.4));
    assert_eq!(cached.next(), Some(0.6));
    assert_eq!(cached.next(), None);
}

#[test]
fn decode_rejects_non_vorbis_bytes() {
    // A tiny valid Ogg Vorbis file cannot be produced without an encoder dependency
    // (rodio only decodes), so the decode-worker test covers the rejection path; the
    // `real_device_plays_harp` test below decodes a real Ogg from the assets.
    assert!(decode(b"not an ogg".to_vec(), "sounds/x.ogg").is_none());
}

#[test]
fn silent_output_accepts_every_call() {
    let output = AudioOutput::silent();
    assert!(!output.is_live());
    let mut drained = Vec::new();
    output.send(PlayRequest {
        id: 0,
        path: "sounds/dig/stone1.ogg".into(),
        assets: None,
        stream: false,
        gains: (1.0, 1.0),
        pitch: 1.0,
    });
    output.send(PlayRequest {
        id: 1,
        path: "sounds/x.ogg".into(),
        assets: None,
        stream: true,
        gains: (0.0, 0.0),
        pitch: 0.5,
    });
    output.drain_controls(&mut drained);
    assert!(drained.is_empty());
    output.send_stop(0);
}

/// The real assets when the cache is present, else `None` (the caller skips).
fn cached_assets() -> Option<haste_assets::Assets> {
    let jar = haste_core::paths::client_jar();
    if !jar.exists() {
        return None;
    }
    haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").ok()
}

#[test]
fn facade_plays_and_stops_through_the_silent_path() {
    let Some(assets) = cached_assets() else {
        // No downloaded assets in this environment; the silent path needs none of
        // them, but `SoundSystem` owns an `Assets` by value, so skip when absent.
        return;
    };
    let events = SoundEvents::parse(br#"{"e": {"sounds": ["dig/stone1"]}}"#).unwrap();
    let mut system = SoundSystem::with_output(events, assets, AudioOutput::silent());
    assert!(!system.output.is_live());
    system.set_listener(DVec3::ZERO, 0.0);
    system.set_volume(SoundSource::Master, 0.5);
    system.play(
        "e",
        SoundSource::Block,
        Some(DVec3::new(2.0, 0.0, 0.0)),
        1.0,
        1.0,
        7,
    );
    assert_eq!(system.playing(), 1);
    system.tick();
    system.stop(Some(SoundSource::Block), None);
    assert_eq!(system.playing(), 0);
    system.tick();
    assert_eq!(system.playing(), 0);
    // An unknown event does nothing.
    system.play("nope", SoundSource::Block, None, 1.0, 1.0, 1);
    assert_eq!(system.playing(), 0);
}

/// A system over the silent output, with a block event and a music event parsed.
fn silent_system(assets: haste_assets::Assets) -> SoundSystem {
    let events = SoundEvents::parse(
        br#"{"block.stone.break": {"sounds": ["dig/stone1"]},
             "music.game": {"sounds": ["music/game/calm1"]}}"#,
    )
    .unwrap();
    SoundSystem::with_output(events, assets, AudioOutput::silent())
}

#[test]
fn queries_answer_per_source_and_per_event() {
    let Some(assets) = cached_assets() else {
        return;
    };
    let mut system = silent_system(assets);
    system.play("block.stone.break", SoundSource::Block, None, 1.0, 1.0, 1);
    system.play("music.game", SoundSource::Music, None, 1.0, 1.0, 2);
    system.play("music.game", SoundSource::Music, None, 1.0, 1.0, 3);
    assert_eq!(system.playing(), 3);
    // The music is one of them, not the whole count: the block sound never makes it
    // 3.
    assert_eq!(system.playing_in(SoundSource::Music), 2);
    assert_eq!(system.playing_in(SoundSource::Block), 1);
    assert_eq!(system.playing_in(SoundSource::Ui), 0);
    assert!(system.is_event_playing("minecraft:music.game"));
    assert!(system.is_event_playing("block.stone.break"));
    assert!(!system.is_event_playing("minecraft:block.stone.place"));
    system.tick();
}

#[test]
fn stop_matches_the_event_name_not_the_variant_path() {
    let Some(assets) = cached_assets() else {
        return;
    };
    let mut system = silent_system(assets);
    system.play("block.stone.break", SoundSource::Block, None, 1.0, 1.0, 1);
    system.play("music.game", SoundSource::Music, None, 1.0, 1.0, 1);
    // Stop by event only: the other event's sounds keep playing.
    system.stop(None, Some("minecraft:block.stone.break"));
    assert_eq!(system.playing(), 1);
    assert!(system.is_event_playing("minecraft:music.game"));
    // Stop by source and event together, and by the namespace-defaulted name.
    system.play("block.stone.break", SoundSource::Ambient, None, 1.0, 1.0, 2);
    system.stop(Some(SoundSource::Ambient), Some("block.stone.break"));
    assert_eq!(system.playing(), 1);
    // An event that is not playing stops nothing rather than everything.
    system.stop(None, Some("minecraft:not.a.real.event"));
    assert_eq!(system.playing(), 1);
    // Both None stops everything, as vanilla's stop() with no filters does.
    system.stop(None, None);
    assert_eq!(system.playing(), 0);
}

#[test]
fn playing_an_event_again_does_not_allocate_a_new_name() {
    let Some(assets) = cached_assets() else {
        return;
    };
    let mut system = silent_system(assets);
    system.play("music.game", SoundSource::Music, None, 1.0, 1.0, 1);
    // The name is interned once: the second play shares it instead of building a
    // new string (the table has one entry per distinct event name).
    let interned = system.interned.len();
    system.play("music.game", SoundSource::Music, None, 1.0, 1.0, 2);
    assert_eq!(system.interned.len(), interned);
    assert_eq!(system.playing_in(SoundSource::Music), 2);
    system.play("block.stone.break", SoundSource::Block, None, 1.0, 1.0, 3);
    assert_eq!(system.interned.len(), interned + 1);
}

#[test]
fn quiet_tick_allocates_nothing() {
    let Some(assets) = cached_assets() else {
        return;
    };
    let events = SoundEvents::parse(br#"{"e": {"sounds": ["dig/stone1"]}}"#).unwrap();
    let mut system = SoundSystem::with_output(events, assets, AudioOutput::silent());
    system.play("e", SoundSource::Block, None, 1.0, 1.0, 1);
    system.play("e", SoundSource::Block, None, 1.0, 1.0, 2);
    system.play("e", SoundSource::Block, None, 1.0, 1.0, 3);
    system.tick();
    // After the first tick with nothing new started, the scratch buffers keep their
    // capacity: clear() + refill reuses the allocation instead of growing it.
    let caps = (
        system.incoming.capacity(),
        system.finished.capacity(),
        system.sounded.capacity(),
    );
    for _ in 0..10 {
        system.tick();
    }
    assert_eq!(
        (
            system.incoming.capacity(),
            system.finished.capacity(),
            system.sounded.capacity()
        ),
        caps
    );
    assert!(system.incoming.is_empty());
    assert!(system.finished.is_empty());
    assert_eq!(system.playing(), 3);
}

/// Opens the real device and plays `minecraft:block.note_block.harp` from the real
/// assets for 1 s. Run explicitly: `cargo test -p haste-audio -- --ignored`.
#[test]
#[ignore = "opens the real audio device and needs the downloaded assets in the cache"]
fn real_device_plays_harp() {
    let assets = cached_assets().expect("the downloaded assets in the cache");
    let mut assets_owner = assets;
    let bytes = assets_owner
        .read_asset("assets/minecraft/sounds.json")
        .unwrap();
    let events = SoundEvents::parse(&bytes).unwrap();
    let mut system = SoundSystem::new(events, assets_owner);
    assert!(system.output.is_live(), "no audio output device");
    system.play(
        "block.note_block.harp",
        SoundSource::Record,
        None,
        1.0,
        1.0,
        42,
    );
    assert_eq!(system.playing(), 1);
    std::thread::sleep(Duration::from_secs(1));
    system.tick();
    system.stop(None, None);
    assert_eq!(system.playing(), 0);
}
