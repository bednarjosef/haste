//! Tests of the sound glue (`crate::sound`).

use super::*;
use haste_audio::output::AudioOutput;
use haste_core::BlockStateId;
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

/// A fabricated step-rule block: state id 1 (non-air) with `sound`, at the cell
/// below the origin.
fn step_block<'a>(sound: Option<&'a haste_data::BlockSoundType>) -> super::steps::StepBlock<'a> {
    super::steps::StepBlock {
        pos: haste_core::BlockPos::new(0, -1, 0),
        state: BlockStateId(1),
        sound,
        climbable: false,
        crystal: false,
    }
}

/// A step context: `movement` this tick, on the ground by default, walking over a
/// block with `sound` (supporting = effect, the common case on a full block).
fn step_ctx<'a>(
    movement: DVec3,
    on_ground: bool,
    sound: Option<&'a haste_data::BlockSoundType>,
) -> super::steps::StepCtx<'a> {
    let block = step_block(sound);
    super::steps::StepCtx {
        player_pos: DVec3::ZERO,
        movement,
        velocity: DVec3::ZERO,
        on_ground,
        flying: false,
        discrete: false,
        crouching: false,
        swimming: false,
        in_water: false,
        supporting: step_block(sound),
        effect: block,
        primary: None,
    }
}

/// Vanilla's step-distance rule (`Entity.applyMovementEmissionAndPlaySound`):
/// the walked distance accumulates times 0.6 and a step plays when it passes
/// `nextStep` = `(int)moveDist + 1`. Walking 0.2 blocks per tick: the first
/// step fires when the accumulated 0.12-per-tick sum passes 1.0 (tick 9), the
/// next when it passes 2.0 (tick 17) — about every 1.67 blocks walked.
#[test]
fn step_distance_rule_fires_every_full_block_walked() {
    let mut steps = StepSounds::new();
    let stone = stone_type();
    let mut fired = 0;
    for tick in 0..40 {
        // 0.2 blocks per tick of horizontal movement.
        let mut requests = Vec::new();
        steps.after_move(
            &step_ctx(DVec3::new(0.2, 0.0, 0.0), true, Some(&stone)),
            &mut requests,
        );
        if !requests.is_empty() {
            fired += 1;
            // Each firing is exactly one step sound at the stone step event,
            // volume 1.0 * 0.15, pitch 1.0 (vanilla's factors), PLAYER source,
            // played at the player (vanilla plays its own sounds at itself).
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

/// `Player.getMovementEmission` is NONE while flying or sneaking on the ground:
/// no step sound ever, and the walked distance does not accumulate either, so
/// the steps that would have fired do not come back on landing.
#[test]
fn no_step_while_flying_or_sneaking_on_the_ground() {
    let stone = stone_type();
    for (name, flying, discrete) in [
        ("flying", true, false),
        ("sneaking on the ground", false, true),
    ] {
        let mut steps = StepSounds::new();
        let mut ctx = step_ctx(DVec3::new(0.2, 0.0, 0.0), true, Some(&stone));
        ctx.flying = flying;
        ctx.discrete = discrete;
        for _ in 0..40 {
            let mut requests = Vec::new();
            steps.after_move(&ctx, &mut requests);
            assert!(requests.is_empty(), "{name} played a step");
        }
        assert!(
            (steps.move_dist() - 0.0).abs() < 1e-6,
            "{name} accumulated distance"
        );
    }
}

/// `nextStep` only advances on a played step (`vibrationAndSoundEffectsFromBlock`
/// produced nothing): airborne over a block, with the gate closed (not on the
/// ground, not climbing, not crouching), the distance accumulates past `nextStep`
/// but the step never fires and `nextStep` stays — so the moment the gate opens
/// (landing), exactly one step plays, not one per crossed boundary.
#[test]
fn next_step_only_advances_on_a_played_step() {
    let mut steps = StepSounds::new();
    let stone = stone_type();
    // Airborne, not swimming: the gate is closed, no sound, no advance.
    for _ in 0..20 {
        let mut requests = Vec::new();
        steps.after_move(
            &step_ctx(DVec3::new(0.2, 0.0, 0.0), false, Some(&stone)),
            &mut requests,
        );
        assert!(requests.is_empty());
    }
    assert!(
        steps.move_dist() > steps.next_step(),
        "distance passed nextStep"
    );
    assert!((steps.next_step() - 1.0).abs() < 1e-6, "nextStep advanced");
    // Land: one step (the gate opens), and nextStep catches up.
    let mut requests = Vec::new();
    steps.after_move(
        &step_ctx(DVec3::new(0.2, 0.0, 0.0), true, Some(&stone)),
        &mut requests,
    );
    assert_eq!(requests.len(), 1, "one step on landing, not a burst");
    assert!((steps.next_step() - (steps.move_dist() as i32 as f32 + 1.0)).abs() < 1e-6);
}

/// On air (no supporting block) no step plays, but the walked distance still
/// accumulates (vanilla's `moveDist` counts it; only the firing needs support):
/// landing after walking far over air fires one step per crossed block boundary,
/// in quick succession — exactly what vanilla does.
#[test]
fn no_step_over_air_then_boundaries_on_landing() {
    let mut steps = StepSounds::new();
    let stone = stone_type();
    // Walk 10 blocks over air: nothing fires, nextStep stays 1.0.
    for _ in 0..50 {
        let mut requests = Vec::new();
        steps.after_move(
            &step_ctx(DVec3::new(0.2, 0.0, 0.0), false, None),
            &mut requests,
        );
        assert!(requests.is_empty());
    }
    assert!((steps.next_step() - 1.0).abs() < 1e-6);
    // Land: the first tick (md 6.12) crosses boundary 1 and the accumulator
    // catches nextStep up to 7; then 7.02 and 8.04 fire in the 20 ticks.
    let mut fired = 0;
    for _ in 0..20 {
        let mut requests = Vec::new();
        steps.after_move(
            &step_ctx(DVec3::new(0.2, 0.0, 0.0), true, Some(&stone)),
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
    let mut steps = StepSounds::new();
    let stone = stone_type();
    // Walk to md 0.6 (no fire yet) and teleport.
    for _ in 0..5 {
        let mut requests = Vec::new();
        steps.after_move(
            &step_ctx(DVec3::new(0.2, 0.0, 0.0), true, Some(&stone)),
            &mut requests,
        );
        assert!(requests.is_empty());
    }
    steps.reset();
    assert!((steps.next_step() - 1.0).abs() < 1e-6);
    assert!((steps.move_dist() - 0.0).abs() < 1e-6);
    // 5 more ticks (md 0.6): still no step; the 9th would have fired without
    // the reset (md 1.08 > 1.0).
    let mut requests = Vec::new();
    for _ in 0..5 {
        steps.after_move(
            &step_ctx(DVec3::new(0.2, 0.0, 0.0), true, Some(&stone)),
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
        steps.after_move(
            &step_ctx(DVec3::new(0.2, 0.0, 0.0), true, Some(&stone)),
            &mut requests,
        );
        fired += requests.len();
    }
    assert_eq!(fired, 1, "the step comes a block after the teleport");
}

/// `Player.playStepSound` in water: the swim sound (`Entity.waterSwimSound` ->
/// `playSwimSound`, `entity.player.swim`) whose volume follows the move's velocity
/// (sqrt(vx^2*0.2 + vy^2 + vz^2*0.2) * 0.35, capped at 1) with the wobbling pitch,
/// then a muffled step of the block (`Entity.playMuffledStepSound`: v*0.05, p*0.8) —
/// never the plain step.
#[test]
fn in_water_the_step_becomes_the_swim_sound_and_a_muffled_step() {
    let stone = stone_type();
    let mut steps = StepSounds::new();
    let mut ctx = step_ctx(DVec3::new(0.2, 0.0, 0.0), true, Some(&stone));
    ctx.in_water = true;
    ctx.velocity = DVec3::new(1.0, 2.0, 1.0);
    // Walk until the step fires.
    let mut fired = 0;
    for _ in 0..10 {
        let mut requests = Vec::new();
        steps.after_move(&ctx, &mut requests);
        if !requests.is_empty() {
            fired += 1;
            // The swim sound then the muffled stone step, no plain step.
            assert_eq!(requests.len(), 2);
            let (swim, volume, pitch) = step_request(&requests[0]);
            assert_eq!(swim, "minecraft:entity.player.swim");
            let speed = (1.0_f64 * 1.0 * 0.2 + 2.0 * 2.0 + 1.0 * 1.0 * 0.2).sqrt() * 0.35;
            assert!((volume - speed as f32).abs() < 1e-6, "swim volume {volume}");
            assert!((pitch - 1.0).abs() <= 0.4 + 1e-6, "swim pitch {pitch}");
            let (muffled, m_volume, m_pitch) = step_request(&requests[1]);
            assert_eq!(muffled, "minecraft:block.stone.step");
            assert!((m_volume - 0.05).abs() < 1e-6, "muffled volume {m_volume}");
            assert!((m_pitch - 0.8).abs() < 1e-6, "muffled pitch {m_pitch}");
        }
    }
    assert_eq!(fired, 1, "one step's worth of sounds in 10 ticks");
}

/// While sprint-swimming (`isSwimming`) the vibration gate closes, so no step plays;
/// the water branch of `applyMovementEmissionAndPlaySound` advances `nextStep` and
/// plays the swim sound alone.
#[test]
fn sprint_swimming_plays_only_the_swim_sound() {
    let stone = stone_type();
    let mut steps = StepSounds::new();
    let mut ctx = step_ctx(DVec3::new(0.2, 0.0, 0.0), false, Some(&stone));
    ctx.swimming = true;
    ctx.in_water = true;
    ctx.velocity = DVec3::new(0.2, 0.0, 0.0);
    let mut fired = 0;
    for _ in 0..20 {
        let mut requests = Vec::new();
        steps.after_move(&ctx, &mut requests);
        for r in &requests {
            let (name, _, _) = step_request(r);
            assert_eq!(
                name, "minecraft:entity.player.swim",
                "no step while swimming"
            );
        }
        fired += requests.len();
    }
    assert!(fired > 0, "the swim sound replaces the step in water");
    assert!(
        steps.next_step() > 1.0,
        "nextStep advanced through the water branch"
    );
}

/// `Player.getPrimaryStepSoundBlockPos` + `playCombinationStepSounds`: a carpet
/// (`#combination_step_sound_blocks`) above the effect block plays its own step
/// (v*0.15) over a muffled step of the block below (v*0.05, p*0.8); a block in
/// `#inside_step_sound_blocks` only (snow) plays its plain step instead of the
/// block below it.
#[test]
fn combination_and_inside_step_sound_blocks() {
    let mut wool = stone_type();
    wool.step_sound = "minecraft:block.wool.step".to_owned();
    let mut snow = stone_type();
    snow.step_sound = "minecraft:block.snow.step".to_owned();
    let stone = stone_type();
    // A carpet over stone: primary = the carpet (combination), effect = stone.
    let mut steps = StepSounds::new();
    let mut ctx = step_ctx(DVec3::new(0.2, 0.0, 0.0), true, Some(&stone));
    ctx.primary = Some(super::steps::PrimaryStep {
        block: step_block(Some(&wool)),
        combination: true,
    });
    let mut requests = Vec::new();
    for _ in 0..10 {
        steps.after_move(&ctx, &mut requests);
    }
    assert_eq!(
        requests.len(),
        2,
        "carpet: the wool step over the muffled stone"
    );
    let (name, volume, _) = step_request(&requests[0]);
    assert_eq!(name, "minecraft:block.wool.step");
    assert!((volume - 0.15).abs() < 1e-6, "carpet volume {volume}");
    let (name, volume, pitch) = step_request(&requests[1]);
    assert_eq!(name, "minecraft:block.stone.step");
    assert!(
        (volume - 0.05).abs() < 1e-6,
        "muffled stone volume {volume}"
    );
    assert!((pitch - 0.8).abs() < 1e-6, "muffled stone pitch {pitch}");
    // Snow over stone (`#inside_step_sound_blocks` only): the snow's plain step,
    // nothing muffled.
    let mut steps = StepSounds::new();
    let mut ctx = step_ctx(DVec3::new(0.2, 0.0, 0.0), true, Some(&stone));
    ctx.primary = Some(super::steps::PrimaryStep {
        block: step_block(Some(&snow)),
        combination: false,
    });
    let mut requests = Vec::new();
    for _ in 0..10 {
        steps.after_move(&ctx, &mut requests);
    }
    assert_eq!(requests.len(), 1, "snow: one plain step");
    let (name, volume, pitch) = step_request(&requests[0]);
    assert_eq!(name, "minecraft:block.snow.step");
    assert!((volume - 0.15).abs() < 1e-6);
    assert!((pitch - 1.0).abs() < 1e-6);
}

/// `Entity.walkingStepSound`'s amethyst chime (`shouldPlayAmethystStepSound`: the
/// block is in `#crystal_sound_blocks` and 20 ticks passed since the last): the
/// first chime comes with intensity 0.07 (volume 0.1 + 0.07 * 1.2), the next no
/// sooner than 20 ticks later, with the intensity decayed by 0.997 per tick and
/// raised again.
#[test]
fn amethyst_steps_chime_with_the_intensity_ramp() {
    let mut amethyst = stone_type();
    amethyst.step_sound = "minecraft:block.amethyst_block.step".to_owned();
    let mut steps = StepSounds::new();
    let mut block = step_block(Some(&amethyst));
    block.crystal = true;
    let mut ctx = step_ctx(DVec3::new(0.2, 0.0, 0.0), true, None);
    ctx.supporting = step_block(Some(&amethyst));
    ctx.effect = block;
    // Walk 55 ticks: steps fire at ticks 9, 17, 26, 34, 43, 51; the chime can
    // first play at tick 20 (>= 0 + 20), so the step at 26 chimes and the next
    // eligible one is at 51 (>= 26 + 20): the 34 and 43 steps stay silent.
    let mut chimes = 0;
    for _ in 0..55 {
        let mut requests = Vec::new();
        steps.after_move(&ctx, &mut requests);
        for r in &requests {
            let (name, volume, _) = step_request(r);
            if name == "minecraft:block.amethyst_block.chime" {
                chimes += 1;
                if chimes == 1 {
                    assert!(
                        (volume - (0.1 + 0.07 * 1.2)).abs() < 1e-6,
                        "first chime volume {volume}"
                    );
                } else if chimes == 2 {
                    let decayed = 0.07 * 0.997_f64.powi(AMETHYST_CHIME_TICK_GAP) as f32;
                    let intensity = decayed + 0.07;
                    assert!(
                        (volume - (0.1 + intensity * 1.2)).abs() < 1e-6,
                        "second chime volume {volume}"
                    );
                }
            }
        }
    }
    assert_eq!(chimes, 2, "chimes in 55 ticks");
}

/// The tick gap between the two chimes of the amethyst test (steps every ~8 ticks,
/// `tick >= last + 20`).
const AMETHYST_CHIME_TICK_GAP: i32 = 25;

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

/// A one-block-thick floor for the step-block resolver: `state` everywhere below
/// y = 0 in a 16x16 patch, `above` in the y = 0 cell (a snow layer or carpet to
/// stand on), air elsewhere.
struct StepFloor {
    state: BlockStateId,
    above: Option<BlockStateId>,
}

impl haste_core::view::BlockView for StepFloor {
    fn block_state(&self, pos: haste_core::BlockPos) -> BlockStateId {
        if pos.y == 0 {
            self.above.unwrap_or(haste_core::BlockStateId::AIR)
        } else if pos.y < 0 && pos.x.abs() <= 8 && pos.z.abs() <= 8 {
            self.state
        } else {
            haste_core::BlockStateId::AIR
        }
    }
    fn is_chunk_loaded(&self, _: haste_core::ChunkPos) -> bool {
        true
    }
    fn sky_light(&self, _: haste_core::BlockPos) -> u8 {
        15
    }
    fn block_light(&self, _: haste_core::BlockPos) -> u8 {
        0
    }
    fn min_y(&self) -> i32 {
        -64
    }
    fn height(&self) -> i32 {
        320
    }
}

/// A full cube for every non-air state (the resolver only asks whether the floor
/// block supports the box).
struct FullShapes;

impl haste_core::view::CollisionShapes for FullShapes {
    fn collision_boxes(&self, state: BlockStateId) -> &[haste_core::Aabb] {
        if state == haste_core::BlockStateId::AIR {
            &[]
        } else {
            const FULL: &[haste_core::Aabb] = &[haste_core::Aabb {
                min: DVec3::ZERO,
                max: DVec3::new(1.0, 1.0, 1.0),
            }];
            FULL
        }
    }
    fn outline_boxes(&self, _: BlockStateId) -> &[haste_core::Aabb] {
        &[]
    }
}

/// `resolve_step_blocks` against the real block table and synced tags: on a full
/// block the effect block is the supporting block; a snow layer above it
/// (`#inside_step_sound_blocks`) becomes the plain primary step, a carpet
/// (`#combination_step_sound_blocks`) the combination primary, and an amethyst
/// floor carries the crystal flag (`#crystal_sound_blocks`).
#[test]
#[ignore = "needs cache/ (data generator reports)"]
fn step_blocks_resolve_from_the_block_table_and_tags() {
    let data = static_game_data();
    let state = |name: &str| {
        data.blocks
            .default_state(&haste_core::Identifier::parse(name).unwrap())
            .unwrap()
    };
    let index = |name: &str| {
        data.blocks
            .block_index(&haste_core::Identifier::parse(name).unwrap())
            .unwrap()
            .0 as i32
    };
    let stone = state("minecraft:stone");
    let snow = state("minecraft:snow");
    let carpet = state("minecraft:white_carpet");
    let amethyst = state("minecraft:amethyst_block");
    let tags = haste_physics::mining::BlockTags::new(
        [
            ("inside_step_sound_blocks", vec![index("minecraft:snow")]),
            (
                "combination_step_sound_blocks",
                vec![index("minecraft:white_carpet")],
            ),
            (
                "crystal_sound_blocks",
                vec![index("minecraft:amethyst_block")],
            ),
        ]
        .map(|(t, e)| (haste_core::Identifier::minecraft(t), e)),
    );
    let aabb = haste_physics::PlayerState::at(DVec3::new(0.5, 1.0, 0.5))
        .aabb(&haste_physics::PlayerAttributes::default());

    // On a full stone floor (feet at y = 1): supporting = effect = the stone, no
    // primary.
    let floor = StepFloor {
        state: stone,
        above: None,
    };
    let (supporting, effect, primary) = resolve_step_blocks(
        DVec3::new(0.5, 1.0, 0.5),
        &aabb,
        true,
        &floor,
        &FullShapes,
        &data,
        Some(&tags),
    );
    assert_eq!(supporting.state, stone);
    assert_eq!(effect.state, stone);
    assert!(primary.is_none());
    assert!(!effect.crystal);

    // A snow layer on top (feet at its 0.125 top): the effect block is the stone
    // under it and the snow is the plain primary step.
    let floor = StepFloor {
        state: stone,
        above: Some(snow),
    };
    let (_, effect, primary) = resolve_step_blocks(
        DVec3::new(0.5, 0.125, 0.5),
        &aabb,
        true,
        &floor,
        &FullShapes,
        &data,
        Some(&tags),
    );
    assert_eq!(effect.state, stone);
    let primary = primary.expect("the snow layer is the primary step block");
    assert_eq!(primary.block.state, snow);
    assert!(!primary.combination);

    // A carpet on top (feet at its 0.0625 top): the combination primary.
    let floor = StepFloor {
        state: stone,
        above: Some(carpet),
    };
    let (_, effect, primary) = resolve_step_blocks(
        DVec3::new(0.5, 0.0625, 0.5),
        &aabb,
        true,
        &floor,
        &FullShapes,
        &data,
        Some(&tags),
    );
    assert_eq!(effect.state, stone);
    let primary = primary.expect("the carpet is the primary step block");
    assert_eq!(primary.block.state, carpet);
    assert!(primary.combination);

    // An amethyst floor: the crystal flag is set on the effect block.
    let floor = StepFloor {
        state: amethyst,
        above: None,
    };
    let (_, effect, _) = resolve_step_blocks(
        DVec3::new(0.5, 1.0, 0.5),
        &aabb,
        true,
        &floor,
        &FullShapes,
        &data,
        Some(&tags),
    );
    assert!(effect.crystal);
}

/// One landing's requests, decoded to (event name, volume, pitch).
fn landed_requests(
    fall_distance: f64,
    may_fly: bool,
    rule: FallBlockRule,
    landed_on: Option<LandedOn<'_>>,
) -> Vec<(String, f32, f32)> {
    let mut falls = FallSounds::new();
    let mut requests = Vec::new();
    falls.landed(
        DVec3::ZERO,
        fall_distance,
        may_fly,
        landed_on,
        rule,
        &mut requests,
    );
    requests
        .into_iter()
        .map(|r| match r {
            SoundRequest::Play(p) => match p.sound {
                SoundRef::Inline(name) => (name, p.volume, p.pitch),
                other => panic!("not an inline event: {other:?}"),
            },
            other => panic!("not a play: {other:?}"),
        })
        .collect()
}

/// A landing a stone-like block plays nothing below vanilla's safe fall distance
/// (`LivingEntity.calculateFallDamage`: `floor(fall + 1.0E-6 - 3.0)` <= 0), one small
/// fall sound and the block fall sound from damage 1 on, and the big fall sound only
/// above damage 4 (`LivingEntity.getFallDamageSound`).
#[test]
fn fall_sound_only_with_damage_small_then_big() {
    let stone = LandedOn {
        sound: &stone_type(),
    };
    // 3.5 blocks: damage floor(3.5 + 1e-6 - 3) = 0 — no sound at all.
    let requests = landed_requests(3.5, false, FallBlockRule::Default, Some(stone));
    assert!(
        requests.is_empty(),
        "no fall sound without damage: {requests:?}"
    );
    // 4.5 blocks: damage 1 — the small fall sound plus the block fall sound
    // (`block.stone.fall` at volume 1.0 * 0.5, pitch 1.0 * 0.75).
    let requests = landed_requests(4.5, false, FallBlockRule::Default, Some(stone));
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].0, "minecraft:entity.player.small_fall");
    assert!((requests[0].1 - 1.0).abs() < 1e-6);
    assert!((requests[0].2 - 1.0).abs() < 1e-6);
    assert_eq!(requests[1].0, "minecraft:block.stone.fall");
    assert!((requests[1].1 - 0.5).abs() < 1e-6);
    assert!((requests[1].2 - 0.75).abs() < 1e-6);
    // Damage exactly 4 (fall 7.0) is still small; damage 5 (fall 8.0) is the first big.
    let requests = landed_requests(7.0, false, FallBlockRule::Default, Some(stone));
    assert_eq!(requests[0].0, "minecraft:entity.player.small_fall");
    let requests = landed_requests(8.0, false, FallBlockRule::Default, Some(stone));
    assert_eq!(requests[0].0, "minecraft:entity.player.big_fall");
}

/// `Player.causeFallDamage`: with `mayfly` (creative and spectator players) a landing
/// plays no fall sound, whatever the distance.
#[test]
fn no_fall_sound_with_may_fly() {
    let stone = LandedOn {
        sound: &stone_type(),
    };
    let requests = landed_requests(20.0, true, FallBlockRule::Default, Some(stone));
    assert!(requests.is_empty(), "mayfly plays nothing: {requests:?}");
}

/// The block fall-damage rules (`Block.fallOn` overrides): hay and honey need
/// damage 13+ for a sound (modifier 0.2), honey plays its slide sound and the block
/// fall sound twice, slime and powder snow never take the damage path, powder snow
/// plays the small/big fall sound by distance alone, and a bed halves the distance.
#[test]
fn block_fall_damage_rules() {
    let on = LandedOn {
        sound: &stone_type(),
    };
    // Hay (`HayBlock.fallOn`, modifier 0.2): 4.5 blocks is damage 0 — silent.
    let requests = landed_requests(4.5, false, FallBlockRule::Hay, Some(on));
    assert!(
        requests.is_empty(),
        "hay silences small falls: {requests:?}"
    );
    // 13.5 blocks: damage floor(10.5 * 0.2) = 2 — sounds.
    let requests = landed_requests(13.5, false, FallBlockRule::Hay, Some(on));
    assert_eq!(requests.len(), 2, "hay at damage 2: {requests:?}");
    // Slime (`SlimeBlock.fallOn`, modifier 0.0): never a sound.
    let requests = landed_requests(50.0, false, FallBlockRule::Slime, Some(on));
    assert!(requests.is_empty(), "slime plays nothing: {requests:?}");
    // Honey (`HoneyBlock.fallOn`): slide sound always, damage chain at 0.2, and the
    // block fall sound twice (its own fallOn plus playBlockFallSound).
    let requests = landed_requests(4.5, false, FallBlockRule::Honey, Some(on));
    assert_eq!(requests.len(), 1, "honey at damage 0: {requests:?}");
    assert_eq!(requests[0].0, "minecraft:block.honey_block.slide");
    let requests = landed_requests(13.5, false, FallBlockRule::Honey, Some(on));
    assert_eq!(requests.len(), 4, "honey at damage 2: {requests:?}");
    assert_eq!(requests[0].0, "minecraft:block.honey_block.slide");
    assert_eq!(requests[1].0, "minecraft:entity.player.small_fall");
    assert_eq!(requests[2].0, "minecraft:block.stone.fall");
    assert_eq!(requests[3].0, "minecraft:block.stone.fall");
    // Powder snow (`PowderSnowBlock.fallOn`): no damage, small from 4, big from 7,
    // never the block fall sound.
    let requests = landed_requests(3.9, false, FallBlockRule::PowderSnow, Some(on));
    assert!(requests.is_empty(), "powder snow below 4: {requests:?}");
    let requests = landed_requests(5.0, false, FallBlockRule::PowderSnow, Some(on));
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, "minecraft:entity.player.small_fall");
    let requests = landed_requests(8.0, false, FallBlockRule::PowderSnow, Some(on));
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, "minecraft:entity.player.big_fall");
    // A bed (`BlockBehaviour.fallDistanceReduction` 0.5): 7.5 blocks halve to 3.75,
    // damage 0 — silent where stone would play.
    let requests = landed_requests(7.4, false, FallBlockRule::HalvesFallDistance, Some(on));
    assert!(requests.is_empty(), "bed halves the fall: {requests:?}");
}

/// `FallBlockRule::of` against the real block table: the four class-override blocks
/// by registry name, and beds through `#minecraft:beds` (which vanilla's tag provider
/// fills from the coloured bed list — the straw bed is not in it). Without the tags
/// nothing halves; unknown blocks take the default rule.
#[test]
#[ignore = "needs cache/ (data generator reports)"]
fn fall_block_rules_resolve_from_the_block_table() {
    let data = static_game_data();
    let state = |name: &str| {
        data.blocks
            .default_state(&haste_core::Identifier::parse(name).unwrap())
            .unwrap()
    };
    let of = |s: haste_core::BlockStateId| FallBlockRule::of(s, None, Some(&data.blocks));
    assert_eq!(of(state("minecraft:hay_block")), FallBlockRule::Hay);
    assert_eq!(of(state("minecraft:slime_block")), FallBlockRule::Slime);
    assert_eq!(of(state("minecraft:honey_block")), FallBlockRule::Honey);
    assert_eq!(
        of(state("minecraft:powder_snow")),
        FallBlockRule::PowderSnow
    );
    assert_eq!(of(state("minecraft:stone")), FallBlockRule::Default);
    // With the beds tag synced, a red bed halves and the straw bed does not.
    let beds = haste_physics::mining::BlockTags::new([(
        haste_core::Identifier::minecraft("beds"),
        vec![
            data.blocks
                .block_index(&haste_core::Identifier::parse("minecraft:red_bed").unwrap())
                .unwrap()
                .0 as i32,
        ],
    )]);
    let tagged = |name: &str| FallBlockRule::of(state(name), Some(&beds), Some(&data.blocks));
    assert_eq!(
        tagged("minecraft:red_bed"),
        FallBlockRule::HalvesFallDistance
    );
    assert_eq!(tagged("minecraft:straw_bed"), FallBlockRule::Default);
    // The shelf mushroom halves by name (not in the beds tag).
    assert_eq!(
        tagged("minecraft:shelf_mushroom"),
        FallBlockRule::HalvesFallDistance
    );
}

/// The place factors (`BlockItem.place`: (v+1)/2, p*0.8) and vanilla's
/// "no sound" event (`intentionally_empty`) plays nothing.
#[test]
fn place_factors_and_the_empty_event() {
    let mut own = OwnBlockSounds::new();
    let mut wool = stone_type();
    wool.volume = 0.8;
    wool.pitch = 1.2;
    let mut requests = Vec::new();
    own.block_placed(DVec3::ZERO, &wool, &mut requests);
    assert_eq!(requests.len(), 1);
    let (_, place_v, place_p) = step_request(&requests[0]);
    assert!((place_v - 0.9).abs() < 1e-6, "place volume {place_v}");
    assert!((place_p - 0.96).abs() < 1e-6, "place pitch {place_p}");
    // A sound type whose event is intentionally_empty (RED_SHRUB's step) plays
    // nothing rather than a silent event.
    let mut quiet = stone_type();
    quiet.step_sound = "minecraft:intentionally_empty".to_owned();
    let mut steps = StepSounds::new();
    let mut requests = Vec::new();
    steps.after_move(
        &step_ctx(DVec3::new(0.2, 0.0, 0.0), true, Some(&quiet)),
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
    // Turning the camera north (yaw 180) makes the same sound right of the listener:
    // east is a north-looking camera's right.
    sounds.tick(DVec3::ZERO, 180.0, 0.0, MusicSituation::Game);
    let (l, r) = sounds.system.engine_gains().next().unwrap();
    assert!(
        r > l,
        "east of a north-looking camera is on the right ({r} vs {l})"
    );
    // Moving the camera away silences it by distance.
    sounds.tick(DVec3::new(1000.0, 0.0, 0.0), 0.0, 0.0, MusicSituation::Game);
    let (l, r) = sounds.system.engine_gains().next().unwrap();
    assert_eq!((l, r), (0.0, 0.0));
}
