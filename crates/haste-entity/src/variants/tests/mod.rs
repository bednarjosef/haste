use super::*;
use haste_core::entity::MetadataEntry;
use haste_core::entity::MetadataValue;
use haste_core::ids::RegistryIndex;

use crate::metadata::Metadata;
use crate::model_dump;

use super::registry_model::layers_for;
use super::synced_registry::{spec_for, variant_id};

/// Test helper: apply another metadata set's entries on top.
trait ApplyExtra {
    fn apply_extra(self, other: Metadata) -> Metadata;
}
impl ApplyExtra for Metadata {
    fn apply_extra(mut self, other: Metadata) -> Metadata {
        use haste_core::entity::MetadataEntry;
        let entries: Vec<MetadataEntry> = other.iter().cloned().collect();
        self.apply(&entries);
        self
    }
}

fn metadata(index: u8, type_id: i32, raw: &[u8]) -> Metadata {
    let mut m = Metadata::default();
    m.apply(&[MetadataEntry {
        index,
        value: MetadataValue::Other {
            type_id,
            raw: raw.to_vec().into(),
        },
    }]);
    m
}

fn int_metadata(entries: &[(u8, i32)]) -> Metadata {
    let mut m = Metadata::default();
    m.apply(
        &entries
            .iter()
            .map(|(i, v)| MetadataEntry {
                index: *i,
                value: MetadataValue::VarInt(*v),
            })
            .collect::<Vec<_>>(),
    );
    m
}

fn byte_metadata(entries: &[(u8, i8)]) -> Metadata {
    let mut m = Metadata::default();
    m.apply(
        &entries
            .iter()
            .map(|(i, v)| MetadataEntry {
                index: *i,
                value: MetadataValue::Byte(*v),
            })
            .collect::<Vec<_>>(),
    );
    m
}

fn bool_metadata(entries: &[(u8, bool)]) -> Metadata {
    let mut m = Metadata::default();
    m.apply(
        &entries
            .iter()
            .map(|(i, v)| MetadataEntry {
                index: *i,
                value: MetadataValue::Boolean(*v),
            })
            .collect::<Vec<_>>(),
    );
    m
}

fn long_metadata(entries: &[(u8, i64)]) -> Metadata {
    let mut m = Metadata::default();
    m.apply(
        &entries
            .iter()
            .map(|(i, v)| MetadataEntry {
                index: *i,
                value: MetadataValue::VarLong(*v),
            })
            .collect::<Vec<_>>(),
    );
    m
}

/// The kind of an entity type name, through the real registries.json (None without the
/// cache: these tests are skipped there).
fn kind_of(name: &str) -> Option<RegistryIndex> {
    crate::registry::entity_type_names()?
        .iter()
        .find(|(_, n)| n.as_str() == name)
        .map(|(k, _)| *k)
}

mod cube_mob;
mod dye;
mod metadata_texture;
mod overlay;
mod registry_model;
mod synced_registry;
