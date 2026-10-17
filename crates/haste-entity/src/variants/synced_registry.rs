//! The synchronized variant registries (cat, cow, wolf, frog, pig, chicken): which
//! metadata field holds each mob's `Holder<XVariant>` id and the tables the server sent.
use std::collections::HashMap;

use haste_core::entity::MetadataValue;
use haste_core::ids::RegistryIndex;

use crate::metadata::Metadata;

use super::{metadata_index, serializer_id, VariantAssets};
/// A single entry of the variant spec: how to read the variant out of one entity type's
/// metadata, and which synced registry the ids index.
pub(super) struct VariantSpec {
    /// `minecraft:*` entity type name.
    entity: &'static str,
    /// Metadata index of the `Holder<XVariant>` field.
    index: u8,
    /// The metadata serializer id of the field (to tell variant fields from other `Other`
    /// values on the same index shape).
    serializer: i32,
    /// The synced registry name (`minecraft:pig_variant`).
    registry: &'static str,
}

impl VariantSpec {
    const fn new(
        entity: &'static str,
        index: u8,
        serializer: i32,
        registry: &'static str,
    ) -> VariantSpec {
        VariantSpec {
            entity,
            index,
            serializer,
            registry,
        }
    }
}

/// The six registry-driven variant mobs. Order follows vanilla's `EntityDataSerializers`
/// registration (cat, cow, wolf, frog, pig, chicken).
const SPECS: &[VariantSpec] = &[
    VariantSpec::new(
        "minecraft:cat",
        metadata_index::CAT,
        serializer_id::CAT_VARIANT,
        "minecraft:cat_variant",
    ),
    VariantSpec::new(
        "minecraft:cow",
        metadata_index::COW,
        serializer_id::COW_VARIANT,
        "minecraft:cow_variant",
    ),
    VariantSpec::new(
        "minecraft:wolf",
        metadata_index::WOLF,
        serializer_id::WOLF_VARIANT,
        "minecraft:wolf_variant",
    ),
    VariantSpec::new(
        "minecraft:frog",
        metadata_index::FROG,
        serializer_id::FROG_VARIANT,
        "minecraft:frog_variant",
    ),
    VariantSpec::new(
        "minecraft:pig",
        metadata_index::PIG,
        serializer_id::PIG_VARIANT,
        "minecraft:pig_variant",
    ),
    VariantSpec::new(
        "minecraft:chicken",
        metadata_index::CHICKEN,
        serializer_id::CHICKEN_VARIANT,
        "minecraft:chicken_variant",
    ),
];

/// The variant registries this module reads, in `EntityDataSerializers` registration order
/// (cat, cow, wolf, frog, pig, chicken). `haste-client` parses exactly these from the
/// server's `registry_data`.
pub const VARIANT_REGISTRIES: &[&str] = &[
    "minecraft:cat_variant",
    "minecraft:cow_variant",
    "minecraft:wolf_variant",
    "minecraft:frog_variant",
    "minecraft:pig_variant",
    "minecraft:chicken_variant",
];

/// The variant spec for an entity type name, when the type is one of the six.
pub(super) fn spec_for(entity: &str) -> Option<&'static VariantSpec> {
    SPECS.iter().find(|s| s.entity == entity)
}

/// The variant spec for a registry kind, resolved through the entity-type names.
pub(super) fn spec_for_kind(kind: RegistryIndex) -> Option<&'static VariantSpec> {
    spec_for(crate::registry::entity_type_name(kind)?.as_str())
}

/// The per-mob variant tables, set once at join. Each entry id maps to its resolved assets;
/// a missing entry means the id is unknown and the default row of the model table applies.
#[derive(Default)]
pub struct MobVariants {
    /// Registry id -> assets, per variant registry (the spec's `registry` name).
    tables: HashMap<&'static str, Vec<Option<VariantAssets>>>,
}

impl MobVariants {
    /// An empty table set (no server registries: every mob keeps its default look).
    pub fn empty() -> MobVariants {
        MobVariants::default()
    }

    /// Replaces one variant registry's table (entries in id order; `None` for an id whose
    /// entry is absent or did not parse — a known pack or a mod's shape: the default look).
    pub fn insert(&mut self, registry: &str, entries: Vec<Option<VariantAssets>>) {
        if let Some(spec) = SPECS.iter().find(|s| s.registry == registry) {
            self.tables.insert(spec.registry, entries);
        }
    }

    /// The assets of the variant an entity's metadata names. `None` when the type has no
    /// variants, the metadata has no variant field yet, the id is not in the synced table or
    /// the entry did not parse: callers keep the model table's default row.
    pub fn assets<'a>(
        &'a self,
        kind: RegistryIndex,
        metadata: &Metadata,
    ) -> Option<&'a VariantAssets> {
        let spec = spec_for_kind(kind)?;
        let id = variant_id(metadata, spec)?;
        self.tables.get(spec.registry)?.get(id as usize)?.as_ref()
    }
}

/// The variant registry id in an entity's metadata, or `None` when the field is absent or
/// malformed (`ByteBufCodecs.holderRegistry` writes a plain VarInt id, no offset).
pub(super) fn variant_id(metadata: &Metadata, spec: &VariantSpec) -> Option<u32> {
    let entry = metadata.get(spec.index)?;
    let (type_id, raw) = match &entry.value {
        MetadataValue::Other { type_id, raw } => (*type_id, raw),
        _ => return None,
    };
    if type_id != spec.serializer {
        return None;
    }
    let (id, rest) = read_var_int(raw)?;
    if !rest.is_empty() || id < 0 {
        return None;
    }
    Some(id as u32)
}

/// Little-endian VarInt (the wire encoding the metadata decoder sliced into `raw`).
fn read_var_int(bytes: &[u8]) -> Option<(i32, &[u8])> {
    let mut value: u32 = 0;
    let mut shift = 0;
    for (i, b) in bytes.iter().enumerate() {
        value |= ((b & 0x7F) as u32) << shift;
        if b & 0x80 == 0 {
            return Some((value as i32, &bytes[i + 1..]));
        }
        shift += 7;
        if shift > 28 {
            return None;
        }
    }
    None
}
