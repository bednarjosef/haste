//! Entity metadata ("synched entity data"): raw entries kept per entity, the shared indices
//! interpreted.
//!
//! Shared indices are the base `Entity` class's slots (per minecraft.wiki, "Entity format" /
//! metadata tables): index 0 is the shared flags byte, index 6 the `Pose` (modern versions;
//! older layouts differ). Anything a specific entity class adds is stored raw and interpreted
//! later by whoever needs it.
use haste_core::entity::{MetadataEntry, MetadataValue, Pose};

/// The shared flags byte (metadata index 0).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct SharedFlags {
    pub on_fire: bool,
    pub crouching: bool,
    pub riding: bool,
    pub invisible: bool,
    pub glowing: bool,
    pub elytra_flying: bool,
}

impl SharedFlags {
    fn from_byte(b: i8) -> SharedFlags {
        let b = b as u8;
        SharedFlags {
            on_fire: b & 0x01 != 0,
            crouching: b & 0x02 != 0,
            riding: b & 0x04 != 0,
            invisible: b & 0x10 != 0,
            glowing: b & 0x40 != 0,
            elytra_flying: b & 0x80 != 0,
        }
    }
}

/// Per-entity metadata: interpreted shared fields plus every raw entry, indexed.
#[derive(Default)]
pub struct Metadata {
    pub shared: SharedFlags,
    pub pose: Option<Pose>,
    /// All entries (including the shared ones), kept so later consumers can read the rest.
    entries: std::vec::Vec<MetadataEntry>,
}

impl Metadata {
    pub fn apply(&mut self, entries: &[MetadataEntry]) {
        for e in entries {
            if let Some(slot) = self.entries.iter_mut().find(|x| x.index == e.index) {
                *slot = e.clone();
            } else {
                self.entries.push(e.clone());
            }
        }
        for e in entries {
            match (e.index, &e.value) {
                (0, MetadataValue::Byte(b)) => self.shared = SharedFlags::from_byte(*b),
                (6, MetadataValue::Pose(p)) => self.pose = Some(*p),
                _ => {}
            }
        }
    }

    /// The stored entry for an index, if any.
    pub fn get(&self, index: u8) -> Option<&MetadataEntry> {
        self.entries.iter().find(|e| e.index == index)
    }

    pub fn iter(&self) -> impl Iterator<Item = &MetadataEntry> {
        self.entries.iter()
    }
}
