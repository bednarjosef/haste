//! Tests of the sound glue (`crate::sound`).

use super::*;
use haste_audio::output::AudioOutput;
use haste_protocol::packets::configuration::clientbound::RegistryEntry;

fn regs_with(names: &[&str]) -> SyncedRegistries {
    let mut regs = SyncedRegistries::default();
    let entries: Vec<RegistryEntry> = names
        .iter()
        .map(|n| RegistryEntry {
            id: Identifier::parse(n).unwrap(),
            data: None,
        })
        .collect();
    regs.insert(sound_registry(), entries);
    regs
}

fn inline(name: &str) -> IdOr<InlineSoundEvent> {
    IdOr::Inline(InlineSoundEvent {
        name: Identifier::parse(name).unwrap(),
        fixed_range: None,
    })
}

/// The real game data when the cache exists, for the name-table tests (ignored
/// otherwise, like `real_world_through_the_sink`).
fn static_game_data() -> std::sync::Arc<haste_data::GameData> {
    crate::net::load_game_data().expect("cache/datagen/26.3/reports")
}

/// The real assets when the cache is present, else `None` (the caller skips), the
/// way `haste-audio`'s output tests do: `SoundSystem` owns an `Assets` by value.
fn cached_assets() -> Option<haste_assets::Assets> {
    let jar = haste_core::paths::client_jar();
    if !jar.exists() {
        return None;
    }
    haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").ok()
}

/// One recorded play request of an inline event in `category`.
fn play_of(event: &str, category: i32) -> SoundRequest {
    SoundRequest::Play(PlayRequest {
        sound: SoundRef::Inline(event.to_owned()),
        category,
        pos: SoundPos::Fixed(DVec3::new(1.0, 2.0, 3.0)),
        volume: 1.0,
        pitch: 1.0,
        seed: 1,
    })
}

#[test]
fn category_index_maps_to_vanilla_source_order() {
    assert_eq!(sound_source(0), Some(SoundSource::Master));
    assert_eq!(sound_source(1), Some(SoundSource::Music));
    assert_eq!(sound_source(2), Some(SoundSource::Record));
    assert_eq!(sound_source(3), Some(SoundSource::Weather));
    assert_eq!(sound_source(4), Some(SoundSource::Block));
    assert_eq!(sound_source(5), Some(SoundSource::Hostile));
    assert_eq!(sound_source(6), Some(SoundSource::Neutral));
    assert_eq!(sound_source(7), Some(SoundSource::Player));
    assert_eq!(sound_source(8), Some(SoundSource::Ambient));
    assert_eq!(sound_source(9), Some(SoundSource::Voice));
    assert_eq!(sound_source(10), Some(SoundSource::Ui));
    assert_eq!(sound_source(11), None);
    assert_eq!(sound_source(-1), None);
}

#[test]
fn registry_id_resolves_through_the_name_table() {
    let names = vec![
        "minecraft:ambient.cave".to_owned(),
        "minecraft:block.stone.break".to_owned(),
    ];
    assert_eq!(
        resolve_event(&SoundRef::Id(0), &names),
        Some("minecraft:ambient.cave".to_owned())
    );
    assert_eq!(
        resolve_event(&SoundRef::Id(1), &names),
        Some("minecraft:block.stone.break".to_owned())
    );
    // Unknown or out-of-range ids play nothing.
    assert_eq!(resolve_event(&SoundRef::Id(2), &names), None);
    assert_eq!(resolve_event(&SoundRef::Id(-1), &names), None);
}

#[test]
fn inline_event_uses_its_own_name() {
    let names = vec![];
    assert_eq!(
        resolve_event(
            &SoundRef::Inline("minecraft:custom.mod_sound".to_owned()),
            &names
        ),
        Some("minecraft:custom.mod_sound".to_owned())
    );
}

#[test]
#[ignore = "needs cache/ (data generator reports)"]
fn name_table_prefers_synced_entries_over_the_static_registry() {
    let regs = regs_with(&["minecraft:from.server"]);
    let names = sound_event_names(&regs, &static_game_data());
    assert_eq!(names, vec!["minecraft:from.server".to_owned()]);
    // Without a synced registry the static one (data generator reports) is used.
    let names = sound_event_names(&SyncedRegistries::default(), &static_game_data());
    assert!(!names.is_empty(), "static sound_event registry is empty");
    assert!(names.iter().all(|n| n.contains(':')));
}

#[test]
fn position_coordinates_are_divided_by_eight() {
    let p = sound_position(800, -240, 88);
    assert!((p.x - 100.0).abs() < 1e-9);
    assert!((p.y + 30.0).abs() < 1e-9);
    assert!((p.z - 11.0).abs() < 1e-9);
}

#[test]
fn situation_follows_game_mode_and_water() {
    assert_eq!(
        music_situation(GameMode::Survival, false),
        MusicSituation::Game
    );
    assert_eq!(
        music_situation(GameMode::Creative, false),
        MusicSituation::Creative
    );
    // Water wins over creative, as in vanilla's underwater music check.
    assert_eq!(
        music_situation(GameMode::Creative, true),
        MusicSituation::UnderWater
    );
    assert_eq!(
        music_situation(GameMode::Adventure, false),
        MusicSituation::Game
    );
}

#[test]
fn sound_packet_records_sound_position_and_fields() {
    use haste_protocol::packets::play::clientbound as cb;
    let p = ClientboundPlay::Sound(cb::Sound {
        sound: IdOr::Id(3),
        category: 4,
        x: 160,
        y: 512,
        z: -88,
        volume: 0.8,
        pitch: 1.2,
        seed: 42,
    });
    assert_eq!(
        record(&p),
        Some(SoundRequest::Play(PlayRequest {
            sound: SoundRef::Id(3),
            category: 4,
            pos: SoundPos::Fixed(DVec3::new(20.0, 64.0, -11.0)),
            volume: 0.8,
            pitch: 1.2,
            seed: 42,
        }))
    );
}

#[test]
fn sound_entity_packet_records_the_entity() {
    use haste_protocol::packets::play::clientbound as cb;
    let p = ClientboundPlay::SoundEntity(cb::SoundEntity {
        sound: inline("minecraft:entity.zombie.ambient"),
        category: 5,
        entity_id: 77,
        volume: 1.0,
        pitch: 0.9,
        seed: 7,
    });
    assert_eq!(
        record(&p),
        Some(SoundRequest::Play(PlayRequest {
            sound: SoundRef::Inline("minecraft:entity.zombie.ambient".to_owned()),
            category: 5,
            pos: SoundPos::Entity(77),
            volume: 1.0,
            pitch: 0.9,
            seed: 7,
        }))
    );
}

#[test]
fn stop_packet_records_category_and_name() {
    use haste_protocol::packets::play::clientbound as cb;
    let p = ClientboundPlay::StopSound(cb::StopSound {
        source: Some(1),
        sound: Some(Identifier::minecraft("music.game")),
    });
    assert_eq!(
        record(&p),
        Some(SoundRequest::Stop(StopRequest {
            source: Some(1),
            sound: Some("minecraft:music.game".to_owned()),
        }))
    );
    let p = ClientboundPlay::StopSound(cb::StopSound {
        source: None,
        sound: None,
    });
    assert_eq!(
        record(&p),
        Some(SoundRequest::Stop(StopRequest {
            source: None,
            sound: None,
        }))
    );
}

#[test]
fn a_block_sound_is_not_the_music_track_playing() {
    let Some(assets) = cached_assets() else {
        // No downloaded assets in this environment; skip like the audio tests.
        return;
    };
    let events = SoundEvents::parse(
        br#"{"block.stone.break": {"sounds": ["dig/stone1"]},
             "music.game": {"sounds": ["music/game/calm1"]}}"#,
    )
    .unwrap();
    let mut sounds = Sounds::with_system(
        SoundSystem::with_output(events.clone(), assets, AudioOutput::silent()),
        events,
        Vec::new(),
    );
    // The music track this client started is running.
    sounds.music_event = Some("minecraft:music.game".to_owned());
    // A block sound arrives while it plays: it must not be mistaken for the track,
    // so no Music-source sound counts as playing.
    let mut requests = vec![play_of("minecraft:block.stone.break", 4)];
    sounds.apply(&mut requests, |_| None);
    assert_eq!(sounds.system.playing(), 1);
    assert_eq!(sounds.system.playing_in(SoundSource::Block), 1);
    assert_eq!(sounds.system.playing_in(SoundSource::Music), 0);
    assert!(!sounds.music_playing());
    // The music event itself counts.
    let mut requests = vec![play_of("minecraft:music.game", 1)];
    sounds.apply(&mut requests, |_| None);
    assert!(sounds.music_playing());
    assert_eq!(sounds.system.playing_in(SoundSource::Music), 1);
}

/// A stone sound type for the own-sound rule tests.
fn stone_type() -> haste_data::BlockSoundType {
    haste_data::BlockSoundType {
        break_sound: "minecraft:block.stone.break".to_owned(),
        step_sound: "minecraft:block.stone.step".to_owned(),
        place_sound: "minecraft:block.stone.place".to_owned(),
        hit_sound: "minecraft:block.stone.hit".to_owned(),
        fall_sound: "minecraft:block.stone.fall".to_owned(),
        volume: 1.0,
        pitch: 1.0,
    }
}

fn step_request(r: &SoundRequest) -> (&str, f32, f32) {
    match r {
        SoundRequest::Play(p) => match &p.sound {
            SoundRef::Inline(name) => (name, p.volume, p.pitch),
            other => panic!("not an inline event: {other:?}"),
        },
        other => panic!("not a play: {other:?}"),
    }
}

/// Vanilla's step-distance rule (`Entity.applyMovementEmissionAndPlaySound`):
/// the walked distance accumulates times 0.6 and a step plays when it passes
/// `nextStep` = `(int)moveDist + 1`. Walking 0.2 blocks per tick: the first
/// step fires when the accumulated 0.12-per-tick sum passes 1.0 (tick 9), the
/// next when it passes 2.0 (tick 17) — about every 1.67 blocks walked.
#[test]
fn step_distance_rule_fires_every_full_block_walked() {
    let mut own = OwnBlockSounds::new();
    let stone = stone_type();
    let mut fired = 0;
    for tick in 0..40 {
        // 0.2 blocks per tick of horizontal movement.
        let mut requests = Vec::new();
        own.after_move(
            DVec3::new(0.2, 0.0, 0.0),
            DVec3::ZERO,
            Some(&stone),
            &mut requests,
        );
        if !requests.is_empty() {
            fired += 1;
            // Each firing is exactly one step sound at the stone step event,
            // volume 1.0 * 0.15, pitch 1.0 (vanilla's factors).
            assert_eq!(requests.len(), 1, "tick {tick}");
            let (name, volume, pitch) = step_request(&requests[0]);
            assert_eq!(name, "minecraft:block.stone.step");
            assert!((volume - 0.15).abs() < 1e-6);
            assert!((pitch - 1.0).abs() < 1e-6);
        }
    }
    // 40 ticks * 0.2 blocks = 8 blocks = 4.8 accumulated: 4 steps (after 1.0, 2.0,
    // 3.0, 4.0), not one per tick and not zero.
    assert_eq!(fired, 4, "steps fired");
}

/// On air (no supporting block) no step plays, but the walked distance still
/// accumulates (vanilla's `moveDist` counts it; only the firing needs support):
/// landing after walking far over air fires one step per crossed block boundary,
/// in quick succession — exactly what vanilla does.
#[test]
fn no_step_over_air_then_boundaries_on_landing() {
    let mut own = OwnBlockSounds::new();
    let stone = stone_type();
    // Walk 10 blocks over air: nothing fires, nextStep stays 1.0.
    for _ in 0..50 {
        let mut requests = Vec::new();
        own.after_move(DVec3::new(0.2, 0.0, 0.0), DVec3::ZERO, None, &mut requests);
        assert!(requests.is_empty());
    }
    assert!((own.next_step - 1.0).abs() < 1e-6);
    // Land: the first tick (md 6.12) crosses boundary 1 and the accumulator
    // catches nextStep up to 7; then 7.02 and 8.04 fire in the 20 ticks.
    let mut fired = 0;
    for _ in 0..20 {
        let mut requests = Vec::new();
        own.after_move(
            DVec3::new(0.2, 0.0, 0.0),
            DVec3::ZERO,
            Some(&stone),
            &mut requests,
        );
        fired += requests.len();
    }
    assert_eq!(fired, 3, "20 ticks on stone after 10 over air");
}

/// A teleport resets the accumulator (vanilla's `nextStep` starts at 1.0), so
/// the first step after one comes a full block of walking later.
#[test]
fn teleport_resets_the_step_accumulator() {
    let mut own = OwnBlockSounds::new();
    let stone = stone_type();
    // Walk to md 0.6 (no fire yet) and teleport.
    for _ in 0..5 {
        let mut requests = Vec::new();
        own.after_move(
            DVec3::new(0.2, 0.0, 0.0),
            DVec3::ZERO,
            Some(&stone),
            &mut requests,
        );
        assert!(requests.is_empty());
    }
    own.reset_steps();
    assert!((own.next_step - 1.0).abs() < 1e-6);
    assert!((own.move_dist - 0.0).abs() < 1e-6);
    // 5 more ticks (md 0.6): still no step; the 9th would have fired without
    // the reset (md 1.08 > 1.0).
    let mut requests = Vec::new();
    for _ in 0..5 {
        own.after_move(
            DVec3::new(0.2, 0.0, 0.0),
            DVec3::ZERO,
            Some(&stone),
            &mut requests,
        );
    }
    assert!(
        requests.is_empty(),
        "no step within half a block of the teleport"
    );
    let mut fired = 0;
    for _ in 0..5 {
        let mut requests = Vec::new();
        own.after_move(
            DVec3::new(0.2, 0.0, 0.0),
            DVec3::ZERO,
            Some(&stone),
            &mut requests,
        );
        fired += requests.len();
    }
    assert_eq!(fired, 1, "the step comes a block after the teleport");
}

/// The mining hit's volume/pitch factors (`ClientLevel.playBreakingSound`:
/// (v+1)/8 and p*0.5) and the break's (`LevelEventHandler` case 2001:
/// (v+1)/2 and p*0.8). A loud metal type (volume 1.0, pitch 1.5) tells the
/// factors apart.
#[test]
fn mining_hit_and_break_volume_pitch_factors() {
    let mut own = OwnBlockSounds::new();
    let mut metal = stone_type();
    metal.volume = 1.0;
    metal.pitch = 1.5;
    let mut requests = Vec::new();
    own.mining_hit(DVec3::ZERO, &metal, &mut requests);
    own.block_broken(DVec3::ZERO, &metal, &mut requests);
    assert_eq!(requests.len(), 2);
    let (_, hit_v, hit_p) = step_request(&requests[0]);
    assert!((hit_v - 2.0 / 8.0).abs() < 1e-6, "hit volume {hit_v}");
    assert!((hit_p - 0.75).abs() < 1e-6, "hit pitch {hit_p}");
    let (_, break_v, break_p) = step_request(&requests[1]);
    assert!((break_v - 1.0).abs() < 1e-6, "break volume {break_v}");
    assert!((break_p - 1.2).abs() < 1e-6, "break pitch {break_p}");
}

/// A server `level_event` plays its sound at the block centre (vanilla
/// `Level.playLocalSound(BlockPos, ...)`): 2001 the break sound with
/// `(v+1)/2`, `p*0.8`, 2020 the hit sound with `(v+1)/8`, `p*0.5`. Any other
/// event is silent, and `intentionally_empty` plays nothing.
#[test]
fn level_events_play_the_block_sound_at_its_centre() {
    let pos = haste_core::BlockPos::new(10, 64, -5);
    let mut metal = stone_type();
    metal.volume = 1.0;
    metal.pitch = 1.5;
    let mut requests = Vec::new();
    level_event_sound(2020, pos, &metal, &mut requests);
    level_event_sound(2001, pos, &metal, &mut requests);
    level_event_sound(2019, pos, &metal, &mut requests);
    assert_eq!(requests.len(), 2, "only 2020 and 2001 play a sound");
    let (hit_name, hit_v, hit_p) = step_request(&requests[0]);
    assert_eq!(hit_name, "minecraft:block.stone.hit");
    assert!((hit_v - 2.0 / 8.0).abs() < 1e-6, "hit volume {hit_v}");
    assert!((hit_p - 0.75).abs() < 1e-6, "hit pitch {hit_p}");
    let (break_name, break_v, break_p) = step_request(&requests[1]);
    assert_eq!(break_name, "minecraft:block.stone.break");
    assert!((break_v - 1.0).abs() < 1e-6, "break volume {break_v}");
    assert!((break_p - 1.2).abs() < 1e-6, "break pitch {break_p}");
    // The position is the block centre, as vanilla's playLocalSound(BlockPos).
    match &requests[0] {
        SoundRequest::Play(p) => match p.pos {
            SoundPos::Fixed(p) => {
                assert_eq!(p, DVec3::new(10.5, 64.5, -4.5));
            }
            _ => panic!("not a fixed position"),
        },
        other => panic!("not a play: {other:?}"),
    }
    // A `intentionally_empty` break event plays nothing.
    let mut quiet = stone_type();
    quiet.break_sound = "minecraft:intentionally_empty".to_owned();
    let mut requests = Vec::new();
    level_event_sound(2001, pos, &quiet, &mut requests);
    assert!(requests.is_empty());
}

/// The place (`BlockItem.place`: (v+1)/2, p*0.8) and fall
/// (`LivingEntity.playBlockFallSound`: v*0.5, p*0.75) factors, and vanilla's
/// "no sound" event (`intentionally_empty`) plays nothing.
#[test]
fn place_and_fall_factors_and_the_empty_event() {
    let mut own = OwnBlockSounds::new();
    let mut wool = stone_type();
    wool.volume = 0.8;
    wool.pitch = 1.2;
    let mut requests = Vec::new();
    own.block_placed(DVec3::ZERO, &wool, &mut requests);
    own.landed_hard(DVec3::ZERO, &wool, &mut requests);
    assert_eq!(requests.len(), 2);
    let (_, place_v, place_p) = step_request(&requests[0]);
    assert!((place_v - 0.9).abs() < 1e-6, "place volume {place_v}");
    assert!((place_p - 0.96).abs() < 1e-6, "place pitch {place_p}");
    let (_, fall_v, fall_p) = step_request(&requests[1]);
    assert!((fall_v - 0.4).abs() < 1e-6, "fall volume {fall_v}");
    assert!((fall_p - 0.9).abs() < 1e-6, "fall pitch {fall_p}");
    // A sound type whose event is intentionally_empty (RED_SHRUB's step) plays
    // nothing rather than a silent event.
    let mut quiet = stone_type();
    quiet.step_sound = "minecraft:intentionally_empty".to_owned();
    let mut own = OwnBlockSounds::new();
    let mut requests = Vec::new();
    own.after_move(
        DVec3::new(0.2, 0.0, 0.0),
        DVec3::ZERO,
        Some(&quiet),
        &mut requests,
    );
    assert!(requests.is_empty());
}

#[test]
fn stop_by_event_stops_only_that_event() {
    let Some(assets) = cached_assets() else {
        return;
    };
    let events = SoundEvents::parse(
        br#"{"block.stone.break": {"sounds": ["dig/stone1"]},
             "music.game": {"sounds": ["music/game/calm1"]}}"#,
    )
    .unwrap();
    let mut sounds = Sounds::with_system(
        SoundSystem::with_output(events.clone(), assets, AudioOutput::silent()),
        events,
        Vec::new(),
    );
    sounds.music_event = Some("minecraft:music.game".to_owned());
    let mut requests = vec![
        play_of("minecraft:block.stone.break", 4),
        play_of("minecraft:music.game", 1),
    ];
    sounds.apply(&mut requests, |_| None);
    assert_eq!(sounds.system.playing(), 2);
    // `minecraft:stop_sound` with both fields, as the packet sends them.
    let stop = StopRequest {
        source: Some(1),
        sound: Some("minecraft:music.game".to_owned()),
    };
    sounds.stop(stop);
    assert_eq!(sounds.system.playing(), 1);
    assert!(sounds
        .system
        .is_event_playing("minecraft:block.stone.break"));
    assert!(!sounds.music_playing());
    // An event that is not playing stops nothing, so no event name can stop more
    // than it should.
    let stop = StopRequest {
        source: Some(4),
        sound: Some("minecraft:entity.zombie.ambient".to_owned()),
    };
    sounds.stop(stop);
    assert_eq!(sounds.system.playing(), 1);
}

#[test]
fn the_listener_is_the_camera_position_and_look() {
    // The gains follow the camera handed to `Sounds::tick` (vanilla's
    // `SoundEngine.updateSource(Camera)`), not the player's eye: a sound left of where
    // the camera looks is louder on the left channel, and the camera's position drives
    // the distance.
    let Some(assets) = cached_assets() else {
        return;
    };
    let events = SoundEvents::parse(br#"{"e": {"sounds": ["dig/stone1"]}}"#).unwrap();
    let mut sounds = Sounds::with_system(
        SoundSystem::with_output(events.clone(), assets, AudioOutput::silent()),
        events,
        Vec::new(),
    );
    // The camera at the origin facing south (+Z): west (-X) is its right.
    let mut requests = vec![play_of("minecraft:e", 4)];
    // The sound goes where the packet put it, at (1, 2, 3): east of the camera, so the
    // left channel is the louder one.
    sounds.apply(&mut requests, |_| None);
    sounds.tick(DVec3::ZERO, 0.0, 0.0, MusicSituation::Game);
    let (l, r) = sounds.system.engine_gains().next().unwrap();
    assert!(
        l > r,
        "east of a south-looking camera is on the left ({l} vs {r})"
    );
    // Turning the camera west makes the same sound right of the listener.
    sounds.tick(DVec3::ZERO, 90.0, 0.0, MusicSituation::Game);
    let (l, r) = sounds.system.engine_gains().next().unwrap();
    assert!(
        r > l,
        "east of a west-looking camera is on the right ({r} vs {l})"
    );
    // Moving the camera away silences it by distance.
    sounds.tick(DVec3::new(1000.0, 0.0, 0.0), 0.0, 0.0, MusicSituation::Game);
    let (l, r) = sounds.system.engine_gains().next().unwrap();
    assert_eq!((l, r), (0.0, 0.0));
}
