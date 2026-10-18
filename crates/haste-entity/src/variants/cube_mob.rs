//! The slime and magma cube size (`AbstractCubeMobRenderer`).
use haste_core::entity::MetadataValue;
use haste_core::ids::RegistryIndex;

use crate::metadata::Metadata;

use super::metadata_index;
/// The rest-pose scale of a slime or magma cube (`AbstractCubeMobRenderer.applySizeAndSquish`
/// with no squish: uniform `size`), or 1.0 for every other type. The squish animation's
/// non-uniform wobble is part of the animation task, not the variant.
///
/// Cheap on the frame path: the metadata probe comes first, so every non-cube entity (the
/// common case) returns without a registry lookup.
pub fn scale_for(kind: RegistryIndex, metadata: &Metadata) -> f32 {
    let size = match metadata.get(metadata_index::CUBE_SIZE).map(|e| &e.value) {
        Some(MetadataValue::VarInt(i)) if *i > 0 => *i,
        _ => return 1.0,
    };
    let Some(name) = crate::registry::entity_type_name(kind) else {
        return 1.0;
    };
    if !matches!(name, "minecraft:slime" | "minecraft:magma_cube") {
        return 1.0;
    }
    size as f32
}
