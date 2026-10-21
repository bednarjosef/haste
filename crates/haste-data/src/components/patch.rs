//! Decoding plumbing shared by the component readers: a minimal reader over a stack's raw
//! component patch (`DataComponentPatch.STREAM_CODEC`: the add and remove counts, then each
//! added component as its type id and its network value), and the data-component-type ids of
//! the few components it can decode.

use crate::registries::Registries;

/// The data-component-type ids (from the registry report) of the components whose network
/// value [`PatchTypes::scan`] can decode or skip. Each is `None` when the report is absent.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PatchTypes {
    /// `minecraft:dyed_color` (`DyedItemColor.STREAM_CODEC`: a plain int).
    dyed_color: Option<i32>,
    /// `minecraft:damage` (a varint).
    damage: Option<i32>,
    /// `minecraft:max_damage` (a varint).
    max_damage: Option<i32>,
    /// `minecraft:unbreakable` (`Unit.STREAM_CODEC`: no bytes).
    unbreakable: Option<i32>,
}

/// What [`PatchTypes::scan`] read from a patch's adds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SimpleAdds {
    pub dyed_color: Option<u32>,
    pub damage: Option<u32>,
    pub max_damage: Option<u32>,
    pub unbreakable: bool,
}

impl PatchTypes {
    pub(crate) fn load(registries: &Registries) -> PatchTypes {
        let id = |name: &str| {
            registries
                .index(
                    "minecraft:data_component_type",
                    &haste_core::Identifier::minecraft(name),
                )
                .map(|i| i.0 as i32)
        };
        PatchTypes {
            dyed_color: id("dyed_color"),
            damage: id("damage"),
            max_damage: id("max_damage"),
            unbreakable: id("unbreakable"),
        }
    }

    /// Reads the patch's adds in order while each is a component this reader knows the wire
    /// size of; the first other component ends the scan (its value cannot be skipped), so
    /// anything after it is not seen. No allocation.
    pub(crate) fn scan(&self, components_raw: &[u8]) -> SimpleAdds {
        let mut out = SimpleAdds::default();
        let mut r = PatchReader::new(components_raw);
        let Some(adds) = r.var_int() else {
            return out;
        };
        if r.var_int().is_none() {
            return out;
        }
        for _ in 0..adds.max(0) {
            let Some(ty) = r.var_int() else { break };
            let ty = Some(ty);
            if ty == self.dyed_color {
                match r.i32() {
                    Some(c) => out.dyed_color = Some(c as u32),
                    None => break,
                }
            } else if ty == self.damage {
                match r.var_int() {
                    Some(d) => out.damage = Some(d.max(0) as u32),
                    None => break,
                }
            } else if ty == self.max_damage {
                match r.var_int() {
                    Some(m) => out.max_damage = Some(m.max(0) as u32),
                    None => break,
                }
            } else if ty == self.unbreakable {
                out.unbreakable = true;
            } else {
                break;
            }
        }
        out
    }
}

/// A minimal varint/int reader over a component patch's raw bytes.
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
