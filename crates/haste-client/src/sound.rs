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

mod own_block;
pub use own_block::OwnBlockSounds;

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
    /// The random each music track's sound instance draws its variant from (vanilla's
    /// `SimpleSoundInstance.forMusic` uses an unseeded random per track).
    music_variants: haste_audio::random::LegacyRandom,
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
            music_variants: haste_audio::random::LegacyRandom::new(own_block::unseeded()),
            situation: MusicSituation::Menu,
            music_event: None,
            unknown_logged: HashSet::new(),
        }
    }

    /// Re-applies the per-category volumes live, the moment the player moves a sound slider
    /// (vanilla's `SoundEngine.refreshCategoryVolume` for every `soundCategory_*` option).
    pub fn set_volumes(&mut self, volumes: &haste_setup::options::SoundVolumes) {
        apply_volumes(&mut self.system, volumes);
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

    /// The end of a game tick (20 Hz): listener from the local player's eye and yaw, the music
    /// situation, the music manager, then the system tick. The situation is recomputed where
    /// vanilla recomputes it (`Minecraft.getSituationalMusic`: game mode, eye in water); the
    /// manager follows vanilla's `MusicManager.tick` from there on.
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
                    // Vanilla's `SimpleSoundInstance.forMusic(soundEvent)`: no position (the
                    // listener hears it centred), the music source, volume and pitch 1.
                    let seed = self.music_variants.next_long();
                    self.system
                        .play(&event, SoundSource::Music, None, 1.0, 1.0, seed);
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
mod tests;
