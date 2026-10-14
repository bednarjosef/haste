//! Sound in the game (client-app): the `SoundSystem` and `MusicManager` from `haste-audio`
//! driven by the packets the server sends, on the game thread at 20 Hz.
//!
//! The packet half is a set of pure functions ([`sound_source`], [`event_name`],
//! [`sound_position`]) so the mapping from the wire to a play call is unit-testable; the
//! `Sounds` glue is only the plumbing. Vanilla's seams are kept: one handler per sound
//! packet (`Session::apply` records a [`SoundRequest`], [`Sounds::apply`] plays it), the
//! listener update and the 20 Hz tick ([`Sounds::tick`]) are separate steps, and the music
//! situation is recomputed where vanilla recomputes it (game mode, eye in water).
//!
//! Unknown sound ids (registry ids the synced `minecraft:sound_event` registry does not
//! have, events `sounds.json` does not know) play nothing and log once per id — a
//! mod-added sound must never crash the client.

use std::collections::HashSet;

use glam::DVec3;
use haste_audio::engine::SoundSource;
use haste_audio::music::{MusicCommand, MusicManager, MusicRng, MusicSituation};
use haste_audio::output::SoundSystem;
use haste_audio::sounds::SoundEvents;
use haste_core::game::GameMode;
use haste_core::Identifier;
use haste_protocol::wire::{IdOr, SoundEvent as InlineSoundEvent};
use haste_protocol::ClientboundPlay;
use haste_protocol::SyncedRegistries;

/// Vanilla's `SoundSource` order, 0..=10 (read from the 26.3 jar by the lead; the packet's
/// category VarInt is the index in it). Matches `haste_audio::engine::SoundSource`.
pub const SOURCES: [SoundSource; 11] = [
    SoundSource::Master,
    SoundSource::Music,
    SoundSource::Record,
    SoundSource::Weather,
    SoundSource::Block,
    SoundSource::Hostile,
    SoundSource::Neutral,
    SoundSource::Player,
    SoundSource::Ambient,
    SoundSource::Voice,
    SoundSource::Ui,
];

/// The packet's category index as a `SoundSource`; `None` when outside 0..=10 (a newer
/// server's category we do not know plays nothing rather than guessing).
pub fn sound_source(category: i32) -> Option<SoundSource> {
    if (0..=10).contains(&category) {
        Some(SOURCES[category as usize])
    } else {
        None
    }
}

/// The synced `minecraft:sound_event` registry's identifier.
fn sound_registry() -> Identifier {
    Identifier::minecraft("sound_event")
}

/// The sound a packet refers to: a registry id into `minecraft:sound_event` or an inline
/// event's own name.
#[derive(Clone, Debug, PartialEq)]
pub enum SoundRef {
    /// Registry id (the packet's `IdOr::Id`).
    Id(i32),
    /// Inline event name (the packet's `IdOr::Inline`), namespaced or not.
    Inline(String),
}

impl SoundRef {
    /// From a packet's sound field.
    pub fn of(sound: &IdOr<InlineSoundEvent>) -> SoundRef {
        match sound {
            IdOr::Id(id) => SoundRef::Id(*id),
            IdOr::Inline(e) => SoundRef::Inline(e.name.to_string()),
        }
    }
}

/// The event name a sound refers to, through the client's `minecraft:sound_event` name
/// table (see [`sound_event_names`]). `None` when the id is not in the table. Names are
/// namespaced (`minecraft:block.stone.break`), as `sounds.json` keys are after parsing.
pub fn resolve_event(sound: &SoundRef, names: &[String]) -> Option<String> {
    match sound {
        SoundRef::Id(id) => usize::try_from(*id)
            .ok()
            .and_then(|i| names.get(i).cloned()),
        SoundRef::Inline(name) => Some(name.clone()),
    }
}

/// The client's `minecraft:sound_event` name table, indexed by the registry id the sound
/// packets send: the server's synced registry entries when it sent them (modded servers),
/// else the static registry from the data generator reports (`registries.json`), which is
/// what vanilla servers index into. Built once at startup.
pub fn sound_event_names(regs: &SyncedRegistries, data: &haste_data::GameData) -> Vec<String> {
    if let Some(entries) = regs.entries(&sound_registry()) {
        return entries.iter().map(|e| e.id.to_string()).collect();
    }
    let registry = "minecraft:sound_event";
    (0..data.registries.len(registry) as u32)
        .map(|i| {
            data.registries
                .name(registry, haste_core::RegistryIndex(i))
                .map(|n| n.to_string())
                .unwrap_or_default()
        })
        .collect()
}

/// The position of a positional sound packet: each coordinate is sent times 8.
pub fn sound_position(x: i32, y: i32, z: i32) -> DVec3 {
    DVec3::new(x as f64 / 8.0, y as f64 / 8.0, z as f64 / 8.0)
}

/// The music situation for the client's state: the eye in water plays `music.under_water`,
/// creative mode `music.creative`, otherwise `music.game` (vanilla's order of checks).
pub fn music_situation(game_mode: GameMode, eyes_in_water: bool) -> MusicSituation {
    if eyes_in_water {
        MusicSituation::UnderWater
    } else if game_mode == GameMode::Creative {
        MusicSituation::Creative
    } else {
        MusicSituation::Game
    }
}

/// One sound the server asked for, recorded by `Session::apply` and played once per tick by
/// [`Sounds::apply`]. The event name is resolved against the client's name table at play
/// time (see [`sound_event_names`]).
#[derive(Clone, Debug, PartialEq)]
pub struct PlayRequest {
    pub sound: SoundRef,
    pub category: i32,
    /// Fixed position (`minecraft:sound`) or entity to take it from (`sound_entity`).
    pub pos: SoundPos,
    pub volume: f32,
    pub pitch: f32,
    pub seed: i64,
}

/// Where a recorded sound plays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SoundPos {
    /// `minecraft:sound`: x/8, y/8, z/8.
    Fixed(DVec3),
    /// `minecraft:sound_entity`: the entity's current position (skipped when unknown).
    Entity(i32),
}

/// One `minecraft:stop_sound`.
#[derive(Clone, Debug, PartialEq)]
pub struct StopRequest {
    pub source: Option<i32>,
    pub sound: Option<String>,
}

/// Records the sound packets into requests (the packet half of the glue, so `Session`
/// stays testable without audio). Pure mapping, no output.
pub fn record(p: &ClientboundPlay) -> Option<SoundRequest> {
    match p {
        ClientboundPlay::Sound(s) => Some(SoundRequest::Play(PlayRequest {
            sound: SoundRef::of(&s.sound),
            category: s.category,
            pos: SoundPos::Fixed(sound_position(s.x, s.y, s.z)),
            volume: s.volume,
            pitch: s.pitch,
            seed: s.seed,
        })),
        ClientboundPlay::SoundEntity(s) => Some(SoundRequest::Play(PlayRequest {
            sound: SoundRef::of(&s.sound),
            category: s.category,
            pos: SoundPos::Entity(s.entity_id),
            volume: s.volume,
            pitch: s.pitch,
            seed: s.seed,
        })),
        ClientboundPlay::StopSound(s) => Some(SoundRequest::Stop(StopRequest {
            source: s.source,
            sound: s.sound.as_ref().map(|n| n.to_string()),
        })),
        _ => None,
    }
}

/// One recorded request: a play or a stop.
#[derive(Clone, Debug, PartialEq)]
pub enum SoundRequest {
    Play(PlayRequest),
    Stop(StopRequest),
}

/// The sound and music output of one client, owned by the game thread (through
/// `Shared::sound`). Everything here runs at 20 Hz or per packet; a tick with no sound
/// started allocates nothing (`SoundSystem::tick` reuses its buffers).
pub struct Sounds {
    system: SoundSystem,
    /// The parsed `sounds.json`, to tell known events from unknown ones (log once per
    /// id): the stop-by-event filter matches event names, so no resolving is needed.
    events: SoundEvents,
    /// The `minecraft:sound_event` name table registry ids resolve through.
    names: Vec<String>,
    music: MusicManager,
    music_rng: MusicRng,
    /// Situation as of the last tick, so a change is only pushed on a change.
    situation: MusicSituation,
    /// The music event we started and believe is playing.
    music_event: Option<String>,
    /// Ids/events already logged as unknown (log once per id, no spam).
    unknown_logged: HashSet<String>,
}

/// The saved per-category volumes onto the system (vanilla's `soundCategory_*` option
/// keys), each scaled by the master volume.
fn apply_volumes(system: &mut SoundSystem, volumes: &haste_setup::options::SoundVolumes) {
    let v = volumes;
    system.set_volume(SoundSource::Master, v.master);
    system.set_volume(SoundSource::Music, v.music);
    system.set_volume(SoundSource::Record, v.record);
    system.set_volume(SoundSource::Weather, v.weather);
    system.set_volume(SoundSource::Block, v.block);
    system.set_volume(SoundSource::Hostile, v.hostile);
    system.set_volume(SoundSource::Neutral, v.neutral);
    system.set_volume(SoundSource::Player, v.player);
    system.set_volume(SoundSource::Ambient, v.ambient);
    system.set_volume(SoundSource::Voice, v.voice);
    system.set_volume(SoundSource::Ui, v.ui);
}

impl Sounds {
    /// Build the output from the parsed `sounds.json`, the open assets, the sound-event
    /// name table and the saved per-category volumes. Without an audio device this stays
    /// silent (`AudioOutput`'s fallback), so headless and CI could build it too.
    pub fn new(
        events: SoundEvents,
        assets: haste_assets::Assets,
        names: Vec<String>,
        volumes: &haste_setup::options::SoundVolumes,
    ) -> Self {
        let mut system = SoundSystem::new(events.clone(), assets);
        apply_volumes(&mut system, volumes);
        Self::with_system(system, events, names)
    }

    /// The glue over an already built `SoundSystem` (a test builds it over the silent
    /// output, so no test ever opens the audio device).
    fn with_system(system: SoundSystem, events: SoundEvents, names: Vec<String>) -> Self {
        Self {
            system,
            events,
            names,
            music: MusicManager::new(),
            music_rng: MusicRng::new(0x5eed_1234),
            situation: MusicSituation::Menu,
            music_event: None,
            unknown_logged: HashSet::new(),
        }
    }

    /// Plays the requests this tick's packets recorded. `entity_pos` resolves a
    /// `sound_entity`'s entity to its current position; `None` (unknown entity) skips the
    /// sound, as vanilla does. The requests are consumed (the caller's buffer is cleared).
    pub fn apply(
        &mut self,
        requests: &mut Vec<SoundRequest>,
        entity_pos: impl Fn(i32) -> Option<DVec3>,
    ) {
        for request in requests.drain(..) {
            match request {
                SoundRequest::Play(p) => self.play(p, &entity_pos),
                SoundRequest::Stop(s) => self.stop(s),
            }
        }
    }

    /// One recorded play request: resolve the category, event and position, then play.
    fn play(&mut self, p: PlayRequest, entity_pos: &impl Fn(i32) -> Option<DVec3>) {
        let Some(source) = sound_source(p.category) else {
            self.log_unknown(&format!("category #{}", p.category));
            return;
        };
        let Some(event) = resolve_event(&p.sound, &self.names) else {
            match p.sound {
                SoundRef::Id(id) => self.log_unknown(&format!("sound_event #{id}")),
                SoundRef::Inline(ref name) => self.log_unknown(name),
            }
            return;
        };
        if self.events.get(&event).is_none() {
            self.log_unknown(&event);
            return;
        }
        let pos = match p.pos {
            SoundPos::Fixed(pos) => Some(pos),
            SoundPos::Entity(id) => entity_pos(id),
        };
        self.system
            .play(&event, source, pos, p.volume, p.pitch, p.seed);
    }

    /// One recorded stop request: stop by category and/or event name, the packet's own
    /// two fields. `SoundSystem::stop` matches playing sounds by event name (vanilla's
    /// `SoundManager.stop(Identifier, SoundSource)`), so a name that is not playing
    /// stops nothing, and both fields absent stops everything.
    fn stop(&mut self, s: StopRequest) {
        let source = s.source.and_then(sound_source);
        self.system.stop(source, s.sound.as_deref());
    }

    /// Log one unknown id/event once; further occurrences stay quiet.
    fn log_unknown(&mut self, id: &str) {
        if self.unknown_logged.insert(id.to_owned()) {
            tracing::info!("unknown sound {id}; not played");
        }
    }

    /// Whether the music this client started is still playing. `SoundSystem` remembers
    /// the event each sound was started with, so this follows our own track and no other:
    /// a block or mob sound never counts, as in vanilla's `isActive(currentMusic)`.
    fn music_playing(&self) -> bool {
        self.music_event
            .as_deref()
            .is_some_and(|event| self.system.is_event_playing(event))
    }

    /// The end of a game tick (20 Hz): listener from the local player's eye and yaw, the
    /// music situation, the music manager, then the system tick.
    pub fn tick(&mut self, eye: DVec3, yaw_deg: f32, situation: MusicSituation) {
        self.system.set_listener(eye, yaw_deg);
        if situation != self.situation {
            self.music.set_situation(situation.clone());
            self.situation = situation;
        }
        let playing = self.music_playing();
        if let Some(command) = self.music.tick(&mut self.music_rng, playing) {
            match command {
                MusicCommand::Play(event) => {
                    self.system
                        .play(&event, SoundSource::Music, None, 1.0, 1.0, 0);
                    self.music_event = Some(event);
                }
                MusicCommand::Stop => {
                    self.system.stop(Some(SoundSource::Music), None);
                    self.music_event = None;
                }
            }
        }
        self.system.tick();
    }
}

#[cfg(test)]
mod tests {
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
}
