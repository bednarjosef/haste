//! Dropping from the hotbar updates the Java inventory (ignored: needs cache/).
use std::path::PathBuf;

use haste_jvm::{JvmConfig, JvmHost};

fn cache() -> PathBuf {
    let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cache");
    if local.join("versions").is_dir() {
        local
    } else {
        haste_core::paths::cache_dir()
    }
}

/// A stack without components, its item resolved through the data generator's registry
/// report (`protocol_id` of `minecraft:item`).
fn stack(item: &str, count: i32) -> haste_core::entity::ItemStackData {
    let reports = cache().join("datagen/26.3/reports/registries.json");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(reports).unwrap()).unwrap();
    let id = v["minecraft:item"]["entries"][item]["protocol_id"]
        .as_u64()
        .unwrap() as u32;
    haste_core::entity::ItemStackData {
        count,
        item: Some(haste_core::RegistryIndex(id)),
        components_raw: bytes::Bytes::from_static(&[0, 0]),
    }
}

/// `MultiPlayerGameMode.dropItem`'s prediction on the client inventory: one Q on a stack of
/// five leaves four (the HUD reads the Java inventory, so the count on screen drops), and
/// `InventorySync` mirrors the change back to Rust. The whole stack (Ctrl+Q) empties the slot.
#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM, data generator reports)"]
fn remove_from_selected_leaves_the_rest_and_mirrors_back() {
    let h =
        JvmHost::start(JvmConfig::from_cache(&cache()).expect("cache layout")).expect("JVM start");
    let gui = h.gui();
    gui.load_jar_registries().expect("registries from the jar");
    // Five dirt in the selected hotbar slot (vanilla `Inventory` numbering: slot 0 is hotbar
    // 0, and `Inventory.selected` starts there).
    gui.set_inventory_slot(0, &stack("minecraft:dirt", 5))
        .unwrap();
    gui.with_state(|s| s.selected_slot = 0);
    // `Mirror.update` (run from the tick, like the game) carries the selected slot to Java.
    gui.tick();
    // One Q: the removal is not empty, four stay in the slot.
    let empty = gui.remove_from_selected(false).unwrap();
    assert!(!empty, "a stack of five was removed from");
    assert_eq!(
        gui.with_state(|s| s.slot(0).map(|x| x.count)),
        Some(4),
        "the Java inventory keeps four and the mirror follows"
    );
    // One Ctrl+Q: the rest of the stack leaves, the slot empties.
    let empty = gui.remove_from_selected(true).unwrap();
    assert!(!empty, "the remaining four were removed from");
    assert_eq!(
        gui.with_state(|s| s.slot(0).map(|x| x.count)),
        None,
        "an emptied slot has no stack"
    );
    // Dropping from the now-empty slot removes nothing.
    let empty = gui.remove_from_selected(false).unwrap();
    assert!(empty, "nothing left an empty slot");
}
