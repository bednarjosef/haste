//! The hotbar's item cooldown overlay (ignored: needs cache/): vanilla
//! `GuiGraphicsExtractor.itemCooldown`'s white half-transparent sweep over a stack whose
//! cooldown group is on cooldown, driven end to end — the Java stack's group
//! (`ItemCooldowns.getCooldownGroup`), the `itemCooldown` native, the Rust snapshot and the
//! batched fill.
use std::path::PathBuf;
use std::time::Instant;

use haste_core::gui::GuiCommand;
use haste_core::{game::GameMode, Identifier};
use haste_jvm::{ItemCooldownHud, ItemCooldownsHud, JvmConfig, JvmHost};

fn cache() -> PathBuf {
    let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cache");
    if local.join("versions").is_dir() {
        local
    } else {
        haste_core::paths::cache_dir()
    }
}

fn host() -> JvmHost {
    let t = Instant::now();
    let h =
        JvmHost::start(JvmConfig::from_cache(&cache()).expect("cache layout")).expect("JVM start");
    eprintln!("host ready in {:?} ({:?})", t.elapsed(), h.timings());
    h.gui()
        .load_jar_registries()
        .expect("registries from the jar");
    h
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

fn render(h: &JvmHost) -> haste_core::gui::GuiFrame {
    match h.gui().render(320, 240, 2, (0.0, 0.0), 0.0) {
        Ok(f) => f,
        Err(e) => panic!(
            "render: {e}\nunimplemented: {:#?}",
            h.unimplemented().unwrap()
        ),
    }
}

/// The white half-transparent fills over the whole frame, as (x1, y1, x2, y2).
fn sweep_fills(f: &haste_core::gui::GuiFrame) -> Vec<(i32, i32, i32, i32)> {
    f.commands
        .iter()
        .filter_map(|c| match c {
            GuiCommand::Fill {
                x1,
                y1,
                x2,
                y2,
                color,
            } if *color == 0x7FFF_FFFF => Some((*x1, *y1, *x2, *y2)),
            _ => None,
        })
        .collect()
}

/// The hotbar slot 0's icon, at the 320x240 GUI's hotbar (see `jvm.rs`'s `hud_frame`):
/// x 160 - 90 + 2, y 240 - 16 - 3, 16x16.
const SLOT: (i32, i32) = (160 - 90 + 2, 240 - 16 - 3);

#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM, data generator reports)"]
fn the_hotbar_item_gets_its_cooldown_sweep() {
    let h = host();
    let gui = h.gui();
    gui.set_inventory_slot(0, &stack("minecraft:ender_pearl", 4))
        .unwrap();
    gui.with_state(|s| {
        s.in_world = true;
        s.selected_slot = 0;
        s.game_mode = GameMode::Survival;
    });
    gui.tick();
    // No cooldown published: no sweep anywhere.
    haste_jvm::set_item_cooldowns(ItemCooldownsHud::default());
    let f = render(&h);
    assert!(
        sweep_fills(&f).is_empty(),
        "no cooldown, no sweep: {:?}",
        sweep_fills(&f)
    );

    // An ender pearl's 20-tick cooldown (the server's `cooldown` packet), ten ticks in: the
    // percent is 0.5, so vanilla's floor/ceil split covers the lower eight rows of the icon
    // (`top = y + floor(16 * (1 - c))`, `bottom = top + ceil(16 * c)`).
    haste_jvm::set_item_cooldowns(ItemCooldownsHud {
        tick_count: 100,
        cooldowns: [(
            Identifier::minecraft("ender_pearl"),
            ItemCooldownHud {
                start_tick: 90,
                end_tick: 110,
            },
        )]
        .into_iter()
        .collect(),
    });
    let f = render(&h);
    let (x, y) = SLOT;
    assert_eq!(
        sweep_fills(&f),
        vec![(x, y + 8, x + 16, y + 16)],
        "the sweep covers the bottom half of the slot's icon"
    );
    // The item and its count still draw (the count goes over the sweep, vanilla's
    // `itemDecorations` order: bar, cooldown, count).
    assert!(f.commands.iter().any(|c| matches!(
        c,
        GuiCommand::Item { item, count, .. } if item.0 == pearl_id() && *count == 4
    )));
    let unimpl = h.unimplemented().unwrap();
    assert!(unimpl.is_empty(), "Unimplemented log: {unimpl:#?}");
    let log = h.class_log().unwrap();
    // The classes every screen run refuses for benign reasons (see `jvm.rs`'s
    // `assert_no_refusals`).
    let known = [
        "com.mojang.renderpearl.api.pipeline.RenderPipeline",
        "net.minecraft.client.multiplayer.ServerStatusPinger",
        "jakarta.servlet.Servlet",
        "javax.servlet.Servlet",
        "net.minecraft.client.sounds.MusicManager",
    ];
    let bad: Vec<_> = log
        .refused()
        .into_iter()
        .filter(|(n, _)| !known.contains(n))
        .collect();
    assert!(bad.is_empty(), "classes refused: {bad:#?}");
}

/// `minecraft:ender_pearl`'s registry index (the frame's `Item` commands carry it).
fn pearl_id() -> u32 {
    let reports = cache().join("datagen/26.3/reports/registries.json");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(reports).unwrap()).unwrap();
    v["minecraft:item"]["entries"]["minecraft:ender_pearl"]["protocol_id"]
        .as_u64()
        .unwrap() as u32
}
