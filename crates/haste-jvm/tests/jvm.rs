//! JVM tests. They need the cache (JDK, client jar, libraries, ASM) and are ignored by default:
//! `cargo test -p haste-jvm -- --ignored --test-threads=1`.
use std::path::PathBuf;
use std::time::Instant;

use haste_jvm::{ClassSource, JvmConfig, JvmHost};
use jni::objects::{JObject, JString, JValue};
use jni::{jni_sig, jni_str};

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
    eprintln!(
        "host ready in {:?} ({:?}); {}",
        t.elapsed(),
        h.timings(),
        h.proxy_summary()
    );
    h
}

#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM)"]
fn tier3_value_types_from_the_jar() {
    let h = host();
    let t = Instant::now();
    let (id, pos, hi) = h
        .with_env(|env| {
            let idc = h.load_class(env, "net.minecraft.resources.Identifier")?;
            let s = JString::from_str(env, "minecraft:stone")?;
            let id = env
                .call_static_method(
                    &idc,
                    jni_str!("parse"),
                    jni_sig!("(Ljava/lang/String;)Lnet/minecraft/resources/Identifier;"),
                    &[JValue::Object(&s)],
                )?
                .l()?;
            let id = to_string(env, &id)?;
            let first = t.elapsed();

            let bp = h.load_class(env, "net.minecraft.core.BlockPos")?;
            let p = env.new_object(
                &bp,
                jni_sig!("(III)V"),
                &[JValue::Int(1), JValue::Int(2), JValue::Int(3)],
            )?;
            let q = env
                .call_method(
                    &p,
                    jni_str!("offset"),
                    jni_sig!("(III)Lnet/minecraft/core/BlockPos;"),
                    &[JValue::Int(4), JValue::Int(5), JValue::Int(6)],
                )?
                .l()?;
            let x = env
                .call_method(&q, jni_str!("getX"), jni_sig!("()I"), &[])?
                .i()?;
            let y = env
                .call_method(&q, jni_str!("getY"), jni_sig!("()I"), &[])?
                .i()?;
            let z = env
                .call_method(&q, jni_str!("getZ"), jni_sig!("()I"), &[])?
                .i()?;

            let comp = h.load_class(env, "net.minecraft.network.chat.Component")?;
            let s = JString::from_str(env, "hi")?;
            let c = env
                .call_static_method(
                    &comp,
                    jni_str!("literal"),
                    jni_sig!("(Ljava/lang/String;)Lnet/minecraft/network/chat/MutableComponent;"),
                    &[JValue::Object(&s)],
                )?
                .l()?;
            let got = env
                .call_method(
                    &c,
                    jni_str!("getString"),
                    jni_sig!("()Ljava/lang/String;"),
                    &[],
                )?
                .l()?;
            let got = to_string(env, &got)?;
            eprintln!(
                "first call (Identifier.parse incl. class loading): {first:?}; all three: {:?}",
                t.elapsed()
            );
            Ok((id, (x, y, z), got))
        })
        .expect("calls");
    assert_eq!(id, "minecraft:stone");
    assert_eq!(pos, (5, 7, 9));
    assert_eq!(hi, "hi");

    let log = h.class_log().unwrap();
    assert_eq!(
        log.classes.get("net.minecraft.resources.Identifier"),
        Some(&ClassSource::Jar)
    );
    assert_eq!(
        log.classes.get("net.minecraft.core.BlockPos"),
        Some(&ClassSource::Jar)
    );
    // tier-1 types never come from the jar
    let refused = h
        .with_env(|env| {
            Ok(h.load_class(env, "net.minecraft.world.level.block.Blocks")
                .is_err())
        })
        .unwrap();
    assert!(refused, "Blocks must be refused");
    let log = h.class_log().unwrap();
    assert!(matches!(
        log.classes.get("net.minecraft.world.level.block.Blocks"),
        Some(ClassSource::Refused(_))
    ));
    eprintln!(
        "class log: {} jar, {} proxy, {} lib, {} refused",
        log.with(|s| *s == ClassSource::Jar).len(),
        log.with(|s| *s == ClassSource::Proxy).len(),
        log.with(|s| *s == ClassSource::Library).len(),
        log.refused().len()
    );
    for (n, why) in log.refused() {
        eprintln!("  refused {n}: {why}");
    }
}

fn to_string(env: &mut jni::Env, o: &JObject) -> jni::errors::Result<String> {
    let s = env
        .call_method(
            o,
            jni_str!("toString"),
            jni_sig!("()Ljava/lang/String;"),
            &[],
        )?
        .l()?;
    // Safety: toString returns a String.
    let s = unsafe { JString::from_raw(env, s.into_raw()) };
    s.try_to_string(env)
}

static GUI: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn texts(f: &haste_core::gui::GuiFrame) -> Vec<String> {
    f.commands
        .iter()
        .filter_map(|c| match c {
            haste_core::gui::GuiCommand::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn dump(f: &haste_core::gui::GuiFrame) {
    for c in &f.commands {
        eprintln!("  {c:?}");
    }
}

fn sprites(f: &haste_core::gui::GuiFrame) -> Vec<(String, i32, i32, i32, i32)> {
    f.commands
        .iter()
        .filter_map(|c| match c {
            haste_core::gui::GuiCommand::Sprite {
                sprite, x, y, w, h, ..
            } => Some((sprite.to_string(), *x, *y, *w, *h)),
            _ => None,
        })
        .collect()
}

/// Opens a screen and fails with the Unimplemented log if that throws.
fn open(h: &JvmHost, kind: haste_jvm::ScreenKind) {
    if let Err(e) = h.gui().open_screen(kind) {
        panic!(
            "open {kind:?}: {e}\nunimplemented: {:#?}",
            h.unimplemented().unwrap()
        );
    }
}

fn render(h: &JvmHost, mouse: (f64, f64)) -> haste_core::gui::GuiFrame {
    match h.gui().render(320, 240, 2, mouse, 0.0) {
        Ok(f) => f,
        Err(e) => panic!(
            "render: {e}\nunimplemented: {:#?}",
            h.unimplemented().unwrap()
        ),
    }
}

#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM)"]
fn pause_screen() {
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    let gui = h.gui();
    h.clear_unimplemented().unwrap();
    open(&h, haste_jvm::ScreenKind::Pause);
    assert!(gui.is_open());
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.PauseScreen")
    );
    let f = render(&h, (0.0, 0.0));
    dump(&f);
    let t = texts(&f);
    for want in [
        "Game Menu",
        "Back to Game",
        "Advancements",
        "Statistics",
        "Disconnect",
    ] {
        assert!(t.iter().any(|x| x == want), "missing {want:?} in {t:?}");
    }
    let buttons: Vec<_> = sprites(&f)
        .into_iter()
        .filter(|s| s.0.starts_with("minecraft:widget/button"))
        .collect();
    assert!(buttons.len() >= 4, "{buttons:?}");
    // the "Back to Game" button: the sprite under its label
    let label = f
        .commands
        .iter()
        .find_map(|c| match c {
            haste_core::gui::GuiCommand::Text { text, x, y, .. } if text == "Back to Game" => {
                Some((*x, *y))
            }
            _ => None,
        })
        .unwrap();
    let (_, bx, by, bw, bh) = buttons
        .iter()
        .find(|b| b.1 <= label.0 && label.0 < b.1 + b.3 && b.2 <= label.1 && label.1 < b.2 + b.4)
        .cloned()
        .expect("button under label");
    let centre = ((bx + bw / 2) as f64, (by + bh / 2) as f64);

    // hovering highlights the button (vanilla picks the highlighted sprite)
    gui.mouse(centre.0, centre.1, None);
    let hovered = render(&h, centre);
    assert!(
        sprites(&hovered)
            .iter()
            .any(|s| s.0 == "minecraft:widget/button_highlighted" && s.1 == bx && s.2 == by),
        "{:?}",
        sprites(&hovered)
    );

    // clicking "Back to Game" closes the screen through Gui.setScreen(null)
    assert!(
        gui.mouse(
            centre.0,
            centre.1,
            Some((haste_jvm::keys::MOUSE_LEFT, true))
        ),
        "click consumed: {:?}",
        gui.take_last_error()
    );
    gui.mouse(
        centre.0,
        centre.1,
        Some((haste_jvm::keys::MOUSE_LEFT, false)),
    );
    assert_eq!(gui.take_last_error(), None);
    assert!(
        !gui.is_open(),
        "Back to Game should close the pause screen: {:?}",
        gui.take_last_error()
    );
    assert!(gui.mouse_grabbed(), "Back to Game grabs the mouse");

    // Escape closes it as well
    open(&h, haste_jvm::ScreenKind::Pause);
    assert!(gui.key(
        haste_jvm::keys::ESCAPE,
        haste_jvm::keys::keycode(haste_jvm::keys::ESCAPE),
        0,
        true
    ));
    assert!(
        !gui.is_open(),
        "Escape should close the pause screen: {:?}",
        gui.take_last_error()
    );

    let unimpl = h.unimplemented().unwrap();
    assert!(
        unimpl.is_empty(),
        "Unimplemented log for PauseScreen: {unimpl:#?}"
    );
}

#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM); run with --release for the number"]
fn bench_pause_render() {
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    open(&h, haste_jvm::ScreenKind::Pause);
    let gui = h.gui();
    for _ in 0..3000 {
        gui.render(320, 240, 2, (10.0, 10.0), 0.0).unwrap();
    }
    let n = 5000;
    let mut times: Vec<u128> = Vec::with_capacity(n);
    for i in 0..n {
        let t = Instant::now();
        let f = gui
            .render(320, 240, 2, (10.0 + (i % 7) as f64, 10.0), 0.0)
            .unwrap();
        times.push(t.elapsed().as_nanos());
        std::hint::black_box(f);
    }
    times.sort_unstable();
    let mean = times.iter().sum::<u128>() as f64 / n as f64 / 1000.0;
    eprintln!("PauseScreen render(): mean {mean:.1} µs, p50 {:.1} µs, p99 {:.1} µs ({n} frames after 3000 warm-up)",
        times[n / 2] as f64 / 1000.0, times[n * 99 / 100] as f64 / 1000.0);
    gui.close_screen();
}

/// Text drawn on one line (runs joined left to right), e.g. the chat input with its syntax colours.
fn line_at(f: &haste_core::gui::GuiFrame, y: i32) -> String {
    let mut runs: Vec<(i32, &str)> = f
        .commands
        .iter()
        .filter_map(|c| match c {
            haste_core::gui::GuiCommand::Text { text, x, y: ty, .. } if *ty == y => {
                Some((*x, text.as_str()))
            }
            _ => None,
        })
        .collect();
    runs.sort_by_key(|r| r.0);
    runs.into_iter().map(|r| r.1).collect()
}

/// The chat input line: the line holding the cursor or the typed text, at the bottom of the screen.
fn chat_input(f: &haste_core::gui::GuiFrame) -> String {
    let y = f
        .commands
        .iter()
        .filter_map(|c| match c {
            haste_core::gui::GuiCommand::Text { y, .. } => Some(*y),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    line_at(f, y).trim_end_matches('_').to_string()
}

fn type_text(h: &JvmHost, s: &str) {
    for c in s.chars() {
        assert!(
            h.gui().char_typed(c, 0),
            "char {c:?}: {:?}",
            h.gui().take_last_error()
        );
    }
}

fn press(h: &JvmHost, key: i32) -> bool {
    let kc = haste_jvm::keys::keycode(key);
    let r = h.gui().key(key, kc, 0, true);
    h.gui().key(key, kc, 0, false);
    assert_eq!(h.gui().take_last_error(), None);
    r
}

#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM)"]
fn chat_screen() {
    use haste_jvm::keys;
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    let gui = h.gui();
    h.clear_unimplemented().unwrap();
    gui.with_state(|s| {
        s.chat_lines = vec!["<Alex> hello".into(), "<Steve> hi there".into()];
        s.sent.clear();
    });
    open(&h, haste_jvm::ScreenKind::Chat);
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.ChatScreen")
    );
    let f = render(&h, (0.0, 0.0));
    let t = texts(&f);
    assert!(
        t.contains(&"<Alex> hello".to_string()) && t.contains(&"<Steve> hi there".to_string()),
        "history: {t:?}"
    );
    // newest line is drawn lowest
    let y_of = |f: &haste_core::gui::GuiFrame, s: &str| {
        f.commands.iter().find_map(|c| match c {
            haste_core::gui::GuiCommand::Text { text, y, .. } if text == s => Some(*y),
            _ => None,
        })
    };
    assert!(y_of(&f, "<Steve> hi there") > y_of(&f, "<Alex> hello"));

    // typing goes into the real EditBox
    type_text(&h, "hey there");
    assert_eq!(chat_input(&render(&h, (0.0, 0.0))), "hey there");
    assert!(press(&h, keys::BACKSPACE));
    press(&h, keys::BACKSPACE);
    assert_eq!(
        chat_input(&render(&h, (0.0, 0.0))),
        "hey the",
        "after two backspaces"
    );

    // Enter sends the line to Rust and closes the screen
    assert!(press(&h, keys::RETURN));
    assert!(!gui.is_open());
    assert_eq!(
        gui.with_state(|s| s.take_sent()),
        vec![haste_jvm::ChatMessage {
            text: "hey the".into(),
            command: false
        }]
    );

    // a command goes out as a command; Up recalls the previous line from the chat history
    open(&h, haste_jvm::ScreenKind::Chat);
    type_text(&h, "/help");
    assert!(press(&h, keys::RETURN));
    assert_eq!(
        gui.with_state(|s| s.take_sent()),
        vec![haste_jvm::ChatMessage {
            text: "help".into(),
            command: true
        }]
    );
    open(&h, haste_jvm::ScreenKind::Chat);
    press(&h, keys::UP);
    assert_eq!(
        chat_input(&render(&h, (0.0, 0.0))),
        "/help",
        "Up recalls the last sent line"
    );
    press(&h, keys::UP);
    assert_eq!(
        chat_input(&render(&h, (0.0, 0.0))),
        "hey the",
        "Up again recalls the one before"
    );
    assert!(press(&h, keys::ESCAPE));
    assert!(!gui.is_open());
    assert!(
        gui.with_state(|s| s.take_sent()).is_empty(),
        "Escape sends nothing"
    );

    let unimpl = h.unimplemented().unwrap();
    assert!(
        unimpl.is_empty(),
        "Unimplemented log for ChatScreen: {unimpl:#?}"
    );
}

#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM)"]
fn inventory_screen() {
    use haste_core::gui::GuiCommand;
    use haste_core::RegistryIndex;
    use haste_jvm::{keys, ContainerInput, ItemSlot};
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    let gui = h.gui();
    h.clear_unimplemented().unwrap();
    gui.with_state(|s| {
        s.inventory.iter_mut().for_each(|x| *x = None);
        s.inventory[0] = Some(ItemSlot {
            item: RegistryIndex(1),
            count: 64,
        }); // hotbar slot 0
        s.inventory[9] = Some(ItemSlot {
            item: RegistryIndex(42),
            count: 3,
        }); // first main slot
        s.item_names = vec!["minecraft:air".into(), "minecraft:stone".into()];
        s.container_clicks.clear();
    });
    open(&h, haste_jvm::ScreenKind::Inventory);
    let f = render(&h, (0.0, 0.0));
    dump(&f);
    assert!(f.commands.iter().any(|c| matches!(c, GuiCommand::Texture { texture, .. } if texture.path() == "textures/gui/container/inventory.png")));
    // the container is centred (176 px wide in a 320 px GUI) and slots are drawn relative to it
    let origin = f
        .commands
        .iter()
        .find_map(|c| match c {
            GuiCommand::TranslatePush { dx, dy, .. } => Some((*dx, *dy)),
            _ => None,
        })
        .unwrap();
    assert_eq!(origin.0, 72.0);
    let items: Vec<_> = f
        .commands
        .iter()
        .filter_map(|c| match c {
            GuiCommand::Item { item, x, y, count } => Some((item.0, *x, *y, *count)),
            _ => None,
        })
        .collect();
    assert_eq!(
        items,
        vec![(42, 8, 84, 3), (1, 8, 142, 64)],
        "items from the Rust inventory as GuiCommand::Item"
    );
    let t = texts(&f);
    assert!(
        t.contains(&"64".to_string()) && t.contains(&"3".to_string()),
        "stack counts: {t:?}"
    );
    assert!(t.contains(&"Crafting".to_string()), "{t:?}");
    for s in [
        "minecraft:container/slot/helmet",
        "minecraft:container/slot/shield",
        "minecraft:recipe_book/button",
    ] {
        assert!(sprites(&f).iter().any(|x| x.0 == s), "missing sprite {s}");
    }

    // hovering the hotbar stack: highlight + tooltip with the item's name
    let hot = (origin.0 as f64 + 16.0, origin.1 as f64 + 150.0);
    gui.mouse(hot.0, hot.1, None);
    let f = render(&h, hot);
    assert!(
        sprites(&f)
            .iter()
            .any(|s| s.0 == "minecraft:container/slot_highlight_front"),
        "{:?}",
        sprites(&f)
    );
    assert!(
        texts(&f).contains(&"minecraft:stone".to_string()),
        "tooltip: {:?}",
        texts(&f)
    );

    // clicking a slot sends a container click to Rust (menu slot 36 = hotbar 0)
    assert!(
        gui.mouse(hot.0, hot.1, Some((keys::MOUSE_LEFT, true))),
        "{:?}",
        gui.take_last_error()
    );
    gui.mouse(hot.0, hot.1, Some((keys::MOUSE_LEFT, false)));
    let clicks = gui.with_state(|s| s.take_container_clicks());
    eprintln!("clicks: {clicks:?}");
    assert_eq!(
        clicks.first().map(|c| (c.container_id, c.slot, c.input)),
        Some((0, 36, ContainerInput::Pickup))
    );

    // Rust state changes show up on the next frame (proxies read through natives)
    gui.with_state(|s| {
        s.inventory[0] = Some(ItemSlot {
            item: RegistryIndex(1),
            count: 12,
        })
    });
    assert!(texts(&render(&h, (0.0, 0.0))).contains(&"12".to_string()));

    // E (the inventory key) closes the screen
    assert!(press(&h, keys::E));
    assert!(!gui.is_open());
    assert_eq!(
        gui.with_state(|s| s.take_container_clicks())
            .last()
            .map(|c| c.input),
        Some(ContainerInput::Close)
    );

    let unimpl = h.unimplemented().unwrap();
    assert!(
        unimpl.is_empty(),
        "Unimplemented log for InventoryScreen: {unimpl:#?}"
    );
}
