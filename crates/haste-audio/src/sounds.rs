//! `sounds.json`: the sound-event table shipped in the client assets.
//!
//! The file maps event names (`"block.stone.break"`, no namespace) to a list of sound
//! entries plus an optional subtitle. An entry is either a file reference
//! (`"dig/stone1"` = `minecraft:dig/stone1` = the Ogg at `sounds/dig/stone1.ogg`, a
//! namespace may be given as `"foo:bar"`) or an object with `name` and overrides, where
//! `type: "event"` makes `name` another event to resolve recursively.
use std::collections::HashMap;

use anyhow::bail;
use haste_core::Identifier;
use serde::Deserialize;

use crate::random::LegacyRandom;

/// One playable sound file with all of its per-entry settings from `sounds.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundVariant {
    /// The sound's full identifier, e.g. `minecraft:dig/stone1`.
    pub id: Identifier,
    /// Path of the Ogg inside the assets, e.g. `sounds/dig/stone1.ogg`.
    pub path: String,
    pub volume: f32,
    pub pitch: f32,
    pub weight: u32,
    pub stream: bool,
    pub attenuation_distance: u32,
    pub preload: bool,
}

/// An entry of an event: a file variant, or a reference to another event.
#[derive(Clone, Debug)]
pub enum SoundEntry {
    File(SoundVariant),
    /// `type: "event"`: `name` is another event whose pick this entry's
    /// `volume`/`pitch` multiply into.
    Event {
        name: String,
        volume: f32,
        pitch: f32,
        weight: u32,
        stream: bool,
    },
}

/// One event of `sounds.json`: its entry list and subtitle.
#[derive(Clone, Debug)]
pub struct SoundEvent {
    pub sounds: Vec<SoundEntry>,
    pub subtitle: Option<String>,
}

/// All events parsed from `assets/minecraft/sounds.json`.
#[derive(Clone, Debug, Default)]
pub struct SoundEvents {
    /// Keyed by namespaced event name (`minecraft:block.stone.break`).
    events: HashMap<String, SoundEvent>,
}

/// Event key/reference with the default `minecraft` namespace applied.
fn namespaced(name: &str) -> String {
    match name.split_once(':') {
        Some(_) => name.to_owned(),
        None => format!("minecraft:{name}"),
    }
}

#[derive(Deserialize)]
struct RawEvent {
    sounds: Vec<RawEntry>,
    subtitle: Option<String>,
    // `replace` only matters when several packs stack the same file; we read one.
    #[serde(default)]
    #[allow(dead_code)]
    replace: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawEntry {
    Name(String),
    Full {
        name: String,
        #[serde(default = "default_f32_1")]
        volume: f32,
        #[serde(default = "default_f32_1")]
        pitch: f32,
        #[serde(default = "default_u32_1")]
        weight: u32,
        #[serde(default)]
        stream: bool,
        #[serde(default = "default_attenuation")]
        attenuation_distance: u32,
        #[serde(default)]
        preload: bool,
        #[serde(rename = "type")]
        kind: Option<String>,
    },
}

fn default_f32_1() -> f32 {
    1.0
}
fn default_u32_1() -> u32 {
    1
}
fn default_attenuation() -> u32 {
    16
}

/// The Ogg asset key of a sound name: `dig/stone1` -> `minecraft/sounds/dig/stone1.ogg`,
/// `foo:bar` -> `foo/sounds/bar.ogg` (the namespace is where in the assets the file lives).
fn sound_path(id: &Identifier) -> String {
    format!("{}/sounds/{}.ogg", id.namespace(), id.path())
}

impl SoundEvents {
    /// Parses `assets/minecraft/sounds.json`. Keys are event names without namespace;
    /// the `minecraft` namespace is applied to them.
    pub fn parse(json: &[u8]) -> anyhow::Result<SoundEvents> {
        let raw: HashMap<String, RawEvent> = serde_json::from_slice(json)
            .map_err(|e| anyhow::anyhow!("parsing sounds.json: {e}"))?;
        let mut events = HashMap::with_capacity(raw.len());
        for (name, ev) in raw {
            let mut sounds = Vec::with_capacity(ev.sounds.len());
            for entry in ev.sounds {
                sounds.push(match entry {
                    RawEntry::Name(name) => {
                        let id = parse_sound_name(&name)?;
                        SoundEntry::File(SoundVariant {
                            id: id.clone(),
                            path: sound_path(&id),
                            volume: 1.0,
                            pitch: 1.0,
                            weight: 1,
                            stream: false,
                            attenuation_distance: 16,
                            preload: false,
                        })
                    }
                    RawEntry::Full {
                        name,
                        volume,
                        pitch,
                        weight,
                        stream,
                        attenuation_distance,
                        preload,
                        kind,
                    } => match kind.as_deref() {
                        None | Some("file") => {
                            let id = parse_sound_name(&name)?;
                            SoundEntry::File(SoundVariant {
                                id: id.clone(),
                                path: sound_path(&id),
                                volume,
                                pitch,
                                weight,
                                stream,
                                attenuation_distance,
                                preload,
                            })
                        }
                        Some("event") => SoundEntry::Event {
                            name: namespaced(&name),
                            volume,
                            pitch,
                            weight,
                            stream,
                        },
                        Some(other) => bail!("unknown sound type {other:?} for {name:?}"),
                    },
                });
            }
            events.insert(
                namespaced(&name),
                SoundEvent {
                    sounds,
                    subtitle: ev.subtitle,
                },
            );
        }
        Ok(SoundEvents { events })
    }

    /// The event with this name, `minecraft:block.stone.break` or `block.stone.break`.
    pub fn get(&self, event: &str) -> Option<&SoundEvent> {
        self.events.get(&namespaced(event))
    }

    /// Drop every file entry whose file does not exist, vanilla's
    /// `SoundManager.Preparations.handleRegistration` skipping entries that fail
    /// `validateSoundResource` (logged there as "File {} does not exist"): a missing file
    /// is skipped at load instead of failing to play later. `exists` answers whether an
    /// asset key (`<namespace>/sounds/<path>.ogg`) is present. An event left with no
    /// (or only weightless) entries simply never plays, as vanilla's does. Returns the
    /// number of dropped entries.
    pub fn retain_files(&mut self, mut exists: impl FnMut(&str) -> bool) -> usize {
        let mut dropped = 0;
        for event in self.events.values_mut() {
            let before = event.sounds.len();
            event.sounds.retain(|entry| match entry {
                SoundEntry::File(variant) => {
                    let present = exists(&variant.path);
                    if !present {
                        tracing::warn!(
                            "File {} does not exist, cannot add it to event",
                            variant.path
                        );
                    }
                    present
                }
                SoundEntry::Event { .. } => true,
            });
            dropped += before - event.sounds.len();
        }
        dropped
    }

    /// The event's subtitle key, if it has one.
    pub fn subtitle(&self, event: &str) -> Option<&str> {
        self.get(event).and_then(|e| e.subtitle.as_deref())
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Picks one variant of the event the way vanilla's `WeighedSoundEvents.getSound` does
    /// with the sound instance's `RandomSource.create(seed)`: one `nextInt(total weight)`
    /// over the entries in file order, so a packet's seed picks the same file as in the
    /// vanilla client. A `type: "event"` entry weighs as much as the whole event it names
    /// (its own `weight` is not used for the choice, as in `SoundManager.Preparations`),
    /// and choosing it draws again from the same generator inside that event; its
    /// `volume`/`pitch` multiply into the picked file's and its `stream` is or-ed in.
    /// References deeper than 8 levels (and cycles) give `None`, as does an event with no
    /// weight (vanilla's empty sound, which never plays).
    pub fn pick(&self, event: &str, seed: i64) -> Option<SoundVariant> {
        self.pick_with(event, &mut LegacyRandom::new(seed))
    }

    /// [`Self::pick`] with a caller-owned generator (a sound instance's own random).
    pub fn pick_with(&self, event: &str, rng: &mut LegacyRandom) -> Option<SoundVariant> {
        self.pick_at(event, rng, 0)
    }

    /// `WeighedSoundEvents.getWeight`: the sum of the entries' weights, a reference
    /// counting the total weight of the event it names (0 when that event is unknown).
    fn weight_of(&self, event: &str, depth: u32) -> u32 {
        if depth > 8 {
            return 0;
        }
        self.get(event).map_or(0, |ev| {
            ev.sounds
                .iter()
                .map(|s| self.entry_weight(s, depth))
                .fold(0u32, u32::saturating_add)
        })
    }

    fn entry_weight(&self, entry: &SoundEntry, depth: u32) -> u32 {
        match entry {
            SoundEntry::File(v) => v.weight,
            SoundEntry::Event { name, .. } => self.weight_of(name, depth + 1),
        }
    }

    fn pick_at(&self, event: &str, rng: &mut LegacyRandom, depth: u32) -> Option<SoundVariant> {
        if depth > 8 {
            return None;
        }
        let ev = self.get(event)?;
        let total = self.weight_of(event, depth);
        if ev.sounds.is_empty() || total == 0 {
            return None;
        }
        let mut index = rng.next_int(total.min(i32::MAX as u32) as i32) as i64;
        let entry = ev.sounds.iter().find(|s| {
            index -= i64::from(self.entry_weight(s, depth));
            index < 0
        })?;
        match entry {
            SoundEntry::File(v) => Some(v.clone()),
            SoundEntry::Event {
                name,
                volume,
                pitch,
                stream,
                ..
            } => {
                let mut v = self.pick_at(name, rng, depth + 1)?;
                v.volume *= volume;
                v.pitch *= pitch;
                v.stream |= stream;
                Some(v)
            }
        }
    }
}

/// `foo:bar/baz` or `bar/baz` (namespace `minecraft`) as an [`Identifier`].
fn parse_sound_name(name: &str) -> anyhow::Result<Identifier> {
    Identifier::parse(name).map_err(|e| anyhow::anyhow!("sound name {name:?}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_and_object_entries_with_defaults() {
        let json = br#"{
            "block.stone.break": {
                "subtitle": "subtitles.block.generic.break",
                "sounds": [
                    "dig/stone1",
                    {"name": "dig/stone2"},
                    {"name": "dig/stone3", "volume": 0.8, "pitch": 1.4, "weight": 2,
                     "stream": true, "attenuation_distance": 8, "preload": true}
                ]
            }
        }"#;
        let events = SoundEvents::parse(json).unwrap();
        let ev = events.get("block.stone.break").unwrap();
        assert_eq!(ev.sounds.len(), 3);
        let SoundEntry::File(v) = &ev.sounds[0] else {
            panic!("expected file entry");
        };
        assert_eq!(v.id.to_string(), "minecraft:dig/stone1");
        assert_eq!(v.path, "minecraft/sounds/dig/stone1.ogg");
        assert_eq!((v.volume, v.pitch, v.weight), (1.0, 1.0, 1));
        assert_eq!(
            (v.stream, v.attenuation_distance, v.preload),
            (false, 16, false)
        );
        let SoundEntry::File(v) = &ev.sounds[1] else {
            panic!("expected file entry");
        };
        assert_eq!((v.volume, v.weight, v.stream), (1.0, 1, false));
        let SoundEntry::File(v) = &ev.sounds[2] else {
            panic!("expected file entry");
        };
        assert_eq!((v.volume, v.pitch, v.weight), (0.8, 1.4, 2));
        assert_eq!(
            (v.stream, v.attenuation_distance, v.preload),
            (true, 8, true)
        );
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn namespaced_names() {
        let json = br#"{"e": {"sounds": ["foo:bar/baz"]}}"#;
        let events = SoundEvents::parse(json).unwrap();
        let SoundEntry::File(v) = &events.get("e").unwrap().sounds[0] else {
            panic!("expected file entry");
        };
        assert_eq!(v.id.to_string(), "foo:bar/baz");
        assert_eq!(v.path, "foo/sounds/bar/baz.ogg");
    }

    #[test]
    fn weighted_pick_favors_the_heavy_entry() {
        // 3:1 weights -> the heavy entry should come up about 75 % of the time.
        let json = br#"{"e": {"sounds": [
            {"name": "heavy", "weight": 3},
            {"name": "light", "weight": 1}
        ]}}"#;
        let events = SoundEvents::parse(json).unwrap();
        let heavy = (0..1000)
            .filter(|seed| events.pick("e", *seed).unwrap().id.path() == "heavy")
            .count();
        assert!(
            (650..=850).contains(&heavy),
            "heavy picked {heavy}/1000 times"
        );
    }

    #[test]
    fn pick_is_deterministic() {
        let json = br#"{"e": {"sounds": [
            {"name": "a"}, {"name": "b"}, {"name": "c", "weight": 5}
        ]}}"#;
        let events = SoundEvents::parse(json).unwrap();
        for seed in [-5, 0, 1, 42, i64::MAX] {
            assert_eq!(events.pick("e", seed), events.pick("e", seed));
        }
        // Different seeds must not all collapse onto the same variant.
        let distinct: std::collections::HashSet<_> =
            (0..100).map(|s| events.pick("e", s).unwrap().id).collect();
        assert!(distinct.len() > 1);
    }

    #[test]
    fn event_references_resolve_and_multiply() {
        let json = br#"{
            "outer": {"sounds": [
                {"name": "inner", "type": "event", "volume": 0.5, "pitch": 2.0}
            ]},
            "inner": {"sounds": [{"name": "file", "volume": 0.8, "pitch": 0.5}]}
        }"#;
        let events = SoundEvents::parse(json).unwrap();
        let v = events.pick("outer", 7).unwrap();
        assert_eq!(v.id.to_string(), "minecraft:file");
        assert_eq!(v.volume, 0.5 * 0.8);
        assert_eq!(v.pitch, 2.0 * 0.5);
    }

    // The expected indices come from `java.util.Random(seed)` (vanilla's
    // `LegacyRandomSource`): a fresh generator's first nextInt(4) is 2 for seed 0, 1 for
    // seed -7 and 0 for seed 1234567; its first nextInt(5) is 0 for seed 0, 3 for seed 2,
    // 4 for seed 3 and 2 for seed -7.
    #[test]
    fn seed_picks_vanillas_variant() {
        let json = br#"{"block.stone.break": {"sounds":
            ["dig/stone1", "dig/stone2", "dig/stone3", "dig/stone4"]}}"#;
        let events = SoundEvents::parse(json).unwrap();
        let path = |seed| {
            events
                .pick("block.stone.break", seed)
                .unwrap()
                .id
                .to_string()
        };
        assert_eq!(path(0), "minecraft:dig/stone3");
        assert_eq!(path(-7), "minecraft:dig/stone2");
        assert_eq!(path(1_234_567), "minecraft:dig/stone1");
    }

    #[test]
    fn seed_walks_the_weights_in_file_order() {
        // Weights 3, 1, 1: index 0..=2 is a, 3 is b, 4 is c.
        let json = br#"{"e": {"sounds": [
            {"name": "a", "weight": 3}, "b", "c"
        ]}}"#;
        let events = SoundEvents::parse(json).unwrap();
        let path = |seed| events.pick("e", seed).unwrap().id.path().to_owned();
        assert_eq!(path(0), "a");
        assert_eq!(path(-7), "a");
        assert_eq!(path(2), "b");
        assert_eq!(path(3), "c");
    }

    #[test]
    fn an_event_reference_weighs_its_whole_event() {
        // `SoundManager.Preparations`: the reference's weight is the referenced event's
        // total (3 here), not its own `weight` field, so the outer total is 4. Seed 0
        // draws nextInt(4) = 2 (the reference) and then nextInt(3) = 1 inside it; seed 3
        // draws 2 and then 2; seed 1234567 draws 0 (the plain file).
        let json = br#"{
            "outer": {"sounds": ["x", {"name": "inner", "type": "event", "weight": 1,
                                       "volume": 0.5, "stream": true}]},
            "inner": {"sounds": ["p", "q", {"name": "r", "volume": 0.8}]}
        }"#;
        let events = SoundEvents::parse(json).unwrap();
        let q = events.pick("outer", 0).unwrap();
        assert_eq!(q.id.path(), "q");
        assert_eq!(q.volume, 0.5);
        assert!(q.stream, "the reference's stream flag is or-ed in");
        let r = events.pick("outer", 3).unwrap();
        assert_eq!(r.id.path(), "r");
        assert_eq!(r.volume, 0.5 * 0.8);
        let x = events.pick("outer", 1_234_567).unwrap();
        assert_eq!(x.id.path(), "x");
        assert!(!x.stream);
    }

    #[test]
    fn a_reference_to_an_unknown_event_weighs_nothing() {
        let json = br#"{"e": {"sounds": [{"name": "missing", "type": "event"}, "a"]}}"#;
        let events = SoundEvents::parse(json).unwrap();
        for seed in [0, 1, 2, 3, -7] {
            assert_eq!(events.pick("e", seed).unwrap().id.path(), "a");
        }
    }

    #[test]
    fn reference_cycle_returns_none() {
        let json = br#"{
            "a": {"sounds": [{"name": "b", "type": "event"}]},
            "b": {"sounds": [{"name": "a", "type": "event"}]}
        }"#;
        let events = SoundEvents::parse(json).unwrap();
        assert_eq!(events.pick("a", 1), None);
    }

    #[test]
    fn missing_files_are_dropped_at_load() {
        // SoundManager.Preparations.handleRegistration / validateSoundResource: an entry
        // whose file does not exist is skipped while loading, so it can never be picked
        // (rather than failing to play later); an event left without entries never plays.
        let json = br#"{
            "broken": {"sounds": ["gone", {"name": "there"}]},
            "all_gone": {"sounds": ["missing1", "missing2"]},
            "by_reference": {"sounds": [{"name": "all_gone", "type": "event"}, "there"]}
        }"#;
        let mut events = SoundEvents::parse(json).unwrap();
        let dropped = events.retain_files(|path| path.contains("/there.ogg"));
        assert_eq!(dropped, 3, "gone, missing1 and missing2 are dropped");
        let broken = events.get("broken").unwrap();
        assert_eq!(broken.sounds.len(), 1, "the existing file stays");
        let SoundEntry::File(v) = &broken.sounds[0] else {
            panic!("expected file entry");
        };
        assert_eq!(v.id.path(), "there");
        // The emptied event can never be picked...
        assert_eq!(events.pick("all_gone", 1), None);
        // ...and neither can it be reached through a reference (its weight is 0).
        assert_eq!(events.pick("by_reference", 1).unwrap().id.path(), "there");
        // An entry with weight 0 counts as no entry at all: nothing changes for it.
        let json = br#"{"z": {"sounds": [{"name": "there", "weight": 0}]}}"#;
        let mut events = SoundEvents::parse(json).unwrap();
        assert_eq!(events.retain_files(|_| true), 0);
    }

    #[test]
    fn unknown_event_returns_none() {
        let events = SoundEvents::parse(br#"{"e": {"sounds": ["x"]}}"#).unwrap();
        assert_eq!(events.pick("nope", 1), None);
        assert!(events.get("nope").is_none());
    }

    #[test]
    fn subtitle_lookup() {
        let json = br#"{"e": {"subtitle": "subtitles.e", "sounds": ["x"]}}"#;
        let events = SoundEvents::parse(json).unwrap();
        assert_eq!(events.subtitle("e"), Some("subtitles.e"));
        assert_eq!(events.subtitle("nope"), None);
    }

    #[test]
    #[ignore = "needs the downloaded client jar and assets in the cache"]
    fn real_sounds_json_parses() {
        let jar = haste_core::paths::client_jar();
        let mut assets =
            haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
        let bytes = assets.read_asset("assets/minecraft/sounds.json").unwrap();
        let events = SoundEvents::parse(&bytes).unwrap();
        assert!(events.len() > 1000, "only {} events", events.len());
        assert!(events.get("block.stone.break").is_some());
        assert!(events.pick("block.stone.break", 42).is_some());
        // `music.creative` starts with a reference to all of `music.game`, so in vanilla
        // most creative tracks are game tracks: the reference outweighs the six creative
        // files many times over.
        let creative = events.get("music.creative").unwrap();
        let reference = events.entry_weight(&creative.sounds[0], 0);
        assert!(reference > 6, "music.game weighs {reference}");
    }
}
