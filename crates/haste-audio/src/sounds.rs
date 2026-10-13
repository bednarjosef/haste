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
    },
}

impl SoundEntry {
    fn weight(&self) -> u32 {
        match self {
            SoundEntry::File(v) => v.weight,
            SoundEntry::Event { weight, .. } => *weight,
        }
    }
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

/// The Ogg path of a sound name: `dig/stone1` -> `sounds/dig/stone1.ogg`.
fn sound_path(id: &Identifier) -> String {
    format!("sounds/{}.ogg", id.path())
}

impl SoundEvents {
    /// Parses `assets/minecraft/sounds.json`. Keys are event names without namespace;
    /// the `minecraft` namespace is applied to them.
    pub fn parse(json: &[u8]) -> anyhow::Result<SoundEvents> {
        let raw: HashMap<String, RawEvent> =
            serde_json::from_slice(json).map_err(|e| anyhow::anyhow!("parsing sounds.json: {e}"))?;
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

    /// Picks one variant of the event, weighted by each entry's `weight` and driven by
    /// the seed: the same seed always picks the same variant. `type: "event"` entries
    /// are followed (up to 8 levels; deeper references and cycles give `None`), with
    /// the reference's `volume`/`pitch` multiplying into the picked variant.
    ///
    /// Vanilla's exact random sequence is not reproduced: this is our own deterministic
    /// RNG, so the chosen variant for a given seed need not match the vanilla client.
    pub fn pick(&self, event: &str, seed: i64) -> Option<SoundVariant> {
        let mut rng = Rng::new(seed);
        self.pick_at(event, &mut rng, 0)
    }

    fn pick_at(&self, event: &str, rng: &mut Rng, depth: u32) -> Option<SoundVariant> {
        if depth > 8 {
            return None;
        }
        let ev = self.get(event)?;
        let total: u32 = ev.sounds.iter().map(SoundEntry::weight).sum();
        if total == 0 {
            return None;
        }
        let mut ticket = (rng.next() % total as u64) as u32;
        let entry = ev.sounds.iter().find(|s| {
            let w = s.weight();
            if ticket < w {
                true
            } else {
                ticket -= w;
                false
            }
        })?;
        match entry {
            SoundEntry::File(v) => Some(v.clone()),
            SoundEntry::Event {
                name,
                volume,
                pitch,
                ..
            } => {
                let mut v = self.pick_at(name, rng, depth + 1)?;
                v.volume *= volume;
                v.pitch *= pitch;
                Some(v)
            }
        }
    }
}

/// `foo:bar/baz` or `bar/baz` (namespace `minecraft`) as an [`Identifier`].
fn parse_sound_name(name: &str) -> anyhow::Result<Identifier> {
    Identifier::parse(name).map_err(|e| anyhow::anyhow!("sound name {name:?}: {e}"))
}

/// A tiny deterministic xorshift64* RNG: same seed, same sequence, always.
struct Rng(u64);

impl Rng {
    fn new(seed: i64) -> Self {
        // 0 would lock xorshift at 0.
        Rng(if seed as u64 == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed as u64
        })
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
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
        assert_eq!(v.path, "sounds/dig/stone1.ogg");
        assert_eq!((v.volume, v.pitch, v.weight), (1.0, 1.0, 1));
        assert_eq!((v.stream, v.attenuation_distance, v.preload), (false, 16, false));
        let SoundEntry::File(v) = &ev.sounds[1] else {
            panic!("expected file entry");
        };
        assert_eq!((v.volume, v.weight, v.stream), (1.0, 1, false));
        let SoundEntry::File(v) = &ev.sounds[2] else {
            panic!("expected file entry");
        };
        assert_eq!((v.volume, v.pitch, v.weight), (0.8, 1.4, 2));
        assert_eq!((v.stream, v.attenuation_distance, v.preload), (true, 8, true));
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
        assert_eq!(v.path, "sounds/bar/baz.ogg");
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
    }
}
