//! The `GameData` loaders: the runtime report list, the missing-report check and the
//! `load`/`load_from` pair that fill [`GameData`] from the cache (block states, registries,
//! shapes, physics, sounds, item components).

use std::path::Path;

use crate::blocks::BlockStates;
use crate::components::ItemComponents;
use crate::physics::BlockProperties;
use crate::registries::Registries;
use crate::shapes::BlockShapes;
use crate::sounds::BlockSoundTypes;

/// All game data, loaded once at startup and shared (`Arc`) with the world, render, physics.
pub struct GameData {
    pub blocks: BlockStates,
    pub registries: Registries,
    pub shapes: BlockShapes,
    pub physics: BlockProperties,
    pub sounds: BlockSoundTypes,
    /// Item default components (the `equippable` component armour needs), from the report's
    /// `minecraft/components/item` directory. Empty when the report is absent.
    pub item_components: ItemComponents,
}

/// The data-generator reports the client reads at runtime, as paths relative to the reports
/// directory (`haste_core::paths::datagen_reports`). `haste setup` must produce all of them
/// (`haste_setup::install::REQUIRED_REPORTS`); without `minecraft/components/item` armour and
/// item data draw nothing. `minecraft/components/item` is a directory.
pub const RUNTIME_REPORTS: &[&str] = &[
    "blocks.json",
    "registries.json",
    "minecraft/components/item",
];

/// The entries of [`RUNTIME_REPORTS`] missing under `reports` (a directory counts as present).
pub fn missing_reports(reports: &Path) -> Vec<&'static str> {
    RUNTIME_REPORTS
        .iter()
        .copied()
        .filter(|r| !reports.join(r).exists())
        .collect()
}

impl GameData {
    /// Loads `datagen/26.3/reports/{blocks,registries}.json`, the item components report and
    /// `oracle/26.3/block_shapes.json` (optional, fallback shapes otherwise) from
    /// `haste_core::paths`. Warns once, naming `haste setup`, for every [`RUNTIME_REPORTS`] entry
    /// missing (so a report-less install fails loudly instead of drawing nothing).
    pub fn load() -> anyhow::Result<GameData> {
        let reports = haste_core::paths::datagen_reports();
        let missing = missing_reports(&reports);
        if !missing.is_empty() {
            tracing::warn!(
                "data generator report(s) missing under {}: {} — run `haste setup` \
                 (armour and item data draw nothing without the item components report)",
                reports.display(),
                missing.join(", ")
            );
        }
        let oracle = haste_core::paths::oracle_dir().join("block_shapes.json");
        let oracle = if oracle.is_file() { Some(oracle) } else { None };
        Self::load_from(&reports, oracle.as_deref())
    }

    /// Loads from explicit paths. `oracle` is the `block_shapes.json` path, if present;
    /// `block_properties.json` and `block_sounds.json` are looked up next to it when they
    /// exist.
    pub fn load_from(reports: &Path, oracle: Option<&Path>) -> anyhow::Result<GameData> {
        let blocks_json = std::fs::read_to_string(reports.join("blocks.json"))?;
        let registries_json = std::fs::read_to_string(reports.join("registries.json"))?;
        let registries = Registries::parse(&registries_json)?;
        let blocks = BlockStates::parse(&blocks_json, Some(&registries))?;
        let shapes = match oracle {
            Some(p) => BlockShapes::parse_oracle(&std::fs::read_to_string(p)?, &blocks)?,
            None => BlockShapes::fallback(&blocks),
        };
        let sibling = |name: &str| oracle.map(|p| p.with_file_name(name));
        let physics = match sibling("block_properties.json") {
            Some(p) if p.is_file() => {
                BlockProperties::parse(&std::fs::read_to_string(p)?, &blocks)?
            }
            _ => BlockProperties::defaults(&blocks),
        };
        let sounds = match sibling("block_sounds.json") {
            Some(p) if p.is_file() => {
                BlockSoundTypes::parse(&std::fs::read_to_string(p)?, &blocks)?
            }
            _ => BlockSoundTypes::empty(&blocks),
        };
        let item_components = ItemComponents::load(
            &reports.join("minecraft").join("components").join("item"),
            &registries,
        );
        Ok(Self {
            blocks,
            registries,
            shapes,
            physics,
            sounds,
            item_components,
        })
    }
}
