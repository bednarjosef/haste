//! The container-result timing test: a `container_set_slot` that arrives between ticks
//! (the crafting result the server computed, `CraftingMenu.slotChangedCraftingGrid`) shows
//! on the very next rendered frame, without a `tick()` in between — vanilla applies
//! packets every frame (`Minecraft.runTick` -> `packetProcessor.processQueuedPackets`),
//! never at the 20 Hz tick, and the client must not add a tick of delay to the result
//! appearing.
//!
//! Needs the cache (JDK, client jar, libraries, ASM, data generator reports) and is
//! ignored by default: `cargo test -p haste-jvm -- --ignored --test-threads=1`.
use std::path::PathBuf;
use std::time::Instant;

use haste_core::entity::ItemStackData;
use haste_core::gui::GuiCommand;
use haste_jvm::{JvmConfig, JvmHost, ScreenKind};

fn cache() -> PathBuf {
    let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cache");
    if local.join("versions").is_dir() {
        local
    } else {
        haste_core::paths::cache_dir()
    }
}

/// The JVM of this test binary, its screens fed with the jar's registries (a real run gets
/// them from the connection's `registry_data`).
fn host() -> JvmHost {
    let h =
        JvmHost::start(JvmConfig::from_cache(&cache()).expect("cache layout")).expect("JVM start");
    h.gui()
        .load_jar_registries()
        .expect("registries from the jar");
    h
}

/// `minecraft:item` registry id of an item, from the data generator's report.
fn item_id(name: &str) -> u32 {
    let reports = cache().join("datagen/26.3/reports/registries.json");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(reports).unwrap()).unwrap();
    v["minecraft:item"]["entries"][name]["protocol_id"]
        .as_u64()
        .unwrap() as u32
}

/// A stack without components (the raw patch is "0 added, 0 removed").
fn stack(item: &str, count: i32) -> ItemStackData {
    ItemStackData {
        count,
        item: Some(haste_core::RegistryIndex(item_id(item))),
        components_raw: bytes::Bytes::from_static(&[0, 0]),
    }
}

/// Play packet id from the data generator's `packets.json`.
fn packet_id(name: &str) -> u8 {
    let reports = cache().join("datagen/26.3/reports/packets.json");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(reports).unwrap()).unwrap();
    let id = v["play"]["clientbound"][name]["protocol_id"]
        .as_u64()
        .unwrap();
    assert!(id < 128, "one-byte VarInt");
    id as u8
}

fn push_var_int(b: &mut Vec<u8>, mut v: u32) {
    loop {
        if v & !0x7f == 0 {
            b.push(v as u8);
            break;
        }
        b.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
}

/// `container_set_slot` for one stack: window and state (VarInts), slot (i16), then the
/// slot's item as vanilla's codec writes it (count, item id, component patch).
fn container_set_slot(window: i32, state: i32, slot: i16, item: &ItemStackData) -> Vec<u8> {
    let mut b = vec![packet_id("minecraft:container_set_slot")];
    push_var_int(&mut b, window as u32);
    push_var_int(&mut b, state as u32);
    b.extend_from_slice(&slot.to_be_bytes());
    match (item.count, item.item) {
        (c, Some(id)) if c > 0 => {
            push_var_int(&mut b, c as u32);
            push_var_int(&mut b, id.0);
            b.extend_from_slice(&item.components_raw);
        }
        _ => push_var_int(&mut b, 0),
    }
    b
}

#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM, data generator reports)"]
fn container_set_slot_shows_on_the_next_frame_without_a_tick() {
    let h = host();
    let gui = h.gui();
    let t = Instant::now();
    gui.reset_for_test();
    for slot in 0..haste_jvm::INVENTORY_SIZE {
        gui.set_inventory_slot(slot as i32, &haste_jvm::empty_stack())
            .unwrap();
    }
    gui.with_state(|s| s.in_world = true);
    // The mirror arrays (game mode, in-world) fill per render; the screen lays out on its
    // first frame too.
    gui.render(320, 240, 2, (0.0, 0.0), 0.0).unwrap();
    gui.open_screen(ScreenKind::Inventory)
        .expect("the inventory screen opens");
    let planks = item_id("minecraft:oak_planks");
    let has_planks = |f: &haste_core::gui::GuiFrame| {
        f.commands.iter().any(
            |c| matches!(c, GuiCommand::Item { item, count, .. } if item.0 == planks && *count == 4),
        )
    };
    assert!(
        !has_planks(&gui.render(320, 240, 2, (0.0, 0.0), 0.0).unwrap()),
        "the crafting result slot starts empty"
    );
    // The server's answer to a grid click arrives between ticks: the packet handler
    // applies it alone, and no `gui.tick()` is called before the next render.
    gui.packet(&container_set_slot(
        0,
        0,
        0, // window 0 (the player's inventory menu), slot 0 (the crafting result)
        &stack("minecraft:oak_planks", 4),
    ))
    .unwrap();
    let f = gui.render(320, 240, 2, (0.0, 0.0), 0.0).unwrap();
    assert!(
        has_planks(&f),
        "the set slot draws on the next frame, without a tick"
    );
    gui.close_screen();
    gui.with_state(|s| s.in_world = false);
    eprintln!(
        "container_set_slot frame timing checked in {:?}",
        t.elapsed()
    );
}
