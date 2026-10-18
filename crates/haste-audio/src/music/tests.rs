use super::*;

/// A no-op fade-gain sink for tests that do not drive the fade.
fn no_gain(_: f32) {}

/// Ticks with `is_playing` until the manager plays something; returns (ticks taken, event).
fn until_play(manager: &mut MusicManager, rng: &mut MusicRng, is_playing: bool) -> (usize, String) {
    for tick in 1..=60_000usize {
        match manager.tick(rng, is_playing, 1.0, no_gain) {
            Some(MusicCommand::Play(event)) => return (tick, event),
            Some(MusicCommand::Stop) => panic!("unexpected Stop"),
            None => {}
        }
    }
    panic!("no Play within 60000 ticks");
}

/// Ticks until the manager returns anything but `None`.
fn until_command(manager: &mut MusicManager, rng: &mut MusicRng, is_playing: bool) -> MusicCommand {
    for _ in 1..=60_000usize {
        if let Some(command) = manager.tick(rng, is_playing, 1.0, no_gain) {
            return command;
        }
    }
    panic!("no command within 60000 ticks");
}

#[test]
fn starts_with_vanillas_starting_delay() {
    // Vanilla's `nextSongDelay` starts at 100 ticks, so the first track of a session plays
    // within 5 seconds — not after a track's long random pause.
    let mut manager = MusicManager::new();
    let mut rng = MusicRng::new(1);
    let (ticks, event) = until_play(&mut manager, &mut rng, false);
    assert_eq!(event, "minecraft:music.menu");
    assert_eq!(
        ticks, STARTING_DELAY as usize,
        "the first track starts after the starting delay"
    );
}

#[test]
fn delays_follow_the_musics_constants() {
    // After a track ends, the next one follows the situation's own range (the `Musics`
    // constants), capped by the DEFAULT music frequency (20 minutes = 24000 ticks).
    let mut manager = MusicManager::new();
    manager.set_situation(MusicSituation::Game);
    let mut rng = MusicRng::new(2);
    let (_, event) = until_play(&mut manager, &mut rng, false);
    assert_eq!(event, "minecraft:music.game");
    // The track plays for a while and then ends.
    for _ in 0..100 {
        assert_eq!(manager.tick(&mut rng, true, 1.0, no_gain), None);
    }
    let (ticks, event) = until_play(&mut manager, &mut rng, false);
    assert_eq!(event, "minecraft:music.game");
    assert!(
        (12_000..=24_000).contains(&ticks),
        "game delay {ticks} outside 12000..=24000"
    );
}

#[test]
fn no_second_track_while_ours_plays() {
    let mut manager = MusicManager::new();
    manager.set_situation(MusicSituation::Game);
    let mut rng = MusicRng::new(3);
    let (_, event) = until_play(&mut manager, &mut rng, false);
    // while our track plays, no second one starts (vanilla's nextSongDelay stays at
    // Integer.MAX_VALUE while the current music is active)
    for _ in 0..60_000 {
        assert_eq!(manager.tick(&mut rng, true, 1.0, no_gain), None);
    }
    assert_eq!(manager.playing_event(), Some(event.as_str()));
}

#[test]
fn end_music_replaces_the_current_track() {
    // `Musics.END` replaces the current music: the playing track is stopped first.
    let mut manager = MusicManager::new();
    manager.set_situation(MusicSituation::Game);
    let mut rng = MusicRng::new(4);
    let (_, event) = until_play(&mut manager, &mut rng, false);
    assert_eq!(event, "minecraft:music.game");
    for _ in 0..10 {
        assert_eq!(manager.tick(&mut rng, true, 1.0, no_gain), None);
    }
    manager.set_situation(MusicSituation::End);
    assert_eq!(
        until_command(&mut manager, &mut rng, true),
        MusicCommand::Stop
    );
    // The replacement gets a short countdown (vanilla: nextInt(0, minDelay / 2)).
    let (ticks, event) = until_play(&mut manager, &mut rng, false);
    assert_eq!(event, "minecraft:music.end");
    assert!(
        ticks <= 3_000,
        "end music followed within minDelay/2 ticks, took {ticks}"
    );
}

#[test]
fn game_to_creative_keeps_current_track() {
    // `Musics.CREATIVE` does not replace the current music: the track keeps playing and the
    // next one comes with the in-game delays.
    let mut manager = MusicManager::new();
    manager.set_situation(MusicSituation::Game);
    let mut rng = MusicRng::new(5);
    let (_, event) = until_play(&mut manager, &mut rng, false);
    assert_eq!(event, "minecraft:music.game");
    for _ in 0..10 {
        assert_eq!(manager.tick(&mut rng, true, 1.0, no_gain), None);
    }
    manager.set_situation(MusicSituation::Creative);
    for _ in 0..100 {
        assert_eq!(manager.tick(&mut rng, true, 1.0, no_gain), None);
    }
    let (ticks, event) = until_play(&mut manager, &mut rng, false);
    assert_eq!(event, "minecraft:music.creative");
    assert!(
        (12_000..=24_000).contains(&ticks),
        "creative delay {ticks} outside 12000..=24000"
    );
}

#[test]
fn a_switching_biome_situation_caps_the_countdown() {
    // The situation's own maximum caps the running countdown every tick (vanilla's
    // `nextSongDelay = Math.min(nextSongDelay, music.maxDelay())`), so a track of a biome
    // with a short range starts within that range.
    let mut manager = MusicManager::new();
    manager.set_situation(MusicSituation::Game);
    let mut rng = MusicRng::new(6);
    assert_eq!(manager.tick(&mut rng, false, 1.0, no_gain), None);
    manager.set_situation(MusicSituation::Biome {
        event: "minecraft:music.cherry_grove".to_owned(),
        min_delay: 10,
        max_delay: 40,
        replace: false,
    });
    let (ticks, event) = until_play(&mut manager, &mut rng, false);
    assert_eq!(event, "minecraft:music.cherry_grove");
    assert!(
        ticks <= 40,
        "clamped countdown should fire within 40 ticks, took {ticks}"
    );
}

#[test]
fn dragon_and_credits_play_at_once_and_replace() {
    for situation in [MusicSituation::Dragon, MusicSituation::Credits] {
        let mut manager = MusicManager::new();
        manager.set_situation(situation.clone());
        let mut rng = MusicRng::new(7);
        assert_eq!(
            manager.tick(&mut rng, false, 1.0, no_gain),
            Some(MusicCommand::Play(situation.music().event)),
            "{situation:?} has no delay"
        );
    }
}

#[test]
fn fixed_seed_is_deterministic() {
    fn script(seed: u64) -> Vec<MusicCommand> {
        let mut manager = MusicManager::new();
        let mut rng = MusicRng::new(seed);
        let mut commands = Vec::new();
        let mut playing = false;
        manager.set_situation(MusicSituation::Game);
        for step in 0..30_000u32 {
            if step == 25_000 {
                manager.set_situation(MusicSituation::Menu);
            }
            match manager.tick(&mut rng, playing, 1.0, no_gain) {
                Some(command @ MusicCommand::Play(_)) => {
                    playing = true;
                    commands.push(command);
                }
                Some(command @ MusicCommand::Stop) => {
                    playing = false;
                    commands.push(command);
                }
                None => {
                    // The current track ends after a while.
                    if playing && step % 700 == 0 {
                        playing = false;
                    }
                }
            }
        }
        commands
    }
    let run = script(1234);
    assert!(!run.is_empty());
    assert_eq!(run, script(1234));
}

#[test]
fn the_track_event_is_followed_not_the_situation() {
    let mut manager = MusicManager::new();
    manager.set_situation(MusicSituation::Game);
    let mut rng = MusicRng::new(8);
    let (_, event) = until_play(&mut manager, &mut rng, false);
    assert_eq!(manager.playing_event(), Some(event.as_str()));
    for _ in 0..10 {
        assert_eq!(manager.tick(&mut rng, true, 1.0, no_gain), None);
    }
    assert_eq!(manager.playing_event(), Some(event.as_str()));
}

#[test]
fn the_fade_eases_the_gain_toward_the_target() {
    // Vanilla's fadePlaying: a falling gain shrinks by 3 % a tick toward the target,
    // and each new gain is handed to updateCategoryVolume.
    let mut manager = MusicManager::new();
    manager.set_situation(MusicSituation::Game);
    let mut rng = MusicRng::new(9);
    let (_, _) = until_play(&mut manager, &mut rng, false);
    let mut applied = Vec::new();
    let first = manager.tick(&mut rng, true, 0.5, |g| applied.push(g));
    assert_eq!(first, None);
    assert_eq!(applied.len(), 1, "one gain per fading tick");
    // 0.03 * 0.5 + 0.97 * 1.0 = 0.985.
    assert!((applied[0] - 0.985).abs() < 1e-6, "gain {}", applied[0]);
    // The gain keeps easing toward the target without overshooting.
    for expected in [0.97045, 0.956_336_5] {
        let g = manager.tick(&mut rng, true, 0.5, |g| {
            assert!((g - expected).abs() < 1e-4)
        });
        assert_eq!(g, None);
    }
    // Reaching the target stops the fade (no gain applied that tick).
    let mut quiet = 0;
    loop {
        quiet += 1;
        assert!(quiet < 10_000);
        let mut gain = None;
        let step = manager.tick(&mut rng, true, 0.5, |g| gain = Some(g));
        assert_eq!(step, None);
        if gain.is_none() {
            break;
        }
    }
}

#[test]
fn the_fade_to_zero_stops_the_track() {
    // fadePlaying at zero target: the gain sinks until it reaches 1e-4, then the
    // track stops (stopPlaying) and a fresh pause of get_nextSongDelay + 100 follows.
    let mut manager = MusicManager::new();
    manager.set_situation(MusicSituation::Game);
    let mut rng = MusicRng::new(10);
    let _ = until_play(&mut manager, &mut rng, false);
    let mut stopped = false;
    for _ in 0..2_000 {
        if manager.tick(&mut rng, true, 0.0, no_gain) == Some(MusicCommand::Stop) {
            stopped = true;
            break;
        }
    }
    assert!(stopped, "the fade reaches zero and stops the track");
    assert_eq!(manager.playing_event(), None);
    // The next track fades back in from the faded-out gain, rising by at most 0.005
    // a tick (fadePlaying's rising branch).
    let mut gains = Vec::new();
    let (ticks, event) = {
        let mut result = None;
        for tick in 1..=60_000usize {
            if let Some(MusicCommand::Play(event)) =
                manager.tick(&mut rng, false, 0.5, |g| gains.push(g))
            {
                result = Some((tick, event));
                break;
            }
        }
        result.expect("no Play within 60000 ticks")
    };
    assert_eq!(event, "minecraft:music.game");
    assert!(ticks > 100, "a fresh pause (+100) precedes the next track");
    assert!(
        gains.iter().all(|g| *g <= 0.005 + 1e-9),
        "the fade-in starts from the faded gain, creeping up"
    );
}

#[test]
fn no_fade_while_the_gain_matches_the_target() {
    // currentGain == volume: fadePlaying returns at once, no gain is applied.
    let mut manager = MusicManager::new();
    manager.set_situation(MusicSituation::Game);
    let mut rng = MusicRng::new(11);
    let _ = until_play(&mut manager, &mut rng, false);
    let mut applied = 0;
    for _ in 0..100 {
        manager.tick(&mut rng, true, 1.0, |_| applied += 1);
    }
    assert_eq!(applied, 0);
}
