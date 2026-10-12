//! Packet ids and the few registry tables the codec needs, loaded at runtime
//! from the data generator reports (`cache/datagen/26.3/reports`). Nothing
//! here is hard-coded: ids come from `packets.json`, data component type ids
//! from `registries.json`.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

/// The five connection states.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum State {
    /// Initial state: one `intention` packet.
    Handshake,
    /// Server list ping.
    Status,
    /// Authentication, encryption, compression.
    Login,
    /// Registries, tags, resource packs.
    Configuration,
    /// The game.
    Play,
}

impl State {
    /// All states, in protocol order.
    pub const ALL: [State; 5] = [State::Handshake, State::Status, State::Login, State::Configuration, State::Play];

    /// The key used in `packets.json`.
    pub fn as_str(self) -> &'static str {
        match self {
            State::Handshake => "handshake",
            State::Status => "status",
            State::Login => "login",
            State::Configuration => "configuration",
            State::Play => "play",
        }
    }
}

/// Direction of a packet relative to the client.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Direction {
    /// Server to client.
    Clientbound,
    /// Client to server.
    Serverbound,
}

impl Direction {
    /// The key used in `packets.json`.
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Clientbound => "clientbound",
            Direction::Serverbound => "serverbound",
        }
    }
}

#[derive(Deserialize)]
struct Entry {
    protocol_id: i32,
}

#[derive(Default, Clone, Debug)]
struct Table {
    by_id: Vec<Option<String>>,
    by_name: HashMap<String, i32>,
}

fn key(state: State, dir: Direction) -> usize {
    let s = State::ALL.iter().position(|x| *x == state).unwrap_or(0);
    s * 2 + matches!(dir, Direction::Serverbound) as usize
}

/// Packet id tables for every state and direction, keyed by the full
/// `minecraft:` packet name.
#[derive(Clone, Debug, Default)]
pub struct PacketIds {
    tables: [Table; 10],
}

impl PacketIds {
    /// Loads `packets.json` from `haste_core::paths::datagen_reports()`.
    pub fn load() -> anyhow::Result<PacketIds> {
        Self::load_from(&haste_core::paths::datagen_reports().join("packets.json"))
    }

    /// Loads from an explicit `packets.json` path.
    pub fn load_from(path: &Path) -> anyhow::Result<PacketIds> {
        let text = std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
        Self::from_json(&text).map_err(|e| anyhow::anyhow!("parsing {}: {e}", path.display()))
    }

    /// Parses the contents of `packets.json`.
    pub fn from_json(text: &str) -> anyhow::Result<PacketIds> {
        let parsed: HashMap<String, HashMap<String, HashMap<String, Entry>>> = serde_json::from_str(text)?;
        let mut ids = PacketIds::default();
        for state in State::ALL {
            let Some(dirs) = parsed.get(state.as_str()) else { continue };
            for dir in [Direction::Clientbound, Direction::Serverbound] {
                let Some(entries) = dirs.get(dir.as_str()) else { continue };
                let t = &mut ids.tables[key(state, dir)];
                for (name, e) in entries {
                    if e.protocol_id < 0 {
                        anyhow::bail!("negative id for {name}");
                    }
                    let i = e.protocol_id as usize;
                    if t.by_id.len() <= i {
                        t.by_id.resize(i + 1, None);
                    }
                    t.by_id[i] = Some(name.clone());
                    t.by_name.insert(name.clone(), e.protocol_id);
                }
            }
        }
        Ok(ids)
    }

    /// The id of a packet (`minecraft:level_chunk_with_light`).
    pub fn id(&self, state: State, dir: Direction, name: &str) -> Option<i32> {
        self.tables[key(state, dir)].by_name.get(name).copied()
    }

    /// The name of a packet id.
    pub fn name(&self, state: State, dir: Direction, id: i32) -> Option<&str> {
        let t = &self.tables[key(state, dir)];
        usize::try_from(id).ok().and_then(|i| t.by_id.get(i)).and_then(|n| n.as_deref())
    }

    /// Number of packets in a state/direction.
    pub fn len(&self, state: State, dir: Direction) -> usize {
        self.tables[key(state, dir)].by_name.len()
    }

    /// Whether no packets are known at all.
    pub fn is_empty(&self) -> bool {
        self.tables.iter().all(|t| t.by_name.is_empty())
    }

    /// All `(id, name)` pairs of a state/direction, by id.
    pub fn iter(&self, state: State, dir: Direction) -> impl Iterator<Item = (i32, &str)> {
        self.tables[key(state, dir)].by_id.iter().enumerate().filter_map(|(i, n)| n.as_deref().map(|n| (i as i32, n)))
    }
}

/// Everything the codec needs from the reports: packet ids and the
/// `minecraft:data_component_type` registry (item stack decoding).
#[derive(Clone, Debug, Default)]
pub struct ProtocolTables {
    /// Packet ids.
    pub packets: PacketIds,
    data_components: Vec<String>,
    particle_types: Vec<String>,
}

impl ProtocolTables {
    /// Loads `packets.json` and `registries.json` from the reports directory.
    pub fn load() -> anyhow::Result<ProtocolTables> {
        Self::load_from(&haste_core::paths::datagen_reports())
    }

    /// Loads from an explicit reports directory.
    pub fn load_from(reports: &Path) -> anyhow::Result<ProtocolTables> {
        let packets = PacketIds::load_from(&reports.join("packets.json"))?;
        let reg_path = reports.join("registries.json");
        let text = std::fs::read_to_string(&reg_path).map_err(|e| anyhow::anyhow!("reading {}: {e}", reg_path.display()))?;
        #[derive(Deserialize)]
        struct Reg {
            entries: HashMap<String, Entry>,
        }
        let regs: HashMap<String, Reg> = serde_json::from_str(&text)?;
        let by_id = |name: &str| -> Vec<String> {
            let mut v = Vec::new();
            if let Some(r) = regs.get(name) {
                for (entry, e) in &r.entries {
                    let i = e.protocol_id as usize;
                    if i >= v.len() {
                        v.resize(i + 1, String::new());
                    }
                    v[i] = entry.clone();
                }
            }
            v
        };
        Ok(ProtocolTables {
            packets,
            data_components: by_id("minecraft:data_component_type"),
            particle_types: by_id("minecraft:particle_type"),
        })
    }

    /// Name of a `minecraft:particle_type` id.
    pub fn particle_name(&self, id: i32) -> Option<&str> {
        usize::try_from(id).ok().and_then(|i| self.particle_types.get(i)).map(String::as_str).filter(|s| !s.is_empty())
    }

    /// Name of a data component type id.
    pub fn data_component_name(&self, id: i32) -> Option<&str> {
        usize::try_from(id).ok().and_then(|i| self.data_components.get(i)).map(String::as_str).filter(|s| !s.is_empty())
    }

    /// Replaces the packet ids (tests and tools).
    pub fn with_packets(mut self, packets: PacketIds) -> Self {
        self.packets = packets;
        self
    }

    /// Adds particle type names (tests).
    pub fn with_particles_for_tests(mut self, particles: &[(i32, &str)]) -> Self {
        for (i, n) in particles {
            let i = *i as usize;
            if self.particle_types.len() <= i {
                self.particle_types.resize(i + 1, String::new());
            }
            self.particle_types[i] = (*n).to_string();
        }
        self
    }

    /// Tables with the given data component names (tests).
    pub fn for_tests(components: &[(i32, &str)]) -> ProtocolTables {
        let mut t = ProtocolTables::default();
        for (i, n) in components {
            let i = *i as usize;
            if t.data_components.len() <= i {
                t.data_components.resize(i + 1, String::new());
            }
            t.data_components[i] = (*n).to_string();
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_snippet() {
        let json = r#"{"login":{"clientbound":{"minecraft:hello":{"protocol_id":1},"minecraft:login_compression":{"protocol_id":3}},
                       "serverbound":{"minecraft:hello":{"protocol_id":0}}}}"#;
        let ids = PacketIds::from_json(json).unwrap();
        assert_eq!(ids.id(State::Login, Direction::Clientbound, "minecraft:hello"), Some(1));
        assert_eq!(ids.id(State::Login, Direction::Clientbound, "minecraft:login_compression"), Some(3));
        assert_eq!(ids.id(State::Login, Direction::Clientbound, "minecraft:nope"), None);
        assert_eq!(ids.name(State::Login, Direction::Serverbound, 0), Some("minecraft:hello"));
        assert_eq!(ids.name(State::Login, Direction::Clientbound, 2), None);
        assert_eq!(ids.name(State::Login, Direction::Clientbound, -1), None);
        assert_eq!(ids.len(State::Login, Direction::Clientbound), 2);
        assert_eq!(ids.len(State::Play, Direction::Clientbound), 0);
        assert_eq!(ids.iter(State::Login, Direction::Clientbound).collect::<Vec<_>>(), vec![(1, "minecraft:hello"), (3, "minecraft:login_compression")]);
    }

    #[test]
    #[ignore = "needs cache/datagen/26.3/reports (run with --ignored)"]
    fn loads_real_reports() {
        let t = ProtocolTables::load().expect("reports load");
        let ids = &t.packets;
        assert_eq!(ids.id(State::Handshake, Direction::Serverbound, "minecraft:intention"), Some(0));
        assert_eq!(ids.len(State::Play, Direction::Clientbound), 144);
        assert_eq!(ids.len(State::Play, Direction::Serverbound), 69);
        assert_eq!(ids.len(State::Configuration, Direction::Clientbound), 21);
        let ka = ids.id(State::Play, Direction::Clientbound, "minecraft:keep_alive").unwrap();
        assert_eq!(ids.name(State::Play, Direction::Clientbound, ka), Some("minecraft:keep_alive"));
        assert_eq!(t.data_component_name(0), Some("minecraft:custom_data"));
        assert_eq!(t.particle_name(1), Some("minecraft:block"));
        assert!(t.data_component_name(10_000).is_none());
    }
}
