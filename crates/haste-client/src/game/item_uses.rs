//! The per-item arm-pose inputs for the entity models (`haste-entity`'s
//! [`ItemUses`](haste_entity::animation::ItemUses)): which use animation an item has,
//! whether it is the crossbow and whether it is a spear. Vanilla reads these off the item
//! classes (`ItemStack.getUseAnimation`, `CrossbowItem.isCharged`'s item,
//! `ItemTags.SPEARS`); the client resolves the same answers once per item from the game
//! data (the `minecraft:consumable` report for the animations, the item registry for the
//! crossbow, the `minecraft:piercing_weapon` report for the spears) and hands the table
//! to the entity store.

use haste_data::GameData;

use haste_entity::animation::{ItemUse, UseAnimation};

/// Builds the table over every `minecraft:item` entry.
pub(crate) fn build(data: &GameData) -> haste_entity::animation::ItemUses {
    let (registries, components) = (&data.registries, &data.item_components);
    let n = registries.len("minecraft:item");
    let items = (0..n as u32)
        .map(|i| {
            let name = registries.name("minecraft:item", haste_core::RegistryIndex(i));
            item_use(name, components)
        })
        .collect();
    // `CrossbowItem.isCharged` reads the stack's `minecraft:charged_projectiles` component.
    let charged_projectiles_type = registries
        .index(
            "minecraft:data_component_type",
            &haste_core::Identifier::minecraft("charged_projectiles"),
        )
        .map(|i| i.0 as i32);
    haste_entity::animation::ItemUses::new(items, charged_projectiles_type)
}

/// One item's pose inputs (vanilla's answers per item class):
/// `ItemStack.getUseAnimation` — the `minecraft:consumable` component's `animation` when
/// present (food and potions), else the classes that override it (`BowItem`,
/// `CrossbowItem`, `TridentItem`, `SpyglassItem`, `ShieldItem`; `GoatHornItem` and
/// `BrushItem`, which vanilla's arm-pose switch also names) — the crossbow's own flag,
/// and `ItemTags.SPEARS` (the items whose default components carry
/// `minecraft:piercing_weapon`).
fn item_use(
    name: Option<&haste_core::Identifier>,
    components: &haste_data::ItemComponents,
) -> ItemUse {
    let Some(name) = name else {
        return ItemUse::default();
    };
    let mut use_ = ItemUse {
        // `Consumable.animation` (eat/drink for food and potions).
        animation: components
            .consumable(name)
            .map_or(UseAnimation::None, |c| use_animation_from_id(c.animation)),
        crossbow: name == &haste_core::Identifier::minecraft("crossbow"),
        spear: components.piercing_weapon_sound(name).is_some(),
    };
    if use_.animation == UseAnimation::None {
        use_.animation = match name.path() {
            // `Item.getUseAnimation`'s class overrides.
            "bow" => UseAnimation::Bow,
            "crossbow" => UseAnimation::Crossbow,
            "trident" => UseAnimation::Trident,
            "spyglass" => UseAnimation::Spyglass,
            "goat_horn" => UseAnimation::TootHorn,
            "brush" => UseAnimation::Brush,
            "shield" => UseAnimation::Block,
            _ => UseAnimation::None,
        };
    }
    use_
}

/// The consumable component's `animation` id (`ItemUseAnimation`'s ordinal; `eat` and any
/// unknown id are vanilla's default).
fn use_animation_from_id(id: i32) -> UseAnimation {
    match id {
        0 => UseAnimation::None,
        2 => UseAnimation::Drink,
        3 => UseAnimation::Block,
        4 => UseAnimation::Bow,
        5 => UseAnimation::Trident,
        6 => UseAnimation::Crossbow,
        7 => UseAnimation::Spyglass,
        8 => UseAnimation::TootHorn,
        9 => UseAnimation::Brush,
        11 => UseAnimation::Spear,
        _ => UseAnimation::Eat,
    }
}
