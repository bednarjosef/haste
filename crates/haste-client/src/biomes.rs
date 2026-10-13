//! Biome colours from the server's synced `minecraft:worldgen/biome` registry (fluids-and-tint).
//!
//! Each entry's network NBT carries `temperature`, `downfall` and `effects` with `water_color`
//! and optional `grass_color`, `foliage_color`, `dry_foliage_color` (an int, or a `#rrggbb`
//! string as in the data pack JSON) and `grass_color_modifier` (`none`, `dark_forest`, `swamp`).

use haste_protocol::haste_core::{BiomeId, BlockPos, Identifier};
use haste_protocol::nbt::NbtCompound;
use haste_protocol::SyncedRegistries;
use haste_render::{BiomeClimate, GrassModifier};

/// The world's biomes for the renderer's tint lookup (cell-resolution, biome 0 where nothing is
/// loaded).
pub struct WorldBiomes<'a>(pub &'a haste_world::World);

impl haste_render::BiomeSource for WorldBiomes<'_> {
    fn biome(&self, pos: BlockPos) -> BiomeId {
        self.0
            .chunk(pos.chunk())
            .and_then(|c| c.section(pos.y >> 4))
            .map(|s| {
                s.biome(
                    ((pos.x & 15) >> 2) as u8,
                    ((pos.y & 15) >> 2) as u8,
                    ((pos.z & 15) >> 2) as u8,
                )
            })
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

/// One biome entry's NBT to its climate; missing fields take plains' values.
pub fn climate_from_nbt(c: &NbtCompound) -> BiomeClimate {
    let plains = BiomeClimate::plains();
    let effects = c.get_compound("effects");
    let eff = |name: &str| effects.and_then(|e| color(e, name));
    BiomeClimate {
        temperature: c
            .get_numeric_float("temperature")
            .unwrap_or(plains.temperature),
        downfall: c.get_numeric_float("downfall").unwrap_or(plains.downfall),
        water_color: eff("water_color").unwrap_or(plains.water_color),
        grass_color: eff("grass_color"),
        foliage_color: eff("foliage_color"),
        dry_foliage_color: eff("dry_foliage_color"),
        grass_modifier: match effects.and_then(|e| e.get_string("grass_color_modifier")) {
            Some("dark_forest") => GrassModifier::DarkForest,
            Some("swamp") => GrassModifier::Swamp,
            _ => GrassModifier::None,
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
                water_color: 0x617B64,
                grass_color: None,
                foliage_color: Some(0x6A7039),
                dry_foliage_color: Some(0x7B5334),
                grass_modifier: GrassModifier::Swamp,
            }
        );
    }

    #[test]
    fn empty_entry_is_plains() {
        assert_eq!(
            climate_from_nbt(&NbtCompound::new()),
            BiomeClimate::plains()
        );
    }
}
