//! `registries.json`: the static registries (entity types, items, ...).

use std::collections::BTreeMap;

use ahash::HashMapExt as _;

use haste_core::{Identifier, RegistryIndex};

pub struct Registries {
    registries: Vec<Registry>,
    by_name: ahash::HashMap<Box<str>, u32>,
}

struct Registry {
    /// Indexed by protocol id; validated dense 0..len at load.
    entries: Vec<Identifier>,
    /// Full identifier string (`ns:path`) -> protocol id.
    by_entry: ahash::HashMap<Box<str>, u32>,
}

impl Registries {
    pub(crate) fn parse(json: &str) -> anyhow::Result<Self> {
        let raw: BTreeMap<&str, RawRegistry> =
            serde_json::from_str(json).map_err(|e| anyhow::anyhow!("registries.json: {e}"))?;
        let mut registries = Vec::with_capacity(raw.len());
        let mut by_name: ahash::HashMap<Box<str>, u32> = ahash::HashMap::with_capacity(raw.len());
        for (name, r) in &raw {
            let n = r.entries.len();
            let mut slots: Vec<Option<Identifier>> = vec![None; n];
            let mut by_entry: ahash::HashMap<Box<str>, u32> = ahash::HashMap::with_capacity(n);
            for (ename, e) in &r.entries {
                let id = e.protocol_id as usize;
                if id >= n {
                    anyhow::bail!(
                        "registries.json: {name}: protocol_id {} out of range (0..{n})",
                        e.protocol_id
                    );
                }
                if slots[id].is_some() {
                    anyhow::bail!(
                        "registries.json: {name}: duplicate protocol_id {}",
                        e.protocol_id
                    );
                }
                slots[id] = Some(
                    Identifier::parse(ename)
                        .map_err(|e| anyhow::anyhow!("registries.json: {name}: {e}"))?,
                );
                by_entry.insert((*ename).into(), e.protocol_id);
            }
            let entries: Option<Vec<_>> = slots.into_iter().collect();
            let entries = entries.ok_or_else(|| {
                anyhow::anyhow!("registries.json: {name}: protocol ids are not dense 0..{n}")
            })?;
            by_name.insert((*name).into(), registries.len() as u32);
            registries.push(Registry { entries, by_entry });
        }
        Ok(Self {
            registries,
            by_name,
        })
    }

    /// Index of `entry` in `registry`, e.g. `("minecraft:entity_type", "minecraft:zombie")`.
    pub fn index(&self, registry: &str, entry: &Identifier) -> Option<RegistryIndex> {
        let r = self.registries.get(*self.by_name.get(registry)? as usize)?;
        let key = entry.to_string();
        r.by_entry.get(key.as_str()).map(|&id| RegistryIndex(id))
    }

    /// Name of entry `i` in `registry`.
    pub fn name(&self, registry: &str, i: RegistryIndex) -> Option<&Identifier> {
        let r = self.registries.get(*self.by_name.get(registry)? as usize)?;
        r.entries.get(i.0 as usize)
    }

    /// Number of entries in `registry`; 0 if the registry is unknown.
    pub fn len(&self, registry: &str) -> usize {
        self.by_name
            .get(registry)
            .and_then(|&i| self.registries.get(i as usize))
            .map_or(0, |r| r.entries.len())
    }

    /// `minecraft:data_component_type` names by protocol id (empty when the registry is
    /// absent): the ids an item stack's component patch indexes.
    pub fn data_component_names(&self) -> Vec<String> {
        self.by_name
            .get("minecraft:data_component_type")
            .and_then(|&i| self.registries.get(i as usize))
            .map(|r| r.entries.iter().map(ToString::to_string).collect())
            .unwrap_or_default()
    }
}

#[derive(serde::Deserialize)]
struct RawRegistry<'a> {
    #[serde(borrow, default)]
    entries: BTreeMap<&'a str, RawEntry>,
}

#[derive(serde::Deserialize)]
struct RawEntry {
    protocol_id: u32,
}
