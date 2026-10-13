//! Background music scheduling: which music event fits the player's situation, and when to
//! play it. Pure logic — the engine turns the [`MusicCommand`]s into actual output.
//!
//! Vanilla plays background music by situation with random pauses between tracks. The delays
//! below come from minecraft.wiki, "Music": in-game tracks every 10-20 minutes (12000-24000
//! ticks at 20 Hz), the main menu every 20-600 ticks, where menu music replaces whatever is
//! playing; creative mode uses `music.creative` with the in-game delays; underwater
//! `music.under_water` with the in-game delays; the End `music.end` 6000-24000; the dragon
//! fight `music.dragon` and the credits `music.credits` with no delay and replacing the
//! current music. Nether biomes and some overworld biomes carry their own music event in the
//! biome's `music` field; resolving that needs biome data this crate does not have, so the
//! caller looks up the biome and passes [`MusicSituation::Biome`] with the event and delays
//! from the biome data.

/// The music event and timing rules for one situation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicSpec {
    /// Sound event name, e.g. `minecraft:music.game`.
    pub event: String,
    /// Ticks to wait after a track ends before playing the next one: lower bound.
    pub min_delay: u32,
    /// Ticks to wait after a track ends before playing the next one: upper bound.
    pub max_delay: u32,
    /// Vanilla stops the current music when switching into this situation.
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
    /// The End: `music.end`.
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
    /// The music this situation plays, with vanilla's delays (see the module docs).
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
            MusicSituation::End => spec("minecraft:music.end", 6_000, 24_000, false),
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

    /// Uniformly distributed in `[min, max]` (inclusive); `max <= min` yields `min`.
    pub fn range(&mut self, min: u32, max: u32) -> u32 {
        if max <= min {
            return min;
        }
        let span = u64::from(max - min) + 1;
        min + (self.next_u64() % span) as u32
    }
}

/// After a replacing situation change, vanilla starts the new track promptly; this is the
/// approximation used here: the new track starts within this many ticks (capped further by the
/// situation's own no-delay cases, the dragon fight and the credits).
const REPLACE_PLAY_MAX: u32 = 100;

/// Decides when background music plays, driven at 20 Hz. Owns no output: each tick it reports
/// the next thing to do, or `None`.
#[derive(Debug)]
pub struct MusicManager {
    situation: MusicSituation,
    /// Ticks left before the next track; `None` = pick a fresh delay when next needed.
    countdown: Option<u32>,
    /// A replacing situation change queued a `Stop` for the next tick.
    stop_pending: bool,
    /// The next countdown is the short one after replacing the current music.
    replace_play: bool,
    /// We sent `Play` and are waiting for the output to confirm it started.
    play_issued: bool,
    /// The output confirmed our track and has not reported it finished yet.
    track_live: bool,
    /// Event of the track we believe is playing.
    playing: Option<String>,
}

impl MusicManager {
    /// New manager in the main menu (the client starts there).
    pub fn new() -> Self {
        Self {
            situation: MusicSituation::Menu,
            countdown: None,
            stop_pending: false,
            replace_play: false,
            play_issued: false,
            track_live: false,
            playing: None,
        }
    }

    /// Switches the situation (menu opened, world joined, biome entered, ...). A situation
    /// whose music replaces the current one and differs from it stops the playing track: the
    /// next [`MusicManager::tick`] returns that stop and the new track follows promptly. After
    /// a non-replacing change the current track keeps playing and the pending countdown is
    /// clamped to the new situation's maximum.
    pub fn set_situation(&mut self, situation: MusicSituation) {
        let new = situation.music();
        if new == self.situation.music() {
            self.situation = situation;
            return;
        }
        let replaces = new.replace_current && self.playing.as_deref() != Some(new.event.as_str());
        self.situation = situation;
        if replaces {
            self.stop_pending = self.playing.is_some();
            self.play_issued = false;
            self.track_live = false;
            self.playing = None;
            self.countdown = None;
            self.replace_play = true;
        } else {
            self.replace_play = false;
            if let Some(countdown) = &mut self.countdown {
                // Clamp so the track still starts within the new situation's maximum: the
                // countdown is one less than the remaining delay (the play tick counts).
                *countdown = (*countdown).min(new.max_delay.saturating_sub(1));
            }
        }
    }

    /// The situation currently in effect.
    pub fn situation(&self) -> &MusicSituation {
        &self.situation
    }

    /// The event of the track this manager started and believes is still playing.
    pub fn playing_event(&self) -> Option<&str> {
        self.playing.as_deref()
    }

    /// One game tick (20 Hz). `is_playing` tells whether the music this manager started is
    /// still playing; while something plays, nothing new is started. `rng` drives the random
    /// pauses (a fixed seed replays exactly).
    pub fn tick(&mut self, rng: &mut MusicRng, is_playing: bool) -> Option<MusicCommand> {
        // A queued replacement stop goes out first.
        if self.stop_pending {
            self.stop_pending = false;
            self.playing = None;
            self.play_issued = false;
            self.track_live = false;
            self.countdown = None;
            return Some(MusicCommand::Stop);
        }

        // Follow the track we started: once the output confirms it, wait for it to end.
        if self.play_issued && is_playing {
            self.play_issued = false;
            self.track_live = true;
        }
        if self.track_live && !is_playing {
            // The track ended: wait a fresh random pause before the next one.
            self.track_live = false;
            self.playing = None;
            self.countdown = None;
        }

        // Nothing playing and nothing on the way: count down to the next track.
        if !is_playing && !self.play_issued && self.playing.is_none() {
            let spec = self.situation.music();
            let countdown = match self.countdown {
                Some(countdown) => countdown,
                None => {
                    let picked = if self.replace_play {
                        let min = spec.min_delay.min(REPLACE_PLAY_MAX);
                        rng.range(min, spec.max_delay.min(REPLACE_PLAY_MAX).max(min))
                    } else {
                        rng.range(spec.min_delay, spec.max_delay)
                    };
                    self.replace_play = false;
                    // A delay of N ticks means the track starts N ticks from now, so this
                    // very tick counts as the first one.
                    if picked > 0 {
                        self.countdown = Some(picked - 1);
                    }
                    picked
                }
            };
            if countdown == 0 {
                self.play_issued = true;
                self.playing = Some(spec.event.clone());
                self.countdown = None;
                Some(MusicCommand::Play(spec.event))
            } else {
                self.countdown = Some(countdown - 1);
                None
            }
        } else {
            None
        }
    }
}

impl Default for MusicManager {
    fn default() -> Self {
        Self::new()
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

    #[test]
    fn menu_countdown_within_wiki_range() {
        let mut manager = MusicManager::new();
        let mut rng = MusicRng::new(1);
        let (ticks, event) = until_play(&mut manager, &mut rng, false);
        assert_eq!(event, "minecraft:music.menu");
        assert!(
            (20..=600).contains(&ticks),
            "menu delay {ticks} outside 20..=600"
        );
    }

    #[test]
    fn plays_when_countdown_reaches_zero_and_nothing_plays() {
        // The dragon fight has no delay and replaces: it plays at once.
        let mut manager = MusicManager::new();
        manager.set_situation(MusicSituation::Dragon);
        let mut rng = MusicRng::new(2);
        assert_eq!(
            manager.tick(&mut rng, false),
            Some(MusicCommand::Play("minecraft:music.dragon".to_owned()))
        );
    }

    #[test]
    fn no_play_while_playing() {
        let mut manager = MusicManager::new();
        manager.set_situation(MusicSituation::Game);
        let mut rng = MusicRng::new(3);
        for _ in 0..24_000 {
            assert_eq!(manager.tick(&mut rng, true), None);
        }
    }

    #[test]
    fn menu_replaces_game_music() {
        let mut manager = MusicManager::new();
        manager.set_situation(MusicSituation::Game);
        let mut rng = MusicRng::new(4);
        let (_, event) = until_play(&mut manager, &mut rng, false);
        assert_eq!(event, "minecraft:music.game");
        // The track plays for a while, then the menu opens.
        for _ in 0..10 {
            assert_eq!(manager.tick(&mut rng, true), None);
        }
        manager.set_situation(MusicSituation::Menu);
        assert_eq!(manager.tick(&mut rng, true), Some(MusicCommand::Stop));
        let (ticks, event) = until_play(&mut manager, &mut rng, false);
        assert_eq!(event, "minecraft:music.menu");
        assert!(
            ticks <= REPLACE_PLAY_MAX as usize,
            "menu music should follow within {REPLACE_PLAY_MAX} ticks, took {ticks}"
        );
    }

    #[test]
    fn game_to_creative_keeps_current_track() {
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
        // The game track ends; creative music follows with the in-game delays.
        let (ticks, event) = until_play(&mut manager, &mut rng, false);
        assert_eq!(event, "minecraft:music.creative");
        assert!(
            (12_000..=24_000).contains(&ticks),
            "creative delay {ticks} outside 12000..=24000"
        );
    }

    #[test]
    fn non_replacing_change_clamps_countdown() {
        let mut manager = MusicManager::new();
        manager.set_situation(MusicSituation::Game);
        let mut rng = MusicRng::new(6);
        // One tick picks the in-game countdown, somewhere in 12000..=24000.
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
}
