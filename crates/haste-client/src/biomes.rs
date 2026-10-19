//! Biome colours from the server's synced `minecraft:worldgen/biome` registry (fluids-and-tint).
//!
//! Each entry's network NBT carries `temperature`, `downfall` and `effects` with `water_color`
//! and optional `grass_color`, `foliage_color`, `dry_foliage_color` (an int, or a `#rrggbb`
//! string as in the data pack JSON) and `grass_color_modifier` (`none`, `dark_forest`, `swamp`).

// `biomes.rs` is included through a `#[path]` module, so its submodules resolve next to it
// in `src/`, not in `src/biomes/` — point them at their files explicitly.
#[path = "biomes/audio.rs"]
pub mod audio;
#[path = "biomes/zoom.rs"]
pub mod zoom;

use haste_protocol::haste_core::{BiomeId, BlockPos, ChunkPos, Identifier};
use haste_protocol::nbt::NbtCompound;
use haste_protocol::SyncedRegistries;
use haste_render::{BiomeClimate, BiomeFog, GrassModifier};

/// The world's biomes for the renderer's tint lookup (cell-resolution, biome 0 where nothing is
/// loaded).
pub struct WorldBiomes<'a>(pub &'a haste_world::World);

impl haste_render::BiomeSource for WorldBiomes<'_> {
    fn biome(&self, pos: BlockPos) -> BiomeId {
        // Vanilla `BiomeManager.getBiome`: the fiddled nearest corner's cell, which can be
        // one quart cell off the block's own in each axis (crossing a section or chunk
        // border; 0 where that neighbour is not loaded).
        let (qx, qy, qz) = zoom::zoomed_cell(self.0.biome_zoom_seed(), pos.x, pos.y, pos.z);
        self.0
            .chunk(ChunkPos::new(qx >> 2, qz >> 2))
            .and_then(|c| c.section(qy >> 2))
            .map(|s| s.biome((qx & 3) as u8, (qy & 3) as u8, (qz & 3) as u8))
            .unwrap_or(BiomeId(0))
    }
}

/// A colour value: an int (0xRRGGBB, alpha ignored) or a `#rrggbb` string.
fn color(c: &NbtCompound, name: &str) -> Option<u32> {
    if let Some(v) = c.get_int(name) {
        return Some(v as u32 & 0xFFFFFF);
    }
    let s = c.get_string(name)?;
    u32::from_str_radix(s.strip_prefix('#').unwrap_or(s), 16)
        .ok()
        .map(|v| v & 0xFFFFFF)
}

/// One environment attribute from a biome's `attributes` map (vanilla
/// `EnvironmentAttributeMap.NETWORK_CODEC`): the key is the attribute id (`visual/fog_color`),
/// the value an int or `#rrggbb` string colour, or a number for the distances.
fn attribute_color(a: &NbtCompound, name: &str) -> Option<u32> {
    color(a, name)
}

fn attribute_float(a: &NbtCompound, name: &str) -> Option<f32> {
    a.get_numeric_float(name)
}

/// One biome entry's NBT to its climate; missing fields take plains' values.
pub fn climate_from_nbt(c: &NbtCompound) -> BiomeClimate {
    let plains = BiomeClimate::plains();
    let effects = c.get_compound("effects");
    let eff = |name: &str| effects.and_then(|e| color(e, name));
    let attrs = c.get_compound("attributes");
    let attr = |name: &str| attrs.and_then(|a| attribute_color(a, name));
    let attrf = |name: &str| attrs.and_then(|a| attribute_float(a, name));
    BiomeClimate {
        temperature: c
            .get_numeric_float("temperature")
            .unwrap_or(plains.temperature),
        downfall: c.get_numeric_float("downfall").unwrap_or(plains.downfall),
        // `has_precipitation` (26.3's rename of `Biome.hasPrecipitation`); plains has it.
        has_precipitation: c.get_bool("has_precipitation").unwrap_or(true),
        water_color: eff("water_color").unwrap_or(plains.water_color),
        grass_color: eff("grass_color"),
        foliage_color: eff("foliage_color"),
        dry_foliage_color: eff("dry_foliage_color"),
        grass_modifier: match effects.and_then(|e| e.get_string("grass_color_modifier")) {
            Some("dark_forest") => GrassModifier::DarkForest,
            Some("swamp") => GrassModifier::Swamp,
            _ => GrassModifier::None,
        },
        // The attribute map keys are the `minecraft:environment_attribute` registry ids, which
        // carry the `visual/` path under the `minecraft` namespace.
        fog: BiomeFog {
            fog_color: attr("minecraft:visual/fog_color"),
            sky_color: attr("minecraft:visual/sky_color"),
            fog_start: attrf("minecraft:visual/fog_start_distance"),
            fog_end: attrf("minecraft:visual/fog_end_distance"),
            sky_fog_end: attrf("minecraft:visual/sky_fog_end_distance"),
            cloud_fog_end: attrf("minecraft:visual/cloud_fog_end_distance"),
            water_fog_color: attr("minecraft:visual/water_fog_color"),
            water_fog_start: attrf("minecraft:visual/water_fog_start_distance"),
            water_fog_end: attrf("minecraft:visual/water_fog_end_distance"),
        },
    }
}

/// The synced biome registry as climates, index = biome registry id. Entries without data
/// (known packs) or with unreadable NBT become plains.
pub fn biome_climates(regs: &SyncedRegistries) -> Vec<BiomeClimate> {
    let Some(entries) = regs.entries(&Identifier::minecraft("worldgen/biome")) else {
        return Vec::new();
    };
    entries
        .iter()
        .map(|e| {
            let nbt = e.data.as_ref().and_then(|bytes| {
                haste_protocol::wire::Reader::new(bytes)
                    .nbt_compound()
                    .ok()
                    .flatten()
            });
            match nbt {
                Some(c) => climate_from_nbt(&c),
                None => {
                    tracing::warn!("biome {} has no readable data: plains colours", e.id);
                    BiomeClimate::plains()
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swamp_entry_converts() {
        let mut effects = NbtCompound::new();
        effects.put_string("water_color", "#617b64".into());
        effects.put_string("foliage_color", "#6a7039".into());
        effects.put_int("dry_foliage_color", 0x7b5334);
        effects.put_string("grass_color_modifier", "swamp".into());
        let mut c = NbtCompound::new();
        c.put_float("temperature", 0.8);
        c.put_float("downfall", 0.9);
        c.put_compound("effects", effects);
        let climate = climate_from_nbt(&c);
        assert_eq!(
            climate,
            BiomeClimate {
                temperature: 0.8,
                downfall: 0.9,
                has_precipitation: true,
                water_color: 0x617B64,
                grass_color: None,
                foliage_color: Some(0x6A7039),
                dry_foliage_color: Some(0x7B5334),
                grass_modifier: GrassModifier::Swamp,
                fog: BiomeFog::default(),
            }
        );
    }

    #[test]
    fn attributes_parse_into_fog() {
        let mut attrs = NbtCompound::new();
        attrs.put_string("minecraft:visual/fog_color", "#330808".into());
        attrs.put_string("minecraft:visual/water_fog_color", "#3f76e4".into());
        attrs.put_float("minecraft:visual/fog_start_distance", 10.0);
        attrs.put_float("minecraft:visual/fog_end_distance", 96.0);
        let mut c = NbtCompound::new();
        c.put_compound("attributes", attrs);
        let fog = climate_from_nbt(&c).fog;
        assert_eq!(fog.fog_color, Some(0x330808));
        assert_eq!(fog.water_fog_color, Some(0x3F76E4));
        assert_eq!(fog.fog_start, Some(10.0));
        assert_eq!(fog.fog_end, Some(96.0));
        assert_eq!(fog.sky_fog_end, None);
    }

    #[test]
    fn empty_entry_is_plains() {
        assert_eq!(
            climate_from_nbt(&NbtCompound::new()),
            BiomeClimate::plains()
        );
    }
}
