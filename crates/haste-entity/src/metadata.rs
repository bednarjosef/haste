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

/// `Mob.DATA_MOB_FLAGS_ID` (index 15: `Entity`'s 8 slots 0..7, `LivingEntity`'s 7 slots 8..14,
/// then `Mob`'s own first `defineId`; `Mob.java:101`). Only the bits the client reads are kept:
/// `MOB_FLAG_LEFTHANDED = 2` and `MOB_FLAG_AGGRESSIVE = 4` (`Mob.isLeftHanded`/`isAggressive`,
/// `Mob.java:1326-1332`).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct MobFlags {
    pub aggressive: bool,
    pub left_handed: bool,
}

impl MobFlags {
    fn from_byte(b: i8) -> MobFlags {
        let b = b as u8;
        MobFlags {
            aggressive: b & 0x04 != 0,
            left_handed: b & 0x02 != 0,
        }
    }
}

/// Per-entity metadata: interpreted shared fields plus every raw entry, indexed.
#[derive(Default)]
pub struct Metadata {
    pub shared: SharedFlags,
    pub pose: Option<Pose>,
    /// The `Mob` flags byte (index 15): aggressive / left-handed.
    pub mob: MobFlags,
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
                (15, MetadataValue::Byte(b)) => self.mob = MobFlags::from_byte(*b),
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
