//! Held-item mapping: what the wire's `minecraft:item` registry ids resolve to when the
//! first-person hand looks them up.

use haste_core::Identifier;
use haste_data::GameData;

/// Items reported invisible in the first-person hand (swords, redstone, torch, potion) and
/// two that visibly work, resolved through `Registries.name("minecraft:item", i)` — the
/// exact path `Renderer::set_item_registry` uses to build the hand's item name table.
/// Guards the mapping against a wrong, shifted or filtered report.
#[test]
#[ignore = "needs cache/datagen/26.3/reports (run via haste setup; HASTE_CACHE overrides)"]
fn reported_held_items_resolve() {
    let reports = haste_core::paths::datagen_reports();
    if !reports.join("blocks.json").is_file() {
        eprintln!("skipped: no real reports at {reports:?} (set HASTE_CACHE or run haste setup)");
        return;
    }
    let d = GameData::load().unwrap();
    for name in [
        "diamond_sword",
        "wooden_sword",
        "redstone",
        "redstone_block",
        "torch",
        "potion",
        "stick",
    ] {
        let idx = d
            .registries
            .index("minecraft:item", &Identifier::minecraft(name))
            .unwrap_or_else(|| panic!("{name} missing from minecraft:item"));
        let back = d
            .registries
            .name("minecraft:item", idx)
            .unwrap_or_else(|| panic!("{name}'s index {idx:?} resolves to nothing"));
        assert_eq!(
            back,
            &Identifier::minecraft(name),
            "{name} at index {idx:?} resolves to {back}"
        );
    }
}

/// Every `minecraft:item` entry's protocol id must be dense and unique in the report.
#[test]
fn dense_ids_without_the_reports() {
    let raw: serde_json::Value = serde_json::from_str(REGISTRY).unwrap();
    let entries: Vec<(String, u64)> = raw["minecraft:item"]["entries"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v["protocol_id"].as_u64().unwrap()))
        .collect();
    let mut ids: Vec<u64> = entries.iter().map(|(_, id)| *id).collect();
    ids.sort();
    let dense = ids.iter().enumerate().all(|(i, id)| *id == i as u64);
    assert!(dense, "protocol ids are not dense: {ids:?}");
}

const REGISTRY: &str = r#"{
  "minecraft:item": {"entries": {
    "minecraft:air": {"protocol_id": 0},
    "minecraft:stone": {"protocol_id": 1},
    "minecraft:diamond_sword": {"protocol_id": 2},
    "minecraft:torch": {"protocol_id": 3}
  }}
}"#;
