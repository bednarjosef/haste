//! Decoding plumbing shared by the component readers: a minimal reader over a stack's raw
//! component-patch bytes, and the `minecraft:dyed_color` data-component-type lookup.

use crate::registries::Registries;

/// The `minecraft:dyed_color` data-component-type id, from the registry report. `None` when
/// the report is absent.
pub(crate) fn dyed_color_type(registries: &Registries) -> Option<i32> {
    registries
        .index(
            "minecraft:data_component_type",
            &haste_core::Identifier::minecraft("dyed_color"),
        )
        .map(|i| i.0 as i32)
}

/// The `minecraft:damage` data-component-type id, from the registry report. `None` when the
/// report is absent.
pub(crate) fn damage_type(registries: &Registries) -> Option<i32> {
    registries
        .index(
            "minecraft:data_component_type",
            &haste_core::Identifier::minecraft("damage"),
        )
        .map(|i| i.0 as i32)
}

/// A minimal varint/int reader over a component patch's raw bytes (only what
/// [`ItemComponents::dyed_color`](super::ItemComponents::dyed_color) and
/// [`ItemComponents::damage`](super::ItemComponents::damage) need).
pub(crate) struct PatchReader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> PatchReader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        PatchReader { bytes, at: 0 }
    }

    pub(crate) fn var_int(&mut self) -> Option<i32> {
        let mut value = 0u32;
        let mut shift = 0;
        loop {
            let byte = *self.bytes.get(self.at)?;
            self.at += 1;
            value |= ((byte & 0x7F) as u32) << shift;
            if byte & 0x80 == 0 {
                return Some(value as i32);
            }
            shift += 7;
            if shift >= 32 {
                return None;
            }
        }
    }

    pub(crate) fn i32(&mut self) -> Option<i32> {
        let b = self.bytes.get(self.at..self.at + 4)?;
        self.at += 4;
        Some(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
}
