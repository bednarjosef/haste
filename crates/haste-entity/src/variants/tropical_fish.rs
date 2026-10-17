//! The tropical fish's shape and pattern texture (`TropicalFishRenderer`).
use std::sync::Arc;

use haste_core::entity::MetadataValue;
use haste_core::model::CubeModel;

use crate::metadata::Metadata;

use super::metadata_index;
use super::overlay::dump_layer_with_texture;
/// The tropical fish's base model: the shape its packed variant names (`Base.SMALL` uses
/// `tropical_fish_small` with `tropical_a`, `Base.LARGE` the large shape with `tropical_b`)
/// — `TropicalFishRenderer.submit` swaps the model with the pattern's base. The base colour
/// tint (`getModelTint`) is not applied yet: the base draw has no tint slot, a later
/// EntityInstance change; the pattern overlay carries its own colour today.
pub fn tropical_fish_model(metadata: &Metadata) -> Option<Arc<CubeModel>> {
    let packed = match metadata
        .get(metadata_index::TROPICAL_FISH_VARIANT)
        .map(|e| &e.value)
    {
        Some(MetadataValue::VarInt(i)) => *i,
        _ => return None,
    };
    let large = packed >> 8 & 0xFF == 1;
    let (layer, texture) = if large {
        (
            "minecraft:tropical_fish_large#main",
            "entity/fish/tropical_b",
        )
    } else {
        (
            "minecraft:tropical_fish_small#main",
            "entity/fish/tropical_a",
        )
    };
    dump_layer_with_texture(layer, texture)
}

/// `TropicalFishPatternLayer`'s texture per packed pattern id (`base | index << 8`).
pub(super) fn tropical_pattern_texture(pattern: i32) -> String {
    let index = pattern & 0xFF;
    let large = pattern >> 8 & 1 == 1;
    let (letter, index) = if large { ("b", index) } else { ("a", index) };
    format!("entity/fish/tropical_{letter}_pattern_{}", index + 1)
}
