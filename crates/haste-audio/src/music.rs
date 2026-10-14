//! Background music scheduling: which music event fits the player's situation, and when to
//! play it. Pure logic — the engine turns the [`MusicCommand`]s into actual output.
//!
//! The rules are vanilla's `MusicManager` (net.minecraft.client.sounds) driven at 20 Hz from
//! `Minecraft.tick`:
//! - `nextSongDelay` starts at vanilla's `STARTING_DELAY` of 100 ticks, so the first track of
//!   a session plays within about 5 seconds; the delay is picked fresh, from the situation's
//!   own range, only after a track has ended (`MusicManager.MusicFrequency.getNextSongDelay`,
//!   DEFAULT = 20 minutes, which caps both ends of the range);
//! - the situation's `maxDelay` caps the running countdown every tick
//!   (`nextSongDelay = Math.min(nextSongDelay, music.maxDelay())`);
//! - a situation whose music replaces the current one stops the playing track and gets a short
//!   new countdown (`Mth.nextInt(random, 0, music.minDelay() / 2)`), as in vanilla's
//!   `canReplace` check;
//! - while a track plays, nothing new starts (vanilla's `startPlaying` sets
//!   `nextSongDelay = Integer.MAX_VALUE`).
//!
//! The situations map to vanilla's `Musics` constants (`Minecraft.getSituationalMusic`: the
//! screen's background music, the End's boss music, else the biome's `BackgroundMusic`
//! selected by `select(isCreative, isUnderwater)` — underwater first, then creative, then the
//! biome's default).

/// Vanilla's `STARTING_DELAY` (MusicManager): the countdown the manager starts with, so the
/// first track of a session plays within 100 ticks (5 s) rather than after a track's long
/// random pause.
pub const STARTING_DELAY: i32 = 100;

/// The `music_frequency` option's DEFAULT (`MusicFrequency.DEFAULT`: 20 minutes), in ticks:
/// the maximum any pause between two tracks can be.
const DEFAULT_MAX_FREQUENCY: u32 = 20 * 1200;

/// The music event and timing rules for one situation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicSpec {
    /// Sound event name, e.g. `minecraft:music.game`.
    pub event: String,
    /// Ticks to wait after a track ends before playing the next one: lower bound.
    pub min_delay: u32,
    /// Ticks to wait after a track ends before playing the next one: upper bound.
    pub max_delay: u32,
    /// Vanilla stops the current music when switching into this situation (`replaceCurrentMusic`).
    pub replace_current: bool,
}

/// What vanilla plays background music by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MusicSituation {
    /// The main menu: `music.menu`, replaces playing music.
    Menu,
    /// Survival in a world without its own biome music: `music.game`.
    Game,
    /// Creative mode: `music.creative`, same delays as in-game.
    Creative,
    /// Underwater: `music.under_water`, in-game delays.
    UnderWater,
    /// The End: `music.end`, replaces the current music (`Musics.END`).
    End,
    /// The dragon fight: `music.dragon`, no delay, replaces the current music.
    Dragon,
    /// The end credits: `music.credits`, no delay, replaces the current music.
    Credits,
    /// A biome with its own `music` field (nether biomes, some overworld biomes): the caller
    /// resolves the biome data and passes its event and delays here.
    Biome {
        event: String,
        min_delay: u32,
        max_delay: u32,
        replace: bool,
    },
}

impl MusicSituation {
    /// The music this situation plays, with vanilla's delays (the `Musics` constants).
    pub fn music(&self) -> MusicSpec {
        fn spec(event: &str, min_delay: u32, max_delay: u32, replace_current: bool) -> MusicSpec {
            MusicSpec {
                event: event.to_owned(),
                min_delay,
                max_delay,
                replace_current,
            }
        }
        match self {
            MusicSituation::Menu => spec("minecraft:music.menu", 20, 600, true),
            MusicSituation::Game => spec("minecraft:music.game", 12_000, 24_000, false),
            MusicSituation::Creative => spec("minecraft:music.creative", 12_000, 24_000, false),
            MusicSituation::UnderWater => {
                spec("minecraft:music.under_water", 12_000, 24_000, false)
            }
            MusicSituation::End => spec("minecraft:music.end", 6_000, 24_000, true),
            MusicSituation::Dragon => spec("minecraft:music.dragon", 0, 0, true),
            MusicSituation::Credits => spec("minecraft:music.credits", 0, 0, true),
            MusicSituation::Biome {
                event,
                min_delay,
                max_delay,
                replace,
            } => MusicSpec {
                event: event.clone(),
                min_delay: *min_delay,
                max_delay: *max_delay,
                replace_current: *replace,
            },
        }
    }
}

/// What the manager wants the output to do this tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MusicCommand {
    /// Start the music event (a sound event name like `minecraft:music.game`).
    Play(String),
    /// Stop the current music (a replacing situation change).
    Stop,
}

/// A small deterministic xorshift RNG, so the random pauses replay exactly for a fixed seed.
#[derive(Clone, Debug)]
pub struct MusicRng {
    state: u64,
}

impl MusicRng {
    /// Seeded from anything (a SplitMix64 step avoids the all-zero fixed point and poor
    /// low-entropy seeds).
    pub fn new(seed: u64) -> Self {
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Self {
            state: (z ^ (z >> 31)) | 1,
        }
    }

    fn next_u64(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniformly distributed in `[min, max]` (inclusive); `max <= min` yields `min`. The
    /// counterpart of vanilla's `Mth.nextInt(RandomSource, int, int)`.
    pub fn range(&mut self, min: u32, max: u32) -> u32 {
        if max <= min {
            return min;
        }
        let span = u64::from(max - min) + 1;
        min + (self.next_u64() % span) as u32
    }
}

/// Decides when background music plays, driven at 20 Hz. Owns no output: each tick it reports
/// the next thing to do, or `None`. Follows vanilla's `MusicManager`.
#[derive(Debug)]
pub struct MusicManager {
    situation: MusicSituation,
    /// Vanilla's `nextSongDelay`: ticks until the next track starts; `i32::MAX` while a track
    /// plays (vanilla's `startPlaying` sets `Integer.MAX_VALUE`).
    next_song_delay: i32,
    /// The event of the track we started and believe is playing (vanilla's `currentMusic`).
    current: Option<String>,
    /// The `music_frequency` option's cap in ticks (`MusicFrequency.DEFAULT`: 20 minutes).
    max_frequency: u32,
    /// Ticks this manager has run, for the diagnostic log (which track started when).
    ticks: u64,
}

impl Default for MusicManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MusicManager {
    /// New manager with the menu situation and vanilla's starting countdown of 100 ticks: the
    /// first track of a session plays within 5 s (vanilla's `Minecraft` starts at the main menu
    /// with the same countdown; haste's starts here at join, already in the game situation).
    pub fn new() -> Self {
        Self {
            situation: MusicSituation::Menu,
            next_song_delay: STARTING_DELAY,
            current: None,
            max_frequency: DEFAULT_MAX_FREQUENCY,
            ticks: 0,
        }
    }

    /// The `music_frequency` option's cap in ticks (vanilla's `setMinutesBetweenSongs`).
    pub fn set_max_frequency(&mut self, ticks: u32) {
        self.max_frequency = ticks;
    }

    /// Switches the situation (menu opened, world joined, biome entered, ...). The tick then
    /// applies vanilla's rules for it, so a situation whose music replaces the current track
    /// stops it and gets a short countdown, while a non-replacing change keeps the track and
    /// only caps the running countdown.
    pub fn set_situation(&mut self, situation: MusicSituation) {
        self.situation = situation;
    }

    /// The situation currently in effect.
    pub fn situation(&self) -> &MusicSituation {
        &self.situation
    }

    /// The event of the track this manager started and believes is still playing.
    pub fn playing_event(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// One game tick (20 Hz). `is_playing` tells whether the music this manager started is
    /// still playing; while something plays, nothing new is started. `rng` drives the random
    /// pauses (a fixed seed replays exactly).
    ///
    /// Vanilla's `MusicManager.tick` order of steps: a replacing situation stops the playing
    /// track; a track that is no longer playing picks a fresh pause; the situation's maximum
    /// caps the countdown; the countdown ticks down and starts the track when it reaches zero.
    pub fn tick(&mut self, rng: &mut MusicRng, is_playing: bool) -> Option<MusicCommand> {
        self.ticks += 1;
        let spec = self.situation.music();
        if let Some(current) = self.current.clone() {
            if is_playing {
                // The track is still playing: a situation with replacing music stops it and
                // gets a short fresh countdown (vanilla's `canReplace`), otherwise nothing
                // happens this tick (vanilla's `nextSongDelay` stays at Integer.MAX_VALUE).
                if spec.replace_current && current != spec.event {
                    self.current = None;
                    self.next_song_delay = self.replace_delay(&spec, rng);
                    tracing::info!(
                        "music: tick {}: stopping {current} (situation {:?})",
                        self.ticks,
                        self.situation
                    );
                    return Some(MusicCommand::Stop);
                }
                return None;
            }
            // The track ended: a fresh random pause before the next one, capped by the
            // situation's maximum below.
            self.current = None;
            let fresh = self.get_next_song_delay(&spec, rng);
            self.next_song_delay = self.next_song_delay.min(fresh as i32);
        }
        // The situation's own maximum caps the running countdown every tick.
        self.next_song_delay = self.next_song_delay.min(spec.max_delay as i32);
        if self.next_song_delay > 0 {
            self.next_song_delay -= 1;
        }
        if self.next_song_delay <= 0 {
            self.current = Some(spec.event.clone());
            self.next_song_delay = i32::MAX;
            tracing::info!(
                "music: tick {}: playing {} (situation {:?})",
                self.ticks,
                spec.event,
                self.situation
            );
            return Some(MusicCommand::Play(spec.event));
        }
        None
    }

    /// The countdown after a replacing situation change: vanilla's
    /// `Mth.nextInt(this.random, 0, music.minDelay() / 2)`.
    fn replace_delay(&self, spec: &MusicSpec, rng: &mut MusicRng) -> i32 {
        rng.range(0, spec.min_delay / 2).min(i32::MAX as u32) as i32
    }

    /// A fresh pause after a track ended: vanilla's
    /// `MusicFrequency.getNextSongDelay(music, random)` with the DEFAULT frequency — the
    /// music's own range with both ends capped by the option's maximum.
    fn get_next_song_delay(&self, spec: &MusicSpec, rng: &mut MusicRng) -> u32 {
        let min = spec.min_delay.min(self.max_frequency);
        let max = spec.max_delay.min(self.max_frequency);
        rng.range(min, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ticks with `is_playing` until the manager plays something; returns (ticks taken, event).
    fn until_play(
        manager: &mut MusicManager,
        rng: &mut MusicRng,
        is_playing: bool,
    ) -> (usize, String) {
        for tick in 1..=60_000usize {
            match manager.tick(rng, is_playing) {
                Some(MusicCommand::Play(event)) => return (tick, event),
                Some(MusicCommand::Stop) => panic!("unexpected Stop"),
                None => {}
            }
        }
        panic!("no Play within 60000 ticks");
    }

    /// Ticks until the manager returns anything but `None`.
    fn until_command(
        manager: &mut MusicManager,
        rng: &mut MusicRng,
        is_playing: bool,
    ) -> MusicCommand {
        for _ in 1..=60_000usize {
            if let Some(command) = manager.tick(rng, is_playing) {
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
            assert_eq!(manager.tick(&mut rng, true), None);
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
            assert_eq!(manager.tick(&mut rng, true), None);
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
            assert_eq!(manager.tick(&mut rng, true), None);
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
            assert_eq!(manager.tick(&mut rng, true), None);
        }
        manager.set_situation(MusicSituation::Creative);
        for _ in 0..100 {
            assert_eq!(manager.tick(&mut rng, true), None);
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
        assert_eq!(manager.tick(&mut rng, false), None);
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
                manager.tick(&mut rng, false),
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
                match manager.tick(&mut rng, playing) {
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
            assert_eq!(manager.tick(&mut rng, true), None);
        }
        assert_eq!(manager.playing_event(), Some(event.as_str()));
    }
}
