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

use super::{Codec, DecodeError, DecodeResult, Reader, Wire, Writer};

impl Wire for ItemStackData {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let count = r.var_int()?;
        if count <= 0 {
            return Ok(ItemStackData { count: 0, item: None, components_raw: Bytes::new() });
        }
        let item = r.var_int()?;
        let start = r.peek_rest();
        skip_patch(r)?;
        let used = start.len() - r.remaining();
        let components_raw = r.to_bytes(&start[..used]);
        Ok(ItemStackData { count, item: Some(RegistryIndex(item as u32)), components_raw })
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
    fn read(r: &mut Reader<'_>) -> DecodeResult<ItemStackData> { ItemStackData::read(r) }
    fn write(v: &ItemStackData, w: &mut Writer) { v.write(w) }
}

/// Skips a data component patch (add count, remove count, adds, removes).
pub fn skip_patch(r: &mut Reader<'_>) -> DecodeResult<()> {
    let adds = r.len_prefix(4096)?;
    let removes = r.len_prefix(4096)?;
    for _ in 0..adds {
        let ty = r.var_int()?;
        let name = component_name(r, ty)?;
        skip_component(r, name)?;
    }
    for _ in 0..removes {
        r.var_int()?;
    }
    Ok(())
}

fn component_name<'a>(r: &Reader<'a>, ty: i32) -> DecodeResult<&'a str> {
    let tables = r.tables().ok_or_else(|| DecodeError::Invalid(format!("data component {ty} needs registry tables")))?;
    tables
        .data_component_name(ty)
        .ok_or_else(|| DecodeError::Invalid(format!("unknown data component type id {ty}")))
}

/// Skips one component's data by its `minecraft:data_component_type` name.
pub fn skip_component(r: &mut Reader<'_>, name: &str) -> DecodeResult<()> {
    let short = name.strip_prefix("minecraft:").unwrap_or(name);
    match short {
        // No data.
        "unbreakable" | "creative_slot_lock" | "glider" => {}
        // A single VarInt (counts, enums, registry ids, colours as int below).
        "max_stack_size" | "max_damage" | "damage" | "rarity" | "damage_type" | "additional_trade_cost" | "repair_cost" | "enchantable" | "map_id"
        | "map_post_processing" | "ominous_bottle_amplifier" | "base_color" | "dye" | "villager/variant" | "wolf/variant"
        | "wolf/sound_variant" | "wolf/collar" | "fox/variant" | "salmon/size" | "parrot/variant"
        | "tropical_fish/pattern" | "tropical_fish/base_color" | "tropical_fish/pattern_color" | "mooshroom/variant"
        | "rabbit/variant" | "pig/variant" | "pig/sound_variant" | "cow/variant" | "cow/sound_variant"
        | "chicken/variant" | "chicken/sound_variant" | "zombie_nautilus/variant" | "frog/variant" | "horse/variant"
        | "painting/variant" | "llama/variant" | "axolotl/variant" | "cat/variant" | "cat/sound_variant"
        | "cat/collar" | "sheep/color" | "shulker/color" | "cushion/color" => {
            r.var_int()?;
        }
        // Int.
        "dyed_color" => {
            r.i32()?;
        }
        // Boolean.
        "enchantment_glint_override" => {
            r.bool()?;
        }
        // One network NBT value.
        "custom_data" | "intangible_projectile" | "bucket_entity_data" | "debug_stick_state" | "lock" | "recipes"
        | "map_decorations" | "container_loot" => {
            r.nbt_raw()?;
        }
        // Registry id + NBT.
        "entity_data" | "block_entity_data" => {
            r.var_int()?;
            r.nbt_raw()?;
        }
        // Text component.
        "custom_name" | "item_name" => {
            r.nbt_raw()?;
        }
        // Prefixed array of text components.
        "lore" => {
            let n = r.len_prefix(256)?;
            for _ in 0..n {
                r.nbt_raw()?;
            }
        }
        // Identifier.
        "item_model" | "tooltip_style" | "note_block_sound" => {
            r.identifier()?;
        }
        // Prefixed array of (VarInt id, VarInt level).
        "enchantments" | "stored_enchantments" => {
            let n = r.len_prefix(1024)?;
            for _ in 0..n {
                r.var_int()?;
                r.var_int()?;
            }
        }
        "custom_model_data" => {
            r.list(super::MAX_LIST, |r| r.f32())?;
            r.list(super::MAX_LIST, |r| r.bool())?;
            r.list(super::MAX_LIST, |r| r.string())?;
            r.list(super::MAX_LIST, |r| r.i32())?;
        }
        "tooltip_display" => {
            r.bool()?;
            r.list(super::MAX_LIST, |r| r.var_int())?;
        }
        "food" => {
            r.var_int()?;
            r.f32()?;
            r.bool()?;
        }
        "use_cooldown" => {
            r.f32()?;
            r.option(|r| r.identifier())?;
        }
        "use_remainder" => {
            ItemStackData::read(r)?;
        }
        "charged_projectiles" | "bundle_contents" | "container" => {
            r.list(256, ItemStackData::read)?;
        }
        "attribute_modifiers" => {
            // Wiki: attribute VarInt, modifier id Identifier, value Double,
            // operation VarInt, slot VarInt, then a display (26.x): not verified.
            return Err(unsupported(name));
        }
        _ => return Err(unsupported(name)),
    }
    Ok(())
}

fn unsupported(name: &str) -> DecodeError {
    DecodeError::Invalid(format!("data component {name} layout not implemented"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_plain_stacks_round_trip() {
        let empty = ItemStackData::default();
        let plain = ItemStackData { count: 3, item: Some(RegistryIndex(42)), components_raw: Bytes::new() };
        let mut w = Writer::new();
        w.write(&empty);
        w.write(&plain);
        assert_eq!(w.buf, vec![0, 3, 42, 0, 0]);
        let mut r = Reader::new(&w.buf);
        assert!(r.read::<ItemStackData>().unwrap().is_empty());
        let back = r.read::<ItemStackData>().unwrap();
        assert_eq!(back.count, 3);
        assert_eq!(back.item, Some(RegistryIndex(42)));
        assert_eq!(&back.components_raw[..], &[0, 0]);
        r.finish().unwrap();
    }

    #[test]
    fn components_need_tables_and_known_layouts() {
        let tables = crate::packets::ids::ProtocolTables::for_tests(&[(3, "minecraft:damage"), (60, "minecraft:mystery")]);
        // count 1, item 7, 1 add, 1 remove, damage=5, remove type 4
        let bytes = [1u8, 7, 1, 1, 3, 5, 4];
        let mut r = Reader::new(&bytes).with_tables(&tables);
        let s = r.read::<ItemStackData>().unwrap();
        assert_eq!(&s.components_raw[..], &[1, 1, 3, 5, 4]);
        r.finish().unwrap();
        assert!(Reader::new(&bytes).read::<ItemStackData>().is_err());
        let unknown = [1u8, 7, 1, 0, 60, 0];
        assert!(Reader::new(&unknown).with_tables(&tables).read::<ItemStackData>().is_err());
    }
}
