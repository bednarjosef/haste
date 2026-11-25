//! Skipping data component patches: the component layouts by registry name (the wire
//! format of vanilla `DataComponentPatch` / `DataComponentType.streamCodec`).

use super::super::{
    Codec, DecodeError, DecodeResult, IdOr, IdSet, Reader, ResolvableProfile, SoundEvent, Wire,
    MAX_LIST,
};
use super::{ItemStackTemplate, OptionalItemStackTemplate};

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

/// Resolves a data component type id to its `minecraft:data_component_type`
/// name (used when reading and when skipping a patch).
pub(crate) fn component_name<'a>(r: &Reader<'a>, ty: i32) -> DecodeResult<&'a str> {
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
pub(crate) fn skip_consume_effect(r: &mut Reader<'_>) -> DecodeResult<()> {
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
