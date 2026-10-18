// The registry lookups around the synced registries the join needs: resolving registry ids to
// names (entity types, attributes), the per-id tables the renderers read (painting variants,
// mob variant assets), the static game data load, and the biome/world-clock registry sizes the
// packets depend on.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use haste_core::Identifier;
use haste_data::GameData;
use haste_protocol::nbt::NbtCompound;
use haste_protocol::SyncedRegistries;

/// The `minecraft:entity_type` registry entries in id order: the synced registry's when the
/// server sent one (its entry order defines the ids), else the static one from the game data
/// (`registries.json`, the data generator's own order; the two agree on vanilla servers).
pub fn entity_type_names(regs: &SyncedRegistries, data: &GameData) -> Vec<Identifier> {
    if let Some(entries) = regs.entries(&Identifier::minecraft("entity_type")) {
        return entries.iter().map(|e| e.id.clone()).collect();
    }
    let n = data.registries.len("minecraft:entity_type");
    (0..n)
        .filter_map(|i| {
            data.registries
                .name("minecraft:entity_type", haste_core::RegistryIndex(i as u32))
                .cloned()
        })
        .collect()
}

/// The `minecraft:attribute` registry entries in id order (the synced registry's when the
/// server sent one, else the static one from the game data): resolves an `update_attributes`
/// property's index to its name, for the attributes a steered mount's simulation reads.
pub fn attribute_names(regs: &SyncedRegistries, data: &GameData) -> Vec<Identifier> {
    if let Some(entries) = regs.entries(&Identifier::minecraft("attribute")) {
        return entries.iter().map(|e| e.id.clone()).collect();
    }
    let n = data.registries.len("minecraft:attribute");
    (0..n)
        .filter_map(|i| {
            data.registries
                .name("minecraft:attribute", haste_core::RegistryIndex(i as u32))
                .cloned()
        })
        .collect()
}

/// The synced `minecraft:painting_variant` registry (id -> width/height/asset) for the
/// `PaintingRenderer`: the server sends every variant's data in `registry_data`
/// (`RegistryDataLoader.SYNCHRONIZED_REGISTRIES`), so paintings get their size and sprite
/// without a hard-coded table. Empty when the server sent none (unknown -> back texture).
pub fn painting_variants(regs: &SyncedRegistries) -> HashMap<u32, haste_entity::PaintingVariant> {
    let name = Identifier::minecraft("painting_variant");
    let mut out = HashMap::new();
    if let Some(entries) = regs.entries(&name) {
        for i in 0..entries.len() {
            let Some(nbt) = regs.entry_nbt(&name, i) else {
                continue;
            };
            let (Some(width), Some(height), Some(asset)) = (
                nbt.get_int("width"),
                nbt.get_int("height"),
                nbt.get_string("asset_id"),
            ) else {
                continue;
            };
            if let Ok(asset) = Identifier::parse(asset) {
                out.insert(
                    i as u32,
                    haste_entity::PaintingVariant {
                        width: width.clamp(1, 16) as u8,
                        height: height.clamp(1, 16) as u8,
                        asset,
                    },
                );
            }
        }
    }
    out
}

/// The variant registries the variant mobs read (`haste_entity::VARIANT_REGISTRIES`), each
/// parsed into haste-entity's per-id asset table: `asset_id`/`baby_asset_id` (and `model`
/// for pig/cow/chicken; wolf nests `assets`/`baby_assets` with `wild`/`tame`/`angry` — the
/// wild texture is what an untamed, calm mob shows). An entry whose NBT is absent (a known
/// pack) or does not parse stays `None`: the mob keeps the model table's default look.
/// Returns the registry name and its table, one per registry the server sent.
pub fn mob_variants(
    regs: &SyncedRegistries,
) -> Vec<(String, Vec<Option<haste_entity::VariantAssets>>)> {
    let parse = |nbt: &NbtCompound| -> Option<haste_entity::VariantAssets> {
        if nbt.get_string("asset_id").is_some() {
            haste_entity::VariantAssets::from_assets(
                nbt.get_string("asset_id").unwrap(),
                nbt.get_string("baby_asset_id"),
                nbt.get_string("model"),
            )
        } else {
            // Wolf: assets/baby_assets, each with wild/tame/angry; batch 1 reads the wild
            // texture (tame/angry are TamableAnimal flag textures, a later task).
            let wild = nbt.get_compound("assets")?.get_string("wild")?;
            let baby_wild = nbt.get_compound("baby_assets")?.get_string("wild")?;
            haste_entity::VariantAssets::from_assets(wild, Some(baby_wild), None)
        }
    };
    let mut out = Vec::new();
    for wanted in haste_entity::VARIANT_REGISTRIES {
        let Some(registry) = regs.names().find(|n| n.to_string() == *wanted) else {
            continue;
        };
        let Some(entries) = regs.entries(registry) else {
            continue;
        };
        let table = (0..entries.len())
            .map(|i| regs.entry_nbt(registry, i).as_ref().and_then(parse))
            .collect();
        out.push(((*wanted).to_owned(), table));
    }
    out
}

/// Loads the game data (block states, registries, shapes) once for the whole client.
pub fn load_game_data() -> anyhow::Result<Arc<GameData>> {
    let t = std::time::Instant::now();
    let data = GameData::load().context(
        "loading game data from the data generator reports (run scripts/rebuild-cache.sh)",
    )?;
    tracing::info!(
        "game data: {} block states in {:.0} ms",
        data.blocks.len(),
        t.elapsed().as_secs_f64() * 1000.0
    );
    Ok(Arc::new(data))
}

/// Number of entries in the synced `minecraft:worldgen/biome` registry (the direct biome
/// palette's bit width depends on it).
pub fn synced_biome_count(regs: &SyncedRegistries) -> Option<usize> {
    regs.entries(&Identifier::minecraft("worldgen/biome"))
        .map(|e| e.len())
}

/// Registry id of `minecraft:overworld` in `minecraft:world_clock` (the day clock; the End
/// has `minecraft:the_end`). 0 (the overworld clock, first registered) when the server sent
/// no such registry.
pub fn overworld_clock_id(regs: &SyncedRegistries) -> i32 {
    regs.entries(&Identifier::minecraft("world_clock"))
        .and_then(|entries| {
            entries
                .iter()
                .position(|e| e.id == Identifier::minecraft("overworld"))
        })
        .map_or(0, |i| i as i32)
}
