//! Item stacks (`Slot`) and their data component patches.
//!
//! The wire format (minecraft.wiki "Slot Data"): count VarInt; if positive,
//! item id VarInt, number of components to add, number to remove, the added
//! components as (type id, data) and the removed type ids. Component data has
//! no length prefix, so skipping a component needs its layout: this module
//! knows the layouts in [`skip_component`] by registry name (resolved through
//! `minecraft:data_component_type` from `registries.json`) and keeps the whole
//! patch raw in `ItemStackData::components_raw`. A component whose layout is
//! not implemented fails the decode with a message naming it.

use bytes::Bytes;
use haste_core::entity::ItemStackData;
use haste_core::RegistryIndex;

use super::{Codec, DecodeResult, Reader, Wire, Writer};

impl Wire for ItemStackData {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let count = r.var_int()?;
        if count <= 0 {
            return Ok(ItemStackData {
                count: 0,
                item: None,
                components_raw: Bytes::new(),
            });
        }
        let item = r.var_int()?;
        let start = r.peek_rest();
        skip_patch(r)?;
        let used = start.len() - r.remaining();
        let components_raw = r.to_bytes(&start[..used]);
        Ok(ItemStackData {
            count,
            item: Some(RegistryIndex(item as u32)),
            components_raw,
        })
    }
    fn write(&self, w: &mut Writer) {
        match (self.count, self.item) {
            (c, Some(item)) if c > 0 => {
                w.var_int(c);
                w.var_int(item.0 as i32);
                if self.components_raw.is_empty() {
                    w.var_int(0);
                    w.var_int(0);
                } else {
                    w.bytes(&self.components_raw);
                }
            }
            _ => w.var_int(0),
        }
    }
}

/// Item stack encoding where the empty stack is allowed (the default `Slot`).
pub struct Slot;
impl Codec<ItemStackData> for Slot {
    fn read(r: &mut Reader<'_>) -> DecodeResult<ItemStackData> {
        ItemStackData::read(r)
    }
    fn write(v: &ItemStackData, w: &mut Writer) {
        v.write(w)
    }
}

/// Item stack template (26.x): `VarInt item, VarInt count, component patch`.
/// Unlike a [`Slot`] it is never empty and the item comes before the count.
/// Used inside `container`, `bundle_contents`, `charged_projectiles`,
/// `use_remainder` and `sulfur_cube_content`, and for advancement icons and
/// recipe display results (layout from Pumpkin's `write_template0`, confirmed
/// on the 26.3 vanilla server).
pub struct ItemStackTemplate;
impl Codec<ItemStackData> for ItemStackTemplate {
    fn read(r: &mut Reader<'_>) -> DecodeResult<ItemStackData> {
        let item = r.var_int()?;
        let count = r.var_int()?;
        let start = r.peek_rest();
        skip_patch(r)?;
        let used = start.len() - r.remaining();
        Ok(ItemStackData {
            count,
            item: Some(RegistryIndex(item as u32)),
            components_raw: r.to_bytes(&start[..used]),
        })
    }
    fn write(v: &ItemStackData, w: &mut Writer) {
        w.var_int(v.item.map_or(0, |i| i.0 as i32));
        w.var_int(v.count);
        if v.components_raw.is_empty() {
            w.var_int(0);
            w.var_int(0);
        } else {
            w.bytes(&v.components_raw);
        }
    }
}

/// Optional item stack template: `bool present`, then an [`ItemStackTemplate`].
/// Absent reads as the empty stack.
pub struct OptionalItemStackTemplate;
impl Codec<ItemStackData> for OptionalItemStackTemplate {
    fn read(r: &mut Reader<'_>) -> DecodeResult<ItemStackData> {
        if r.bool()? {
            ItemStackTemplate::read(r)
        } else {
            Ok(ItemStackData::default())
        }
    }
    fn write(v: &ItemStackData, w: &mut Writer) {
        let present = v.count > 0 && v.item.is_some();
        w.bool(present);
        if present {
            ItemStackTemplate::write(v, w);
        }
    }
}

mod patch;

pub(crate) use patch::{component_name, skip_consume_effect};
pub use patch::{skip_component, skip_exact_components, skip_patch};

#[cfg(test)]
mod tests;
