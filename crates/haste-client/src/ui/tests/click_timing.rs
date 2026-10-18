//! The click-timing tests: a screen click's packets leave in the frame the click is made
//! (vanilla sends inside the input handler), not at the next tick's drain.

use super::{frame_tests, host};
use crate::ui::{keys, Ui};
use haste_core::entity::ItemStackData;
use haste_jvm::ScreenKind;
use haste_protocol::ServerboundPlay;

/// A click on an open screen's slot sends its `container_click` in the same frame it is
/// made: vanilla's `MouseHandler.onPress` -> `AbstractContainerScreen.slotClicked` ->
/// `MultiPlayerGameMode.handleContainerInput` -> `Connection.send` all run inside the input
/// event, so the immediate sender (the game thread's queue-and-wake, set once the
/// connection exists) is handed the packet right after the input event — and the per-tick
/// [`Ui::drain_outgoing`] drain does not send it a second time.
///
/// Boots the JVM from cache/ (the client jar under it); skips when the jar is absent.
#[test]
fn screen_click_sends_in_the_same_frame() {
    let cache = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cache");
    if !cache
        .join("versions")
        .join(haste_core::paths::MC_VERSION)
        .join("client.jar")
        .is_file()
    {
        eprintln!("skipped: no cache/");
        return;
    }
    let _serial = frame_tests().lock();
    let mut ui = Ui::new(host());
    // The screens need the synchronised registries (a real run gets them from `joined`).
    ui.gui()
        .load_jar_registries()
        .expect("registries from the jar");
    ui.gui().reset_for_test();
    for slot in 0..haste_jvm::INVENTORY_SIZE {
        ui.gui()
            .set_inventory_slot(slot as i32, &haste_jvm::empty_stack())
            .unwrap();
    }
    // What `Ui::joined` installs from a real connection: the tables and the serverbound
    // dispatch that decode what Java sent (this test has no server to join).
    let tables = std::sync::Arc::new(
        haste_protocol::packets::ProtocolTables::load().expect("protocol tables from cache"),
    );
    *ui.shared.serverbound.lock() = Some(haste_protocol::packets::Dispatch::new(
        &tables.packets,
        haste_protocol::packets::State::Play,
        haste_protocol::packets::Direction::Serverbound,
        ServerboundPlay::DECODERS,
    ));
    *ui.shared.tables.lock() = Some(tables);
    // One oak log in hotbar slot 0, the item the click picks up.
    let log = item_stack("minecraft:oak_log", 1);
    ui.gui().set_inventory_slot(0, &log).unwrap();
    ui.gui().with_state(|s| {
        s.in_world = true;
        s.outgoing.clear();
    });
    let _ = ui.frame(320, 240, 2, (0.0, 0.0), 0.0);
    ui.open_screen(ScreenKind::Inventory)
        .expect("the inventory screen opens");
    let _ = ui.frame(320, 240, 2, (0.0, 0.0), 0.0);

    // The game thread's send-at-once seam, recording what it is handed.
    let sent: std::sync::Arc<parking_lot::Mutex<Vec<ServerboundPlay>>> =
        std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let record = sent.clone();
    ui.set_immediate_sender(std::sync::Arc::new(move |packets: &[ServerboundPlay]| {
        record.lock().extend(packets.iter().cloned())
    }));

    // Click hotbar slot 0 (menu slot 36, at (88, 187) in the 320x240 GUI): press and
    // release, as the event loop forwards them. No `ui.tick()` runs in between.
    ui.mouse_moved(88.0, 187.0);
    ui.mouse_button(88.0, 187.0, keys::MOUSE_LEFT, true);
    ui.mouse_button(88.0, 187.0, keys::MOUSE_LEFT, false);

    let clicks: Vec<_> = sent
        .lock()
        .iter()
        .filter_map(|p| match p {
            ServerboundPlay::ContainerClick(c) => Some(c.clone()),
            _ => None,
        })
        .collect();
    assert!(!clicks.is_empty(), "no container_click was sent at once");
    assert_eq!(
        (clicks[0].slot, clicks[0].button, clicks[0].mode),
        (36, 0, 0),
        "the hotbar click: {:?}",
        *sent.lock()
    );
    assert!(
        clicks[0].carried.item.is_some(),
        "the picked-up log: {clicks:?}"
    );
    // The per-tick drain has nothing left: Java's queue was taken by the input event, so
    // the click cannot also leave at the next tick (a double send).
    let mut out = Vec::new();
    ui.drain_outgoing(&mut out);
    assert!(
        !out.iter()
            .any(|p| matches!(p, ServerboundPlay::ContainerClick(_))),
        "the click would be sent twice: {out:?}"
    );
    ui.gui().close_screen();
    ui.gui().with_state(|s| s.in_world = false);
}

/// A stack without components, its item's registry id read from the data generator's
/// report (the same source `haste-protocol`'s tables use).
fn item_stack(name: &str, count: i32) -> ItemStackData {
    let reports = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache")
        .join("datagen/26.3/reports/registries.json");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(reports).unwrap()).unwrap();
    let id = v["minecraft:item"]["entries"][name]["protocol_id"]
        .as_u64()
        .unwrap() as u32;
    ItemStackData {
        count,
        item: Some(haste_core::RegistryIndex(id)),
        components_raw: bytes::Bytes::from_static(&[0, 0]),
    }
}
