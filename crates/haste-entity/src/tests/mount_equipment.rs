//! `mount_equipment`: the layer table per entity type (`EntityRenderers` registrations), the
//! rules of `SimpleEquipmentLayer`, `WolfArmorLayer` and `LlamaDecorLayer`, the saddles' reins
//! and `Crackiness.WOLF_ARMOR`'s thresholds (`0.95, 0.69, 0.32`).
use haste_core::entity::ItemStackData;
use haste_core::model::{EquipmentSlot, PoseOverrides};
use haste_core::{Identifier, RegistryIndex};
use haste_data::{ItemComponents, ItemEquippable};

use crate::mount_equipment::{
    cracked_fraction, crackiness_texture, layers_for, resolve, saddle_setup_anim,
    wolf_armor_cracks, LayerKind,
};

#[test]
fn crackiness_thresholds_match_vanilla() {
    // From 0.95 up no crack; below 0.95 low, below 0.69 medium, below 0.32 high.
    assert_eq!(crackiness_texture(1.0), None);
    assert_eq!(crackiness_texture(0.95), None);
    let low = Some("entity/wolf/wolf_armor_crackiness_low");
    let medium = Some("entity/wolf/wolf_armor_crackiness_medium");
    let high = Some("entity/wolf/wolf_armor_crackiness_high");
    assert_eq!(crackiness_texture(0.94), low);
    assert_eq!(crackiness_texture(0.69), low);
    assert_eq!(crackiness_texture(0.68), medium);
    assert_eq!(crackiness_texture(0.32), medium);
    assert_eq!(crackiness_texture(0.31), high);
    assert_eq!(crackiness_texture(0.0), high);
}

#[test]
fn remaining_fraction_is_max_minus_damage_over_max() {
    assert_eq!(cracked_fraction(64, 0), Some(1.0));
    assert_eq!(cracked_fraction(64, 32), Some(0.5));
    assert_eq!(cracked_fraction(64, 64), Some(0.0));
    assert_eq!(cracked_fraction(0, 0), None);
}

#[test]
fn wolf_armor_cracks_follow_the_wear() {
    // Wolf armour has 64 durability: 3 damage keeps 0.953 (no crack), 4 drops to 0.9375 (low),
    // 20 to 0.6875 (medium), 44 to 0.3125 (high).
    let crack = |d| wolf_armor_cracks(Some((d, 64)));
    assert_eq!(crack(0), None);
    assert_eq!(crack(3), None);
    assert_eq!(
        crack(4),
        Some(Identifier::minecraft(
            "entity/wolf/wolf_armor_crackiness_low"
        ))
    );
    assert_eq!(
        crack(20),
        Some(Identifier::minecraft(
            "entity/wolf/wolf_armor_crackiness_medium"
        ))
    );
    assert_eq!(
        crack(44),
        Some(Identifier::minecraft(
            "entity/wolf/wolf_armor_crackiness_high"
        ))
    );
    // A stack that is not damageable (no max damage, or unbreakable) never cracks.
    assert_eq!(wolf_armor_cracks(None), None);
}

#[test]
fn layer_table_per_type() {
    let names = |t| {
        layers_for(t)
            .iter()
            .map(|r| (r.slot, r.adult, r.layer_type))
            .collect::<Vec<_>>()
    };
    use EquipmentSlot::{Body, Saddle};
    // `HorseRenderer`: armour, then saddle.
    assert_eq!(
        names("minecraft:horse"),
        vec![
            (Body, "minecraft:horse_armor#main", "horse_body"),
            (Saddle, "minecraft:horse#saddle", "horse_saddle"),
        ]
    );
    // `UndeadHorseRenderer`: the undead armour model, and the type's own saddle layer.
    assert_eq!(
        names("minecraft:zombie_horse"),
        vec![
            (Body, "minecraft:undead_horse_armor#main", "horse_body"),
            (
                Saddle,
                "minecraft:zombie_horse#saddle",
                "zombie_horse_saddle"
            ),
        ]
    );
    assert_eq!(
        names("minecraft:skeleton_horse")[1],
        (
            Saddle,
            "minecraft:skeleton_horse#saddle",
            "skeleton_horse_saddle"
        )
    );
    // Saddles only.
    for (t, layer, ty) in [
        (
            "minecraft:donkey",
            "minecraft:donkey#saddle",
            "donkey_saddle",
        ),
        ("minecraft:mule", "minecraft:mule#saddle", "mule_saddle"),
        ("minecraft:camel", "minecraft:camel#saddle", "camel_saddle"),
        (
            "minecraft:camel_husk",
            "minecraft:camel_husk#saddle",
            "camel_husk_saddle",
        ),
        ("minecraft:pig", "minecraft:pig#saddle", "pig_saddle"),
        (
            "minecraft:strider",
            "minecraft:strider#saddle",
            "strider_saddle",
        ),
    ] {
        assert_eq!(names(t), vec![(Saddle, layer, ty)], "{t}");
    }
    assert_eq!(
        names("minecraft:wolf"),
        vec![(Body, "minecraft:wolf_armor#main", "wolf_body")]
    );
    assert_eq!(layers_for("minecraft:wolf")[0].kind, LayerKind::WolfArmor);
    assert_eq!(
        names("minecraft:trader_llama"),
        vec![(Body, "minecraft:llama#decor", "llama_body")]
    );
    // Types without equipment layers (and unknown, modded types) wear nothing here.
    assert!(layers_for("minecraft:sheep").is_empty());
    assert!(layers_for("mymod:yak").is_empty());
}

/// Item 1 is a saddle (asset `minecraft:saddle`), item 2 has no `equippable`, item 3 is a
/// carpet (asset `minecraft:white_carpet`).
fn components() -> ItemComponents {
    let mut c = ItemComponents::default();
    let eq = |slot, asset: &str| ItemEquippable {
        slot,
        asset: Some(Identifier::minecraft(asset)),
    };
    c.insert_equippable(RegistryIndex(1), eq(EquipmentSlot::Saddle, "saddle"));
    c.insert_equippable(RegistryIndex(3), eq(EquipmentSlot::Body, "white_carpet"));
    c
}

fn stack(item: u32) -> ItemStackData {
    ItemStackData {
        count: 1,
        item: Some(RegistryIndex(item)),
        components_raw: Default::default(),
    }
}

#[test]
fn simple_layer_draws_an_equippable_with_an_asset_on_adults() {
    let c = components();
    let pig = &layers_for("minecraft:pig")[0];
    let saddle = stack(1);
    let (model, asset, from) = resolve(pig, false, Some(&saddle), &c).unwrap();
    assert_eq!(model, "minecraft:pig#saddle");
    assert_eq!(asset, Identifier::minecraft("saddle"));
    assert!(from.is_some());
    // No baby model: a saddled piglet draws nothing; neither does an empty slot or an item
    // without an equipment asset.
    assert!(resolve(pig, true, Some(&saddle), &c).is_none());
    assert!(resolve(pig, false, None, &c).is_none());
    assert!(resolve(pig, false, Some(&stack(2)), &c).is_none());
}

#[test]
fn llama_decor_and_the_trader_llama_fallback() {
    let c = components();
    let llama = &layers_for("minecraft:llama")[0];
    let trader = &layers_for("minecraft:trader_llama")[0];
    let carpet = stack(3);
    // A carpet on an adult.
    let (model, asset, _) = resolve(llama, false, Some(&carpet), &c).unwrap();
    assert_eq!(
        (model, asset),
        (
            "minecraft:llama#decor",
            Identifier::minecraft("white_carpet")
        )
    );
    // A baby llama's carpet is not drawn; a bare llama draws nothing.
    assert!(resolve(llama, true, Some(&carpet), &c).is_none());
    assert!(resolve(llama, false, None, &c).is_none());
    // A bare trader llama wears its own decor, the baby one on babies, with no stack.
    let (model, asset, from) = resolve(trader, false, None, &c).unwrap();
    assert_eq!(
        (model, asset),
        (
            "minecraft:llama#decor",
            Identifier::minecraft("trader_llama")
        )
    );
    assert!(from.is_none());
    let (model, asset, _) = resolve(trader, true, Some(&carpet), &c).unwrap();
    assert_eq!(
        (model, asset),
        (
            "minecraft:llama_baby#decor",
            Identifier::minecraft("trader_llama_baby")
        )
    );
}

#[test]
fn wolf_armor_is_adult_only() {
    let c = components();
    let wolf = &layers_for("minecraft:wolf")[0];
    let armor = stack(3);
    assert!(resolve(wolf, false, Some(&armor), &c).is_some());
    assert!(resolve(wolf, true, Some(&armor), &c).is_none());
}

#[test]
fn saddle_reins_show_only_while_ridden() {
    let hidden = |t: &str, ridden| {
        let mut pose = PoseOverrides::default();
        saddle_setup_anim(Some(t), ridden, &mut pose);
        pose.hidden().count()
    };
    // `EquineSaddleModel`: both saddle lines; `CamelSaddleModel`: the reins.
    assert_eq!(hidden("minecraft:horse", false), 2);
    assert_eq!(hidden("minecraft:mule", false), 2);
    assert_eq!(hidden("minecraft:camel", false), 1);
    assert_eq!(hidden("minecraft:horse", true), 0);
    assert_eq!(hidden("minecraft:camel", true), 0);
    // The pig's and strider's saddles have no reins.
    assert_eq!(hidden("minecraft:pig", false), 0);
    let mut pose = PoseOverrides::default();
    saddle_setup_anim(Some("minecraft:horse"), false, &mut pose);
    assert!(pose.is_hidden(haste_core::parts::intern("left_saddle_line")));
    assert!(pose.is_hidden(haste_core::parts::intern("right_saddle_line")));
}

/// Every layer model the table names is in the recorder's dump, so no row silently draws
/// nothing for want of geometry.
#[test]
#[ignore = "needs cache/oracle/26.3/entity_models.json (recorder mod dump; HASTE_CACHE overrides)"]
fn every_layer_row_names_a_dumped_layer() {
    let Some(lib) = crate::model_dump::library() else {
        panic!("entity_models.json not found");
    };
    for t in [
        "minecraft:horse",
        "minecraft:skeleton_horse",
        "minecraft:zombie_horse",
        "minecraft:donkey",
        "minecraft:mule",
        "minecraft:camel",
        "minecraft:camel_husk",
        "minecraft:pig",
        "minecraft:strider",
        "minecraft:nautilus",
        "minecraft:wolf",
        "minecraft:llama",
        "minecraft:trader_llama",
    ] {
        for row in layers_for(t) {
            for layer in std::iter::once(row.adult).chain(row.baby) {
                assert!(lib.get(layer, 1.0).is_some(), "{t}: {layer} not dumped");
            }
        }
    }
}
