use super::tool::ToolBlocks;
use super::*;
use crate::registries::Registries;
use haste_core::model::EquipmentSlot;
use haste_core::{Identifier, RegistryIndex};

fn attr(name: &str) -> Identifier {
    Identifier::minecraft(name)
}

fn modifier(slot: &str, amount: f64) -> AttributeModifier {
    AttributeModifier {
        attribute: attr("attack_speed"),
        amount,
        operation: 0,
        slot: slot.to_owned(),
    }
}

#[test]
fn parses_the_equippable_component() {
    let json =
        br#"{"components":{"minecraft:equippable":{"slot":"chest","asset_id":"minecraft:iron"}}}"#;
    let eq = parse_equippable(json).unwrap();
    assert_eq!(eq.slot, EquipmentSlot::Chest);
    assert_eq!(eq.asset, Some(Identifier::minecraft("iron")));
}

#[test]
fn missing_equippable_is_none() {
    assert_eq!(parse_equippable(br#"{"components":{}}"#), None);
    assert_eq!(parse_equippable(b"not json"), None);
}

#[test]
fn main_hand_attribute_sums_only_main_hand_modifiers() {
    let mut c = ItemComponents::default();
    c.insert(
        attr("diamond_sword"),
        vec![
            AttributeModifier {
                attribute: attr("attack_damage"),
                amount: 7.0,
                operation: 0,
                slot: "mainhand".into(),
            },
            modifier("mainhand", -2.4),
            // Off-hand and armor modifiers are not the held item's.
            modifier("offhand", 100.0),
            modifier("armor", 100.0),
        ],
    );
    // Player base attack speed 4.0 plus the sword's -2.4 modifier: 1.6.
    assert!(
        (c.main_hand_attribute(&attr("diamond_sword"), &attr("attack_speed"), 4.0) - 1.6).abs()
            < 1e-9
    );
    // An item without the attribute keeps the base.
    assert_eq!(
        c.main_hand_attribute(&attr("stick"), &attr("attack_speed"), 4.0),
        4.0
    );
}

#[test]
fn parses_the_codec_shape() {
    let v = serde_json::json!([
        {"type": "minecraft:attack_speed", "id": "minecraft:base_attack_speed",
         "amount": -2.8, "operation": "add_value", "slot": "mainhand"}
    ]);
    let m = parse_modifiers(&v);
    assert_eq!(m, vec![modifier("mainhand", -2.8)]);
    // A wrapped object and a missing slot (defaults to `any`) both parse.
    let wrapped = serde_json::json!({"modifiers": [
        {"type": "minecraft:attack_speed", "amount": 0.5}
    ]});
    assert_eq!(
        parse_modifiers(&wrapped),
        vec![AttributeModifier {
            attribute: attr("attack_speed"),
            amount: 0.5,
            operation: 0,
            slot: "any".into(),
        }]
    );
}

#[test]
fn parses_consumables_with_defaults() {
    // `Consumable.CODEC` defaults: 1.6 s, `eat`, the generic eat sound, particles on
    // (bread's whole component in the report is `{}`).
    assert_eq!(
        parse_consumable(&serde_json::json!({})),
        Consumable::default_food()
    );
    // A drink with its own time: `consumeTicks` = seconds * 20.
    let drink = parse_consumable(&serde_json::json!({
        "consume_seconds": 1.6, "animation": "drink",
        "sound": "minecraft:entity.generic.drink", "has_consume_particles": false
    }));
    assert_eq!(drink.animation, 2);
    assert_eq!(drink.consume_ticks(), 32);
    assert_eq!(drink.sound, Identifier::minecraft("entity.generic.drink"));
    assert!(!drink.has_consume_particles);
    // An inline sound object names `sound_id` (`SoundEvent.DIRECT_CODEC`'s shape).
    let inline = parse_consumable(&serde_json::json!({
        "sound": {"sound_id": "minecraft:item.honey_bottle.drink", "range": 16.0}
    }));
    assert_eq!(
        inline.sound,
        Identifier::minecraft("item.honey_bottle.drink")
    );
}

/// The report's `minecraft:consumable`: bread is every default, the potion the default
/// drink (no particles, `entity.generic.drink`), the honey bottle overrides time and sound.
#[test]
#[ignore = "needs the item component reports in the cache"]
fn report_consumables_carry_sound_and_particles() {
    let c = super::ItemComponents::load_default().unwrap();
    let consumable = |name: &str| {
        c.consumable(&attr(name))
            .unwrap_or_else(|| panic!("{name} has a consumable"))
    };
    assert_eq!(consumable("bread"), Consumable::default_food());
    assert_eq!(consumable("potion"), Consumable::default_drink());
    let honey = consumable("honey_bottle");
    assert_eq!(honey.consume_seconds, 2.0);
    assert_eq!(honey.animation, 2);
    assert_eq!(
        honey.sound,
        Identifier::minecraft("item.honey_bottle.drink")
    );
    assert!(!honey.has_consume_particles);
    assert!(c.consumable_items().any(|i| i == &attr("bread")));
    assert!(!c.consumable_items().any(|i| i == &attr("diamond_sword")));
}

#[test]
fn parses_the_tool_component() {
    let t = ToolComponent::from_json(&serde_json::json!({
        "rules": [
            {"blocks": "minecraft:cobweb", "correct_for_drops": true, "speed": 15.0},
            {"blocks": ["minecraft:stone", "minecraft:dirt"], "speed": 1.5},
            {"blocks": "#minecraft:mineable/pickaxe", "speed": 8.0}
        ],
        "default_mining_speed": 1.0,
        "damage_per_block": 1,
        "can_destroy_blocks_in_creative": false
    }))
    .unwrap();
    assert_eq!(t.rules.len(), 3);
    assert_eq!(
        t.rules[0].blocks,
        ToolBlocks::Block(Identifier::minecraft("cobweb"))
    );
    assert_eq!(t.rules[0].speed, Some(15.0));
    assert_eq!(
        t.rules[2].blocks,
        ToolBlocks::Tag(Identifier::minecraft("mineable/pickaxe"))
    );
    assert!(!t.can_destroy_blocks_in_creative);
    // Defaults: speed 1, damage 1, creative break allowed; malformed is skipped.
    let d = ToolComponent::from_json(&serde_json::json!({"rules": []})).unwrap();
    assert_eq!((d.default_mining_speed, d.damage_per_block), (1.0, 1));
    assert!(d.can_destroy_blocks_in_creative);
    assert_eq!(
        ToolComponent::from_json(&serde_json::json!({"rules": [{"blocks": 5}]})),
        None
    );
}

#[test]
fn loads_by_registry_name_and_ignores_unknown_items() {
    let dir = std::env::temp_dir().join(format!(
        "haste-components-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let item_dir = dir.join("item");
    std::fs::create_dir_all(&item_dir).unwrap();
    std::fs::write(
        item_dir.join("iron_chestplate.json"),
        br#"{"components":{"minecraft:equippable":{"slot":"chest","asset_id":"minecraft:iron"}}}"#,
    )
    .unwrap();
    std::fs::write(item_dir.join("unknown_item.json"), br#"{"components":{}}"#).unwrap();
    let registries_json = r#"{
  "minecraft:item": {
    "entries": {
      "minecraft:air": { "protocol_id": 0 },
      "minecraft:iron_chestplate": { "protocol_id": 1 }
    }
  },
  "minecraft:data_component_type": {
    "entries": {
      "minecraft:damage": { "protocol_id": 0 },
      "minecraft:dyed_color": { "protocol_id": 1 }
    }
  }
}"#;
    let regs = Registries::parse(registries_json).unwrap();
    let components = ItemComponents::load(&item_dir, &regs);
    let eq = components.equippable(RegistryIndex(1)).unwrap();
    assert_eq!(eq.asset, Some(Identifier::minecraft("iron")));
    assert!(components.equippable(RegistryIndex(0)).is_none());
    // `dyed_color` as the patch's first add (type id 1, a big-endian int) decodes.
    let patch = [1u8, 0, 1, 0x00, 0x11, 0x22, 0x33];
    assert_eq!(components.dyed_color(&patch), Some(0x0011_2233));
    // A damage add (type 0, a varint) is skipped on the way to the dye; a damage alone has no
    // dye.
    assert_eq!(
        components.dyed_color(&[2, 0, 0, 5, 1, 0x00, 0x11, 0x22, 0x33]),
        Some(0x0011_2233)
    );
    assert_eq!(components.dyed_color(&[1, 0, 0, 5]), None);
    // An add Haste cannot skip (type 9) ends the scan: the dye behind it is not seen.
    assert_eq!(
        components.dyed_color(&[2, 0, 9, 1, 1, 0x00, 0x11, 0x22, 0x33]),
        None
    );
    assert_eq!(components.dyed_color(&[0, 0]), None);
    // A missing directory is empty, not an error.
    let empty = ItemComponents::load(&dir.join("nope"), &regs);
    assert!(empty.equippable(RegistryIndex(1)).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// `minecraft:max_damage` loads per item index, and a stack's wear (`Crackiness.byDamage`'s
/// damage and max damage) reads the patch's `damage`, `max_damage` and `unbreakable` adds, the
/// way `ItemStack.isDamageableItem` / `getDamageValue` / `getMaxDamage` see them.
#[test]
fn loads_max_damage_and_reads_the_wear() {
    let dir = std::env::temp_dir().join(format!("haste-max-damage-{}", std::process::id()));
    let item_dir = dir.join("item");
    std::fs::create_dir_all(&item_dir).unwrap();
    std::fs::write(
        item_dir.join("wolf_armor.json"),
        br#"{"components":{"minecraft:max_damage":64,"minecraft:equippable":{"slot":"body","asset_id":"minecraft:armadillo_scute"}}}"#,
    )
    .unwrap();
    std::fs::write(item_dir.join("stone.json"), br#"{"components":{}}"#).unwrap();
    let registries_json = r#"{
  "minecraft:item": {
    "entries": {
      "minecraft:air": { "protocol_id": 0 },
      "minecraft:stone": { "protocol_id": 1 },
      "minecraft:wolf_armor": { "protocol_id": 2 }
    }
  },
  "minecraft:data_component_type": {
    "entries": {
      "minecraft:damage": { "protocol_id": 0 },
      "minecraft:dyed_color": { "protocol_id": 1 },
      "minecraft:max_damage": { "protocol_id": 2 },
      "minecraft:unbreakable": { "protocol_id": 3 }
    }
  }
}"#;
    let regs = Registries::parse(registries_json).unwrap();
    let components = ItemComponents::load(&item_dir, &regs);
    let armor = RegistryIndex(2);
    assert_eq!(components.max_damage(armor), Some(64));
    assert_eq!(components.max_damage(RegistryIndex(1)), None);
    // No patch: undamaged, the default max.
    assert_eq!(components.wear(armor, &[0, 0]), Some((0, 64)));
    // A damage of 40 (type id 0, varint 40); a two-byte varint 200 (0xC8 0x01).
    assert_eq!(components.wear(armor, &[1, 0, 0, 40]), Some((40, 64)));
    assert_eq!(
        components.wear(armor, &[1, 0, 0, 0xC8, 0x01]),
        Some((200, 64))
    );
    // Dyed first, then damaged: the dye is skipped to reach the damage.
    assert_eq!(
        components.wear(armor, &[2, 0, 1, 0x00, 0x11, 0x22, 0x33, 0, 12]),
        Some((12, 64))
    );
    // A patched max damage wins over the default.
    assert_eq!(
        components.wear(armor, &[2, 0, 2, 100, 0, 30]),
        Some((30, 100))
    );
    // Unbreakable stacks and items without a max damage are not damageable.
    assert_eq!(components.wear(armor, &[2, 0, 0, 30, 3]), None);
    assert_eq!(components.wear(RegistryIndex(1), &[1, 0, 0, 30]), None);
    // A missing directory loads as an empty table: no default max, so nothing is damageable
    // unless the patch sets one.
    let empty = ItemComponents::load(&dir.join("nope"), &regs);
    assert_eq!(empty.max_damage(armor), None);
    assert_eq!(empty.wear(armor, &[1, 0, 0, 7]), None);
    assert_eq!(empty.wear(armor, &[2, 0, 2, 10, 0, 7]), Some((7, 10)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// One report file per component: the load reads them all in one pass.
#[test]
fn loads_equippable_attributes_consumables_tools_and_gliders() {
    let dir = std::env::temp_dir().join(format!(
        "haste-components-all-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let item_dir = dir.join("item");
    std::fs::create_dir_all(&item_dir).unwrap();
    std::fs::write(
        item_dir.join("iron_chestplate.json"),
        br#"{"components":{"minecraft:equippable":{"slot":"chest","asset_id":"minecraft:iron"}}}"#,
    )
    .unwrap();
    std::fs::write(
        item_dir.join("diamond_sword.json"),
        br#"{"components":{
            "minecraft:attribute_modifiers": [
                {"type": "minecraft:attack_speed", "amount": -2.4, "operation": "add_value", "slot": "mainhand"}
            ],
            "minecraft:tool": {"rules": [
                {"blocks": "minecraft:cobweb", "speed": 15.0, "correct_for_drops": true}
            ], "can_destroy_blocks_in_creative": false}
        }}"#,
    )
    .unwrap();
    std::fs::write(
        item_dir.join("apple.json"),
        br#"{"components":{"minecraft:consumable": {"consume_seconds": 1.6, "animation": "eat"}}}"#,
    )
    .unwrap();
    std::fs::write(
        item_dir.join("elytra.json"),
        br#"{"components":{"minecraft:glider": {}}}"#,
    )
    .unwrap();
    // A malformed tool entry is skipped, not fatal.
    std::fs::write(
        item_dir.join("broken_pickaxe.json"),
        br#"{"components":{"minecraft:tool": {"rules": [{"blocks": 5}]}}}"#,
    )
    .unwrap();
    let registries_json = r#"{
  "minecraft:item": {
    "entries": {
      "minecraft:air": { "protocol_id": 0 },
      "minecraft:iron_chestplate": { "protocol_id": 1 },
      "minecraft:diamond_sword": { "protocol_id": 2 },
      "minecraft:apple": { "protocol_id": 3 },
      "minecraft:elytra": { "protocol_id": 4 },
      "minecraft:broken_pickaxe": { "protocol_id": 5 }
    }
  },
  "minecraft:data_component_type": {
    "entries": {
      "minecraft:dyed_color": { "protocol_id": 0 }
    }
  }
}"#;
    let regs = Registries::parse(registries_json).unwrap();
    let c = ItemComponents::load(&item_dir, &regs);
    // equippable by registry index.
    assert!(c.equippable(RegistryIndex(1)).is_some());
    assert!(c.equippable(RegistryIndex(0)).is_none());
    // attribute modifiers and consumable by name.
    let sword = attr("diamond_sword");
    assert_eq!(
        c.main_hand_attribute(&sword, &attr("attack_speed"), 4.0),
        1.6
    );
    assert_eq!(c.consumable(&attr("apple")).unwrap().consume_ticks(), 32);
    // the tool component parses (and the malformed one was skipped).
    let tool = c.tool(&sword).unwrap();
    assert_eq!(tool.rules.len(), 1);
    assert!(!tool.can_destroy_blocks_in_creative);
    assert!(c.tool(&attr("broken_pickaxe")).is_none());
    assert_eq!(c.tools().len(), 1);
    // glider.
    assert!(c.is_glider(&attr("elytra")));
    assert!(!c.is_glider(&attr("stick")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn operation_order_matches_vanilla() {
    let mut c = ItemComponents::default();
    c.insert(
        attr("test_item"),
        vec![
            AttributeModifier {
                attribute: attr("attack_speed"),
                amount: 0.5,
                operation: 2, // add_multiplied_total (applied last)
                slot: "mainhand".into(),
            },
            AttributeModifier {
                attribute: attr("attack_speed"),
                amount: 1.0,
                operation: 0, // add_value (applied first)
                slot: "mainhand".into(),
            },
            AttributeModifier {
                attribute: attr("attack_speed"),
                amount: 0.25,
                operation: 1, // add_multiplied_base
                slot: "mainhand".into(),
            },
        ],
    );
    // base 4 -> +1 = 5 -> +0.25*4 = 6 -> +0.5*6 = 9.
    assert_eq!(
        c.main_hand_attribute(&attr("test_item"), &attr("attack_speed"), 4.0),
        9.0
    );
}

#[test]
#[ignore = "needs cache/datagen/26.3/reports (data generator)"]
fn loads_the_data_generator_attribute_modifiers() {
    let c = ItemComponents::load_default().unwrap();
    let speed = attr("attack_speed");
    // Diamond pickaxe: base 4.0 - 2.8 = 1.2 (the report stores the float32 value).
    let pick = c.main_hand_attribute(&attr("diamond_pickaxe"), &speed, 4.0);
    assert!((pick - 1.2).abs() < 1e-6, "diamond pickaxe {pick}");
    // Diamond sword: base 4.0 - 2.4 = 1.6.
    let sword = c.main_hand_attribute(&attr("diamond_sword"), &speed, 4.0);
    assert!((sword - 1.6).abs() < 1e-6, "diamond sword {sword}");
    assert!(c.len() > 30, "{} items", c.len());
    // Food and drink carry a `minecraft:consumable`.
    let apple = c.consumable(&attr("apple")).expect("apple consumable");
    assert_eq!((apple.animation, apple.consume_ticks()), (1, 32));
    let potion = c.consumable(&attr("potion")).expect("potion consumable");
    assert_eq!(potion.animation, 2);
    // Every item carries `minecraft:use_effects` (`UseEffects.DEFAULT`), the spear family
    // overrides it (`Item.Properties.spear`).
    assert_eq!(c.use_effects(&attr("bow")), UseEffects::DEFAULT);
    assert_eq!(
        c.use_effects(&attr("wooden_spear")),
        UseEffects {
            can_sprint: true,
            interact_vibrations: false,
            speed_multiplier: 1.0
        }
    );
}

#[test]
fn parses_the_use_effects_component() {
    let parse = super::use_effects::parse_use_effects;
    // `{}`: every field at its default (`UseEffects.DEFAULT`).
    assert_eq!(parse(&serde_json::json!({})), UseEffects::DEFAULT);
    assert_eq!(
        parse(&serde_json::json!({
            "can_sprint": true, "interact_vibrations": false, "speed_multiplier": 1.0
        })),
        UseEffects {
            can_sprint: true,
            interact_vibrations: false,
            speed_multiplier: 1.0
        }
    );
    // A missing item (or one without the component) falls back to the default, like
    // vanilla's `getOrDefault(USE_EFFECTS, UseEffects.DEFAULT)`.
    let c = ItemComponents::default();
    assert_eq!(c.use_effects(&attr("wooden_spear")), UseEffects::DEFAULT);
}

#[test]
fn parses_the_use_cooldown_group() {
    // `UseCooldown.CODEC`'s `cooldown_group`: only a named group lands in the table; a
    // `use_cooldown` without one (or none at all) leaves the item grouping under its own id.
    let mut c = ItemComponents::default();
    assert_eq!(c.cooldown_group(&attr("ender_pearl")), None);
    c.insert_use_cooldown(attr("ender_pearl"), attr("ender_pearl"));
    assert_eq!(
        c.cooldown_group(&attr("ender_pearl")),
        Some(&attr("ender_pearl"))
    );
    // The loader reads the group out of a report file.
    let dir = std::env::temp_dir().join(format!(
        "haste-cooldown-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let item_dir = dir.join("item");
    std::fs::create_dir_all(&item_dir).unwrap();
    std::fs::write(
        item_dir.join("shield.json"),
        br#"{"components":{"minecraft:use_cooldown":{"seconds":0.5,"cooldown_group":"minecraft:shield"}}}"#,
    )
    .unwrap();
    std::fs::write(
        item_dir.join("ender_pearl.json"),
        br#"{"components":{"minecraft:use_cooldown":{"seconds":1.0}}}"#,
    )
    .unwrap();
    let regs = Registries::parse(
        r#"{"minecraft:item": {"entries": {
            "minecraft:shield": {"protocol_id": 0},
            "minecraft:ender_pearl": {"protocol_id": 1}
        }},"minecraft:data_component_type": {"entries": {}}}"#,
    )
    .unwrap();
    let c = ItemComponents::load(&item_dir, &regs);
    assert_eq!(c.cooldown_group(&attr("shield")), Some(&attr("shield")));
    assert_eq!(c.cooldown_group(&attr("ender_pearl")), None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The report's `minecraft:use_cooldown`: vanilla 26.3 registers it with seconds only
/// (`Items.ENDER_PEARL`... `.useCooldown(1.0F)`, wind charge 0.5, chorus fruit 1.0), so no
/// item names a `cooldown_group` and every item groups under its own registry id — the
/// fallback of `ItemCooldowns.getCooldownGroup`. (The group field stays mod/datapack
/// territory; the client resolves it off the stack in Java.)
#[test]
#[ignore = "needs cache/datagen/26.3/reports (data generator)"]
fn report_use_cooldowns_name_no_groups() {
    let c = ItemComponents::load_default().unwrap();
    assert!(c.len() > 30, "the report did not load");
    assert!(c.use_cooldown_items().next().is_none());
}
