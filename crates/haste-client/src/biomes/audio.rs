//! The `audio/music_volume` environment attribute (vanilla
//! `EnvironmentAttributes.MUSIC_VOLUME`): the target the music fade eases the music gain
//! toward, read off the camera like vanilla's `Minecraft.getMusicVolume`.
//!
//! The attribute is a plain float (default 1, unit range), positional but **not** spatially
//! interpolated, so the biome is the raw stored quart cell at the camera's block position
//! (vanilla `BiomeManager.getNoiseBiomeAtPosition`) — no zoom fiddle and no blending,
//! unlike the fog and sky colours. Vanilla's `EnvironmentAttributeSystem` layers it
//! dimension-type constant first, then biome, and clamps the result into 0..=1; no
//! vanilla dimension type, timeline or weather layer carries it, and of the biomes only
//! the pale garden sets it (0).

use glam::DVec3;
use haste_core::game::DimensionInfo;
use haste_core::{BiomeId, ChunkPos, Identifier};
use haste_protocol::nbt::NbtCompound;
use haste_protocol::wire::Reader;
use haste_protocol::SyncedRegistries;

use super::attribute_float;

/// The `MUSIC_VOLUME` environment attribute's default (vanilla
/// `EnvironmentAttributes.MUSIC_VOLUME`), the target outside the biomes that lower it.
pub const MUSIC_VOLUME_ATTRIBUTE_DEFAULT: f32 = 1.0;

/// One biome's audio environment attributes: the `audio/*` entries of its attribute map
/// (vanilla `EnvironmentAttributeMap`). `music_volume` is the map's override entry,
/// absent (`None`) when the biome carries none — then the dimension type's value shows
/// through, as in vanilla's layering.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BiomeAudio {
    pub music_volume: Option<f32>,
}

/// The synced biome registry as audio attributes, index = biome registry id. An entry
/// without readable NBT carries no entries (the dimension value shows through).
pub fn biome_audio(regs: &SyncedRegistries) -> Vec<BiomeAudio> {
    let Some(entries) = regs.entries(&Identifier::minecraft("worldgen/biome")) else {
        return Vec::new();
    };
    entries
        .iter()
        .map(|e| BiomeAudio {
            music_volume: e
                .data
                .as_ref()
                .and_then(|bytes| Reader::new(bytes).nbt_compound().ok().flatten())
                .and_then(|c| music_volume_entry(&c)),
        })
        .collect()
}

/// The dimension type's `audio/music_volume` (vanilla's constant dimension layer,
/// `EnvironmentAttributeSystem.addDimensionLayer`): the attribute's default when the
/// type's map carries no entry — no vanilla dimension sets it.
pub fn dimension_music_volume(regs: &SyncedRegistries, dimension: &DimensionInfo) -> f32 {
    let reg = Identifier::minecraft("dimension_type");
    regs.index_of(&reg, &dimension.dimension_type)
        .and_then(|i| regs.entry_nbt(&reg, i))
        .and_then(|nbt| music_volume_entry(&nbt))
        .unwrap_or(MUSIC_VOLUME_ATTRIBUTE_DEFAULT)
}

/// The camera's `MUSIC_VOLUME` (vanilla `Minecraft.getMusicVolume` without its screen
/// rule — no Haste screen carries background music yet): the dimension type's value as
/// the base, overridden by the biome at the camera's block position, clamped to the
/// attribute's unit range. The biome is the raw stored quart cell because the attribute
/// is not spatially interpolated; an id the synced table does not know (mods) keeps the
/// dimension value, and where no chunk is loaded the cell reads biome 0 like the tint
/// lookup. No allocation.
pub fn camera_music_volume(
    world: &haste_world::World,
    biomes: &[BiomeAudio],
    camera: DVec3,
    dimension: f32,
) -> f32 {
    // `QuartPos.fromBlock(Mth.floor(pos))`: the quart cell holding the block position.
    let (x, y, z) = (camera.x.floor(), camera.y.floor(), camera.z.floor());
    let (qx, qy, qz) = (x as i32 >> 2, y as i32 >> 2, z as i32 >> 2);
    let biome = world
        .chunk(ChunkPos::new(qx >> 2, qz >> 2))
        .and_then(|c| c.section(qy >> 2))
        .map(|s| s.biome((qx & 3) as u8, (qy & 3) as u8, (qz & 3) as u8))
        .unwrap_or(BiomeId(0));
    biomes
        .get(biome.0 as usize)
        .and_then(|a| a.music_volume)
        .unwrap_or(dimension)
        .clamp(0.0, 1.0)
}

/// One attribute map's `minecraft:audio/music_volume`: a synced value is the override
/// form, a plain number (the pale garden's 0); a modifier-form compound is not a plain
/// number and reads as absent rather than guessed at.
fn music_volume_entry(c: &NbtCompound) -> Option<f32> {
    attribute_float(
        c.get_compound("attributes")?,
        "minecraft:audio/music_volume",
    )
}

#[cfg(test)]
#[path = "audio/tests.rs"]
mod tests;
