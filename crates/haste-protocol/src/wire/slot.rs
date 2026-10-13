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

use super::{
    Codec, DecodeError, DecodeResult, IdOr, IdSet, Reader, ResolvableProfile, SoundEvent, Wire,
    Writer, MAX_LIST,
};

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

/// Skips an exact component list (Prefixed Array of type id + data), as in
/// `Trade Item`.
pub fn skip_exact_components(r: &mut Reader<'_>) -> DecodeResult<()> {
    let n = r.len_prefix(4096)?;
    for _ in 0..n {
        let ty = r.var_int()?;
        let name = component_name(r, ty)?;
        skip_component(r, name)?;
    }
    Ok(())
}

fn component_name<'a>(r: &Reader<'a>, ty: i32) -> DecodeResult<&'a str> {
    let tables = r.tables().ok_or_else(|| {
        DecodeError::Invalid(format!("data component {ty} needs registry tables"))
    })?;
    tables
        .data_component_name(ty)
        .ok_or_else(|| DecodeError::Invalid(format!("unknown data component type id {ty}")))
}

/// Skips one component's data by its `minecraft:data_component_type` name.
/// Layouts from the wiki "Slot Data" page; components the page does not
/// document (or documents doubtfully) fail with "layout not implemented".
pub fn skip_component(r: &mut Reader<'_>, name: &str) -> DecodeResult<()> {
    let short = name.strip_prefix("minecraft:").unwrap_or(name);
    match short {
        // No data.
        "unbreakable" | "creative_slot_lock" | "glider" => {}
        // A single VarInt (counts, enums, registry ids, dye colours).
        "max_stack_size"
        | "max_damage"
        | "damage"
        | "rarity"
        | "damage_type"
        | "additional_trade_cost"
        | "repair_cost"
        | "enchantable"
        | "map_id"
        | "map_post_processing"
        | "ominous_bottle_amplifier"
        | "base_color"
        | "dye"
        | "villager/variant"
        | "wolf/variant"
        | "wolf/sound_variant"
        | "wolf/collar"
        | "fox/variant"
        | "salmon/size"
        | "parrot/variant"
        | "tropical_fish/pattern"
        | "tropical_fish/base_color"
        | "tropical_fish/pattern_color"
        | "mooshroom/variant"
        | "rabbit/variant"
        | "pig/variant"
        | "pig/sound_variant"
        | "cow/variant"
        | "cow/sound_variant"
        | "chicken/variant"
        | "chicken/sound_variant"
        | "zombie_nautilus/variant"
        | "frog/variant"
        | "horse/variant"
        | "llama/variant"
        | "axolotl/variant"
        | "cat/variant"
        | "cat/sound_variant"
        | "cat/collar"
        | "sheep/color"
        | "shulker/color" => {
            r.var_int()?;
        }
        // Int.
        "dyed_color" => {
            r.i32()?;
        }
        // Float.
        "minimum_attack_charge" | "potion_duration_scale" => {
            r.f32()?;
        }
        // Boolean.
        "enchantment_glint_override" => {
            r.bool()?;
        }
        // One network NBT value.
        "custom_data"
        | "intangible_projectile"
        | "bucket_entity_data"
        | "debug_stick_state"
        | "lock"
        | "recipes"
        | "map_decorations"
        | "container_loot" => {
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
        // ID Set.
        "damage_resistant" | "repairable" | "provides_banner_patterns" => {
            IdSet::read(r)?;
        }
        // ID or Sound Event.
        "break_sound" => {
            IdOr::<SoundEvent>::read(r)?;
        }
        // Item stack template.
        "use_remainder" | "sulfur_cube_content" => {
            ItemStackTemplate::read(r)?;
        }
        // Prefixed array of (VarInt id, VarInt level).
        "enchantments" | "stored_enchantments" => {
            let n = r.len_prefix(1024)?;
            for _ in 0..n {
                r.var_int()?;
                r.var_int()?;
            }
        }
        "can_place_on" | "can_break" => {
            r.list(MAX_LIST, |r| skip_block_predicate(r))?;
        }
        "custom_model_data" => {
            r.list(MAX_LIST, |r| r.f32())?;
            r.list(MAX_LIST, |r| r.bool())?;
            r.list(MAX_LIST, |r| r.string())?;
            r.list(MAX_LIST, |r| r.i32())?;
        }
        "tooltip_display" => {
            r.bool()?;
            r.list(MAX_LIST, |r| r.var_int())?;
        }
        "food" => {
            r.var_int()?;
            r.f32()?;
            r.bool()?;
        }
        "consumable" => {
            r.f32()?;
            r.var_int()?;
            IdOr::<SoundEvent>::read(r)?;
            r.bool()?;
            r.list(MAX_LIST, |r| skip_consume_effect(r))?;
        }
        "use_cooldown" => {
            r.f32()?;
            r.option(|r| r.identifier())?;
        }
        "use_effects" => {
            r.bool()?;
            r.bool()?;
            r.f32()?;
        }
        "attack_range" => {
            for _ in 0..6 {
                r.f32()?;
            }
        }
        "tool" => {
            r.list(MAX_LIST, |r| {
                IdSet::read(r)?;
                r.option(|r| r.f32())?;
                r.option(|r| r.bool())?;
                Ok(())
            })?;
            r.f32()?;
            r.var_int()?;
            r.bool()?;
        }
        "weapon" => {
            r.var_int()?;
            r.f32()?;
        }
        // Slot, equip sound, asset, camera overlay, allowed entities, then
        // five bools (dispensable, swappable, damage on hurt, equip on
        // interact, can be sheared) and the shearing sound. The fifth bool
        // was observed on the 26.3 vanilla server (diamond helmet with
        // `equippable={slot:"head"}`); Pumpkin agrees.
        "equippable" => {
            r.var_int()?;
            IdOr::<SoundEvent>::read(r)?;
            r.option(|r| r.identifier())?;
            r.option(|r| r.identifier())?;
            r.option(|r| IdSet::read(r))?;
            for _ in 0..5 {
                r.bool()?;
            }
            IdOr::<SoundEvent>::read(r)?;
        }
        "death_protection" => {
            r.list(MAX_LIST, |r| skip_consume_effect(r))?;
        }
        "blocks_attacks" => {
            r.f32()?;
            r.f32()?;
            r.list(MAX_LIST, |r| {
                r.f32()?;
                r.option(|r| IdSet::read(r))?;
                r.f32()?;
                r.f32()
            })?;
            r.f32()?;
            r.f32()?;
            r.f32()?;
            r.option(|r| IdSet::read(r))?;
            r.option(|r| IdOr::<SoundEvent>::read(r))?;
            r.option(|r| IdOr::<SoundEvent>::read(r))?;
        }
        "piercing_weapon" => {
            r.bool()?;
            r.bool()?;
            r.option(|r| SoundEvent::read(r))?;
            r.option(|r| SoundEvent::read(r))?;
        }
        "kinetic_weapon" => {
            r.var_int()?;
            r.var_int()?;
            for _ in 0..3 {
                r.option(|r| {
                    r.var_int()?;
                    r.f32()?;
                    r.f32()
                })?;
            }
            r.f32()?;
            r.f32()?;
            r.option(|r| SoundEvent::read(r))?;
            r.option(|r| SoundEvent::read(r))?;
        }
        "charged_projectiles" | "bundle_contents" => {
            r.list(MAX_LIST, ItemStackTemplate::read)?;
        }
        "container" => {
            r.list(256, OptionalItemStackTemplate::read)?;
        }
        "potion_contents" => {
            r.option(|r| r.var_int())?;
            r.option(|r| r.i32())?;
            r.list(MAX_LIST, |r| skip_potion_effect(r))?;
            r.option(|r| r.string())?;
        }
        "suspicious_stew_effects" => {
            r.list(MAX_LIST, |r| {
                r.var_int()?;
                r.var_int()
            })?;
        }
        "writable_book_content" => {
            r.list(100, |r| {
                r.string_max(1024)?;
                r.option(|r| r.string_max(1024))
            })?;
        }
        "written_book_content" => {
            r.string_max(32)?;
            r.option(|r| r.string_max(32))?;
            r.string()?;
            r.var_int()?;
            r.list(100, |r| {
                r.nbt_raw()?;
                r.option(|r| r.nbt_raw())
            })?;
            r.bool()?;
        }
        "trim" => {
            skip_id_or(r, skip_trim_material)?;
            skip_id_or(r, skip_trim_pattern)?;
        }
        "provides_trim_material" => {
            skip_id_or(r, skip_trim_material)?;
        }
        "instrument" => {
            skip_id_or(r, |r| {
                IdOr::<SoundEvent>::read(r)?;
                r.f32()?;
                r.f32()?;
                r.nbt_raw()?;
                Ok(())
            })?;
        }
        "jukebox_playable" => {
            skip_id_or(r, |r| {
                IdOr::<SoundEvent>::read(r)?;
                r.nbt_raw()?;
                r.f32()?;
                r.var_int()?;
                Ok(())
            })?;
        }
        "lodestone_tracker" => {
            r.option(|r| {
                r.identifier()?;
                r.block_pos()
            })?;
            r.bool()?;
        }
        "firework_explosion" => {
            skip_firework_explosion(r)?;
        }
        "fireworks" => {
            r.var_int()?;
            r.list(MAX_LIST, |r| skip_firework_explosion(r))?;
        }
        "profile" => {
            ResolvableProfile::read(r)?;
        }
        "banner_patterns" => {
            r.list(MAX_LIST, |r| {
                skip_id_or(r, |r| {
                    r.identifier()?;
                    r.string()?;
                    Ok(())
                })?;
                r.var_int()
            })?;
        }
        // Four optional item stack templates (back, left, right, front), no
        // count prefix. Observed on the 26.3 vanilla server (the
        // `craft_decorated_pot_using_only_sherds` advancement icon); the wiki
        // and Pumpkin still have a list of item ids.
        "pot_decorations" => {
            for _ in 0..4 {
                OptionalItemStackTemplate::read(r)?;
            }
        }
        "block_state" => {
            r.list(MAX_LIST, |r| {
                r.string()?;
                r.string()
            })?;
        }
        "bees" => {
            r.list(MAX_LIST, |r| {
                r.var_int()?;
                r.nbt_raw()?;
                r.var_int()?;
                r.var_int()
            })?;
        }
        // Prefixed array of (attribute id, modifier id, amount, operation,
        // slot group, display: 0 default, 1 hidden, 2 override + component).
        // Observed on the 26.3 vanilla server; matches Pumpkin.
        "attribute_modifiers" => {
            r.list(MAX_LIST, |r| {
                r.var_int()?;
                r.identifier()?;
                r.f64()?;
                r.var_int()?;
                r.var_int()?;
                if r.var_int()? == 2 {
                    r.component()?;
                }
                Ok(())
            })?;
        }
        // Holder<PaintingVariant>: registry id + 1, or 0 + inline variant.
        // Observed on the 26.3 vanilla server (`painting[painting/variant=
        // "minecraft:kebab"]` is the single byte 25).
        "painting/variant" => {
            IdOr::<crate::metadata::PaintingVariant>::read(r)?;
        }
        // Swing animation: VarInt type, VarInt duration (Pumpkin).
        "attack_animation" => {
            r.var_int()?;
            r.var_int()?;
        }
        _ => return Err(unsupported(name)),
    }
    Ok(())
}

/// `ID or X` where X is skipped by `f`.
fn skip_id_or(
    r: &mut Reader<'_>,
    f: impl FnOnce(&mut Reader<'_>) -> DecodeResult<()>,
) -> DecodeResult<()> {
    if r.var_int()? == 0 {
        f(r)?;
    }
    Ok(())
}

/// `Block Predicate`.
fn skip_block_predicate(r: &mut Reader<'_>) -> DecodeResult<()> {
    r.option(|r| IdSet::read(r))?;
    r.option(|r| {
        r.list(MAX_LIST, |r| {
            r.string()?;
            if r.bool()? {
                r.string()?;
            } else {
                r.string()?;
                r.string()?;
            }
            Ok(())
        })
    })?;
    r.option(|r| r.nbt_raw())?;
    // Exact data component matchers: type + structured component value.
    r.list(MAX_LIST, |r| {
        let ty = r.var_int()?;
        let name = component_name(r, ty)?;
        skip_component(r, name)
    })?;
    // Partial data component predicates: type + NBT compound.
    r.list(64, |r| {
        r.var_int()?;
        r.nbt_raw()
    })?;
    Ok(())
}

/// `Potion Effect` detail (recursive through the hidden effect).
fn skip_effect_detail(r: &mut Reader<'_>, depth: usize) -> DecodeResult<()> {
    if depth > 32 {
        return Err(DecodeError::TooLarge("nested hidden potion effects".into()));
    }
    r.var_int()?;
    r.var_int()?;
    r.bool()?;
    r.bool()?;
    r.bool()?;
    if r.bool()? {
        skip_effect_detail(r, depth + 1)?;
    }
    Ok(())
}

/// `Potion Effect`.
fn skip_potion_effect(r: &mut Reader<'_>) -> DecodeResult<()> {
    r.var_int()?;
    skip_effect_detail(r, 0)
}

/// `Consume Effect` (type ids are the wiki's: 0 apply_effects .. 4 play_sound).
fn skip_consume_effect(r: &mut Reader<'_>) -> DecodeResult<()> {
    match r.var_int()? {
        0 => {
            r.list(MAX_LIST, |r| skip_potion_effect(r))?;
            r.f32()?;
        }
        1 => {
            IdSet::read(r)?;
        }
        2 => {}
        3 => {
            r.f32()?;
        }
        4 => {
            SoundEvent::read(r)?;
        }
        t => return Err(DecodeError::Invalid(format!("consume effect type {t}"))),
    }
    Ok(())
}

/// `Firework Explosion`.
fn skip_firework_explosion(r: &mut Reader<'_>) -> DecodeResult<()> {
    r.var_int()?;
    r.list(MAX_LIST, |r| r.i32())?;
    r.list(MAX_LIST, |r| r.i32())?;
    r.bool()?;
    r.bool()?;
    Ok(())
}

/// `Trim Material` (inline).
fn skip_trim_material(r: &mut Reader<'_>) -> DecodeResult<()> {
    r.string()?;
    r.list(MAX_LIST, |r| {
        r.identifier()?;
        r.string()
    })?;
    r.nbt_raw()?;
    Ok(())
}

/// `Trim Pattern` (inline).
fn skip_trim_pattern(r: &mut Reader<'_>) -> DecodeResult<()> {
    r.string()?;
    r.var_int()?;
    r.nbt_raw()?;
    r.bool()?;
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
        let plain = ItemStackData {
            count: 3,
            item: Some(RegistryIndex(42)),
            components_raw: Bytes::new(),
        };
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
        let tables = crate::packets::ids::ProtocolTables::for_tests(&[
            (3, "minecraft:damage"),
            (60, "minecraft:mystery"),
        ]);
        // count 1, item 7, 1 add, 1 remove, damage=5, remove type 4
        let bytes = [1u8, 7, 1, 1, 3, 5, 4];
        let mut r = Reader::new(&bytes).with_tables(&tables);
        let s = r.read::<ItemStackData>().unwrap();
        assert_eq!(&s.components_raw[..], &[1, 1, 3, 5, 4]);
        r.finish().unwrap();
        assert!(Reader::new(&bytes).read::<ItemStackData>().is_err());
        let unknown = [1u8, 7, 1, 0, 60, 0];
        assert!(Reader::new(&unknown)
            .with_tables(&tables)
            .read::<ItemStackData>()
            .is_err());
    }

    /// Hand-built values for every documented component; each must be
    /// consumed exactly.
    fn samples() -> Vec<(&'static str, Vec<u8>)> {
        use haste_core::{BlockPos, Component, Identifier};
        let w = |f: &dyn Fn(&mut Writer)| {
            let mut w = Writer::new();
            f(&mut w);
            w.into_inner()
        };
        let nbt = |w: &mut Writer| {
            let mut c = crate::nbt::NbtCompound::new();
            c.put_int("x", 1);
            w.nbt_compound(Some(&c));
        };
        let text = |w: &mut Writer| w.component(&Component::text("t"));
        let sound = |w: &mut Writer| {
            w.write(&SoundEvent {
                name: Identifier::minecraft("s"),
                fixed_range: Some(1.0),
            })
        };
        let id_set = |w: &mut Writer| w.write(&IdSet::Ids(vec![1, 2]));
        // Components hold item stack templates: item 300 (two VarInt bytes),
        // then count 2, then an empty patch.
        let stack = |w: &mut Writer| {
            w.write_as::<_, ItemStackTemplate>(&ItemStackData {
                count: 2,
                item: Some(RegistryIndex(300)),
                components_raw: Bytes::new(),
            })
        };
        let effect = |w: &mut Writer| {
            w.var_int(1); // type
            w.var_int(0);
            w.var_int(200);
            w.bool(false);
            w.bool(true);
            w.bool(true);
            w.bool(true); // hidden effect present
            w.var_int(0);
            w.var_int(100);
            w.bool(false);
            w.bool(true);
            w.bool(true);
            w.bool(false);
        };
        let consume_effects = move |w: &mut Writer| {
            w.var_int(5);
            w.var_int(0);
            w.var_int(1);
            effect(w);
            w.f32(1.0);
            w.var_int(1);
            id_set(w);
            w.var_int(2);
            w.var_int(3);
            w.f32(16.0);
            w.var_int(4);
            sound(w);
        };
        let firework = |w: &mut Writer| {
            w.var_int(1);
            w.write(&vec![0xff0000i32]);
            w.write(&Vec::<i32>::new());
            w.bool(true);
            w.bool(false);
        };
        let predicate = move |w: &mut Writer| {
            w.bool(true);
            id_set(w);
            w.bool(true);
            w.var_int(2);
            w.string("facing");
            w.bool(true);
            w.string("north");
            w.string("age");
            w.bool(false);
            w.string("0");
            w.string("3");
            w.bool(true);
            nbt(w);
            w.var_int(1); // exact matcher: damage = 4
            w.var_int(3);
            w.var_int(4);
            w.var_int(1); // partial predicate
            w.var_int(0);
            nbt(w);
        };
        vec![
            ("custom_data", w(&nbt)),
            ("max_stack_size", w(&|w| w.var_int(16))),
            ("unbreakable", vec![]),
            (
                "use_effects",
                w(&|w| {
                    w.bool(true);
                    w.bool(false);
                    w.f32(0.5);
                }),
            ),
            ("custom_name", w(&text)),
            ("minimum_attack_charge", w(&|w| w.f32(0.5))),
            ("damage_type", w(&|w| w.var_int(3))),
            (
                "lore",
                w(&|w| {
                    w.var_int(2);
                    text(w);
                    text(w);
                }),
            ),
            (
                "enchantments",
                w(&|w| {
                    w.var_int(1);
                    w.var_int(5);
                    w.var_int(3);
                }),
            ),
            (
                "can_place_on",
                w(&|w| {
                    w.var_int(1);
                    predicate(w);
                }),
            ),
            (
                "custom_model_data",
                w(&|w| {
                    w.write(&vec![1.0f32]);
                    w.write(&vec![true]);
                    w.write(&vec!["a".to_string()]);
                    w.write(&vec![7i32]);
                }),
            ),
            (
                "tooltip_display",
                w(&|w| {
                    w.bool(false);
                    w.write_as::<_, crate::wire::List<crate::wire::VarInt>>(&vec![1i32, 2]);
                }),
            ),
            (
                "food",
                w(&|w| {
                    w.var_int(4);
                    w.f32(2.4);
                    w.bool(false);
                }),
            ),
            (
                "consumable",
                w(&|w| {
                    w.f32(1.6);
                    w.var_int(1);
                    w.write(&IdOr::<SoundEvent>::Id(10));
                    w.bool(true);
                    consume_effects(w);
                }),
            ),
            ("use_remainder", w(&stack)),
            (
                "use_cooldown",
                w(&|w| {
                    w.f32(1.0);
                    w.write(&Some(Identifier::minecraft("g")));
                }),
            ),
            ("damage_resistant", w(&id_set)),
            ("attack_range", w(&|w| (0..6).for_each(|_| w.f32(1.0)))),
            (
                "tool",
                w(&|w| {
                    w.var_int(1);
                    id_set(w);
                    w.write(&Some(8.0f32));
                    w.write(&Some(true));
                    w.f32(1.0);
                    w.var_int(1);
                    w.bool(true);
                }),
            ),
            (
                "weapon",
                w(&|w| {
                    w.var_int(1);
                    w.f32(5.0);
                }),
            ),
            (
                "equippable",
                w(&|w| {
                    w.var_int(4);
                    w.write(&IdOr::Inline(SoundEvent {
                        name: Identifier::minecraft("e"),
                        fixed_range: None,
                    }));
                    w.write(&Some(Identifier::minecraft("m")));
                    w.write(&None::<Identifier>);
                    w.write(&Some(IdSet::Tag(Identifier::minecraft("t"))));
                    w.bool(true);
                    w.bool(true);
                    w.bool(true);
                    w.bool(false);
                    w.bool(false);
                    w.write(&IdOr::<SoundEvent>::Id(1));
                }),
            ),
            ("repairable", w(&id_set)),
            ("death_protection", w(&consume_effects)),
            (
                "blocks_attacks",
                w(&|w| {
                    w.f32(0.25);
                    w.f32(1.0);
                    w.var_int(1);
                    w.f32(90.0);
                    w.bool(true);
                    id_set(w);
                    w.f32(0.0);
                    w.f32(1.0);
                    w.f32(3.0);
                    w.f32(1.0);
                    w.f32(1.0);
                    w.bool(false);
                    w.bool(true);
                    w.write(&IdOr::<SoundEvent>::Id(2));
                    w.bool(false);
                }),
            ),
            (
                "piercing_weapon",
                w(&|w| {
                    w.bool(true);
                    w.bool(false);
                    w.bool(true);
                    sound(w);
                    w.bool(false);
                }),
            ),
            (
                "kinetic_weapon",
                w(&|w| {
                    w.var_int(10);
                    w.var_int(5);
                    w.bool(true);
                    w.var_int(20);
                    w.f32(0.1);
                    w.f32(0.2);
                    w.bool(false);
                    w.bool(false);
                    w.f32(1.0);
                    w.f32(2.0);
                    w.bool(false);
                    w.bool(true);
                    sound(w);
                }),
            ),
            (
                "charged_projectiles",
                w(&|w| {
                    w.var_int(1);
                    stack(w);
                }),
            ),
            (
                "potion_contents",
                w(&|w| {
                    w.option(&Some(0i32), |w, p| w.var_int(*p));
                    w.write(&Some(0xff00ffi32));
                    w.var_int(1);
                    effect(w);
                    w.write(&Some("n".to_string()));
                }),
            ),
            ("potion_duration_scale", w(&|w| w.f32(0.5))),
            (
                "suspicious_stew_effects",
                w(&|w| {
                    w.var_int(1);
                    w.var_int(2);
                    w.var_int(160);
                }),
            ),
            (
                "writable_book_content",
                w(&|w| {
                    w.var_int(1);
                    w.string("p");
                    w.write(&None::<String>);
                }),
            ),
            (
                "written_book_content",
                w(&|w| {
                    w.string("title");
                    w.write(&None::<String>);
                    w.string("author");
                    w.var_int(0);
                    w.var_int(1);
                    text(w);
                    w.bool(true);
                    text(w);
                    w.bool(true);
                }),
            ),
            (
                "trim",
                w(&|w| {
                    w.var_int(0);
                    w.string("_gold");
                    w.var_int(1);
                    w.identifier(&Identifier::minecraft("iron"));
                    w.string("gold_darker");
                    text(w);
                    w.var_int(0);
                    w.string("coast");
                    w.var_int(5);
                    text(w);
                    w.bool(false);
                }),
            ),
            ("debug_stick_state", w(&nbt)),
            (
                "entity_data",
                w(&|w| {
                    w.var_int(5);
                    nbt(w);
                }),
            ),
            (
                "instrument",
                w(&|w| {
                    w.var_int(0);
                    w.write(&IdOr::<SoundEvent>::Id(1));
                    w.f32(7.0);
                    w.f32(256.0);
                    text(w);
                }),
            ),
            (
                "jukebox_playable",
                w(&|w| {
                    w.var_int(0);
                    w.write(&IdOr::<SoundEvent>::Id(1));
                    text(w);
                    w.f32(100.0);
                    w.var_int(5);
                }),
            ),
            (
                "lodestone_tracker",
                w(&|w| {
                    w.bool(true);
                    w.identifier(&Identifier::minecraft("overworld"));
                    w.block_pos(&BlockPos::new(1, 2, 3));
                    w.bool(true);
                }),
            ),
            ("firework_explosion", w(&firework)),
            (
                "fireworks",
                w(&|w| {
                    w.var_int(2);
                    w.var_int(1);
                    firework(w);
                }),
            ),
            (
                "profile",
                w(&|w| {
                    w.write(&ResolvableProfile {
                        profile: crate::wire::ResolvableProfileKind::Partial {
                            name: Some("Steve".into()),
                            uuid: None,
                            properties: vec![],
                        },
                        body: None,
                        cape: None,
                        elytra: None,
                        model: Some(1),
                    })
                }),
            ),
            (
                "banner_patterns",
                w(&|w| {
                    w.var_int(2);
                    w.var_int(3);
                    w.var_int(14);
                    w.var_int(0);
                    w.identifier(&Identifier::minecraft("stripe"));
                    w.string("block.minecraft.banner.stripe");
                    w.var_int(0);
                }),
            ),
            (
                "pot_decorations",
                // Observed bytes: back and right empty, left and front sherds.
                vec![0, 1, 0xC9, 0x0C, 1, 0, 0, 0, 1, 0xC5, 0x0C, 1, 0, 0],
            ),
            (
                "container",
                w(&|w| {
                    w.var_int(2);
                    w.bool(true);
                    stack(w);
                    w.bool(false);
                }),
            ),
            (
                "block_state",
                w(&|w| {
                    w.var_int(1);
                    w.string("lit");
                    w.string("true");
                }),
            ),
            (
                "bees",
                w(&|w| {
                    w.var_int(1);
                    w.var_int(9);
                    nbt(w);
                    w.var_int(10);
                    w.var_int(600);
                }),
            ),
            (
                "lock",
                w(&|w| w.nbt_tag(Some(&crate::nbt::NbtTag::String("key".into())))),
            ),
            ("break_sound", w(&|w| w.write(&IdOr::<SoundEvent>::Id(3)))),
            ("sulfur_cube_content", w(&stack)),
            ("dyed_color", w(&|w| w.i32(0x00ff00))),
            (
                "provides_banner_patterns",
                w(&|w| w.write(&IdSet::Tag(Identifier::minecraft("pattern_item/flower")))),
            ),
            ("provides_trim_material", w(&|w| w.var_int(4))),
            ("sheep/color", w(&|w| w.var_int(14))),
            // Observed on the 26.3 vanilla server: `iron_sword[attribute_modifiers=
            // [{type:"attack_damage",amount:5,id:"x:y",operation:"add_value"}]]`.
            (
                "attribute_modifiers",
                vec![
                    1, 3, 3, b'x', b':', b'y', 0x40, 0x14, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                ],
            ),
            ("painting/variant", vec![25]),
            (
                "attack_animation",
                w(&|w| {
                    w.var_int(1);
                    w.var_int(6);
                }),
            ),
        ]
    }

    /// Bytes observed from the 26.3 vanilla server for a shulker box holding
    /// five diamonds (`container` = list of optional templates).
    #[test]
    fn container_holds_optional_templates() {
        let t = crate::packets::ids::ProtocolTables::for_tests(&[(77, "minecraft:container")]);
        let wire = [1u8, 0x90, 0x05, 1, 0, 77, 1, 1, 0xF4, 0x07, 5, 0, 0];
        let mut r = Reader::new(&wire).with_tables(&t);
        let s = ItemStackData::read(&mut r).unwrap();
        r.finish().unwrap();
        assert_eq!(s.item, Some(RegistryIndex(656)));
        let mut r = Reader::new(&s.components_raw[3..]);
        let inner = r.list(256, OptionalItemStackTemplate::read).unwrap();
        assert_eq!(inner[0].item, Some(RegistryIndex(1012)));
        assert_eq!(inner[0].count, 5);
    }

    #[test]
    fn every_documented_component_skips_exactly() {
        let tables = crate::packets::ids::ProtocolTables::for_tests(&[(3, "minecraft:damage")]);
        for (name, bytes) in samples() {
            let mut with_tail = bytes.clone();
            with_tail.push(0xAB);
            let mut r = Reader::new(&with_tail).with_tables(&tables);
            skip_component(&mut r, &format!("minecraft:{name}"))
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(
                r.peek_rest(),
                &[0xAB],
                "{name} did not consume exactly its bytes"
            );
        }
    }

    #[test]
    #[ignore = "needs cache/datagen/26.3/reports (run with --ignored); prints unsupported components"]
    fn report_unsupported_components() {
        let t = crate::packets::ids::ProtocolTables::load().expect("reports");
        let mut missing = Vec::new();
        for id in 0..200 {
            let Some(name) = t.data_component_name(id) else {
                continue;
            };
            let mut r = Reader::new(&[]);
            if let Err(DecodeError::Invalid(m)) = skip_component(&mut r, name) {
                if m.contains("not implemented") {
                    missing.push(name.to_string());
                }
            }
        }
        println!("unsupported data components: {missing:?}");
    }
}
