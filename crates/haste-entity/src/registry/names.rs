//! Loads the `minecraft:entity_type` names from the data-generator reports (see `registry`).

use std::collections::HashMap;

use haste_core::ids::RegistryIndex;
use serde::Deserialize;

/// `minecraft:entity_type` names by protocol id, from the data-generator reports
/// (`haste_core::paths::datagen_reports`, owned by world-and-data). haste-entity reads the one
/// registry it needs itself: haste-data's `Registries::parse` is not exported and pulling all
/// of `GameData` in here would duplicate the client's load. Every entity type is kept (the
/// decoration kinds need names the model table does not cover); `model_entry` filters.
pub(super) fn load_entity_type_names() -> Option<HashMap<RegistryIndex, &'static str>> {
    let path = haste_core::paths::datagen_reports().join("registries.json");
    let bytes = std::fs::read(path).ok()?;
    let registries: HashMap<&str, EntityTypeRegistryJson> = serde_json::from_slice(&bytes).ok()?;
    let reg = registries.get("minecraft:entity_type")?;
    let mut names = HashMap::with_capacity(reg.entries.len());
    for (name, e) in &reg.entries {
        names.insert(
            RegistryIndex(e.protocol_id),
            Box::leak((*name).to_owned().into_boxed_str()) as &'static str,
        );
    }
    Some(names)
}

#[derive(Deserialize)]
struct EntityTypeRegistryJson {
    entries: HashMap<String, EntryId>,
}

#[derive(Deserialize)]
struct EntryId {
    protocol_id: u32,
}
