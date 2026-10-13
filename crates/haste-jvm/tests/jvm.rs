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
    // screens need the synchronised registries (dialogs, chat types, ...): from the jar's data pack
    static JAR_REGISTRIES: std::sync::Once = std::sync::Once::new();
    JAR_REGISTRIES.call_once(|| {
        let t = Instant::now();
        h.gui()
            .load_jar_registries()
            .expect("registries from the jar");
        eprintln!(
            "synchronised registries from the jar's data pack in {:?}",
            t.elapsed()
        );
    });
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
    // the client engine never comes from the jar
    let refused = h
        .with_env(|env| {
            Ok(
                h.load_class(env, "net.minecraft.client.renderer.LevelRenderer")
                    .is_err(),
            )
        })
        .unwrap();
    assert!(refused, "LevelRenderer must be refused");
    let log = h.class_log().unwrap();
    assert!(matches!(
        log.classes
            .get("net.minecraft.client.renderer.LevelRenderer"),
        Some(ClassSource::Refused(_))
    ));
    // registry data is real vanilla Java (vanilla's bootstrap ran)
    assert_eq!(
        log.classes.get("net.minecraft.world.level.block.Blocks"),
        Some(&ClassSource::Jar)
    );
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
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM)"]
fn title_screen() {
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    let gui = h.gui();
    h.clear_unimplemented().unwrap();
    open(&h, haste_jvm::ScreenKind::Title);
    assert!(gui.is_open());
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.TitleScreen")
    );
    let f = render(&h, (0.0, 0.0));
    dump(&f);
    let t = texts(&f);
    for want in [
        "Singleplayer",
        "Multiplayer",
        "Minecraft Realms",
        "Options...",
        "Quit Game",
        "Copyright Mojang AB. Do not distribute!",
    ] {
        assert!(t.iter().any(|x| x == want), "missing {want:?} in {t:?}");
    }
    // the realms notification bell is Haste's replacement (it must draw nothing), never vanilla's
    assert_eq!(
        h.class_log()
            .unwrap()
            .classes
            .get("com.mojang.realmsclient.gui.screens.RealmsNotificationsScreen"),
        Some(&ClassSource::Replacement)
    );
    let unimpl = h.unimplemented().unwrap();
    assert!(
        unimpl.is_empty(),
        "Unimplemented log for TitleScreen: {unimpl:#?}"
    );
}

#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM)"]
fn options_screen() {
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    let gui = h.gui();
    h.clear_unimplemented().unwrap();
    open(&h, haste_jvm::ScreenKind::Options);
    assert!(gui.is_open());
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.options.OptionsScreen")
    );
    let f = render(&h, (0.0, 0.0));
    dump(&f);
    let t = texts(&f);
    for want in [
        "Options",
        "Done",
        // the FOV slider at the default value, and two of the setting buttons
        "FOV: Normal",
        "Skin Customization...",
        "Music & Sounds...",
    ] {
        assert!(t.iter().any(|x| x == want), "missing {want:?} in {t:?}");
    }
    let unimpl = h.unimplemented().unwrap();
    assert!(
        unimpl.is_empty(),
        "Unimplemented log for OptionsScreen: {unimpl:#?}"
    );
}

#[test]
#[ignore = "JoinMultiplayerScreen.<init> throws NoClassDefFoundError: net/minecraft/client/multiplayer/ServerStatusPinger is deny-listed (its init then needs ServerList too), see crates/haste-jvm/src/loader.rs DENY"]
fn multiplayer_screen() {
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    let gui = h.gui();
    h.clear_unimplemented().unwrap();
    open(&h, haste_jvm::ScreenKind::Multiplayer);
    assert!(gui.is_open());
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.multiplayer.JoinMultiplayerScreen")
    );
    let f = render(&h, (0.0, 0.0));
    dump(&f);
    let t = texts(&f);
    for want in [
        "Play Multiplayer",
        "Join Server",
        "Direct Connection",
        "Add Server",
        "Back",
    ] {
        assert!(t.iter().any(|x| x == want), "missing {want:?} in {t:?}");
    }
    let unimpl = h.unimplemented().unwrap();
    assert!(
        unimpl.is_empty(),
        "Unimplemented log for JoinMultiplayerScreen: {unimpl:#?}"
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
    // received chat goes through vanilla's chat pipeline into the HUD's chat, which the open chat screen shows
    gui.with_state(|s| {
        s.in_world = true;
        s.sent.clear();
    });
    gui.packet(&system_chat("<Alex> hello", false)).unwrap();
    gui.packet(&system_chat("<Steve> hi there", false)).unwrap();
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
    gui.with_state(|s| s.in_world = false);
}

#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM)"]
fn inventory_screen() {
    use haste_core::gui::GuiCommand;
    use haste_jvm::keys;
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    let gui = h.gui();
    h.clear_unimplemented().unwrap();
    for slot in 0..41 {
        gui.set_inventory_slot(slot, &haste_jvm::empty_stack())
            .unwrap();
    }
    gui.set_inventory_slot(0, &stack("minecraft:stone", 64))
        .unwrap(); // hotbar slot 0
    gui.set_inventory_slot(9, &stack("minecraft:oak_planks", 3))
        .unwrap(); // first main slot
    gui.with_state(|s| s.outgoing.clear());
    // Rust's mirror follows Java's inventory
    assert_eq!(
        gui.with_state(|s| s.slot(0).cloned()),
        Some(stack("minecraft:stone", 64))
    );
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
    let (stone, planks) = (item_id("minecraft:stone"), item_id("minecraft:oak_planks"));
    assert_eq!(
        items,
        vec![(planks, 8, 84, 3), (stone, 8, 142, 64)],
        "items from the inventory as GuiCommand::Item"
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
        texts(&f).contains(&"Stone".to_string()),
        "tooltip with the item's translated name: {:?}",
        texts(&f)
    );

    // clicking a slot runs vanilla's click prediction (the stack moves to the cursor) and sends a
    // container_click (menu slot 36 = hotbar 0) with the changed slot and the carried stack, hashed
    assert!(
        gui.mouse(hot.0, hot.1, Some((keys::MOUSE_LEFT, true))),
        "{:?}",
        gui.take_last_error()
    );
    gui.mouse(hot.0, hot.1, Some((keys::MOUSE_LEFT, false)));
    let sent = outgoing(&h);
    let click = sent
        .iter()
        .find(|(id, _)| *id == serverbound_id("minecraft:container_click"))
        .expect("container_click sent");
    let (window, state, slot, button, mode) = container_click_header(&click.1);
    assert_eq!(
        (window, slot, button, mode),
        (0, 36, 0, 0),
        "state id {state}"
    );
    // the prediction is mirrored into Rust: the slot is empty, the cursor carries the stone
    assert!(gui.with_state(|s| s.slot(0).is_none()));
    assert_eq!(
        gui.with_state(|s| s.carried.clone()),
        stack("minecraft:stone", 64)
    );

    // inventory changes show up on the next frame
    gui.set_inventory_slot(9, &stack("minecraft:stone", 12))
        .unwrap();
    assert!(texts(&render(&h, (0.0, 0.0))).contains(&"12".to_string()));

    // E (the inventory key) closes the screen and tells the server (container_close for window 0)
    assert!(press(&h, keys::E));
    assert!(!gui.is_open());
    let sent = outgoing(&h);
    assert!(
        sent.iter()
            .any(|(id, body)| *id == serverbound_id("minecraft:container_close") && body == &[0]),
        "{sent:?}"
    );

    let unimpl = h.unimplemented().unwrap();
    assert!(
        unimpl.is_empty(),
        "Unimplemented log for InventoryScreen: {unimpl:#?}"
    );
}

/// `minecraft:item` registry id of an item, from the data generator's report (Rust's numbering).
fn item_id(name: &str) -> u32 {
    let reports = cache().join("datagen/26.3/reports/registries.json");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(reports).unwrap()).unwrap();
    v["minecraft:item"]["entries"][name]["protocol_id"]
        .as_u64()
        .unwrap() as u32
}

/// A stack without components (the raw patch is "0 added, 0 removed", as `haste-protocol` keeps it).
fn stack(item: &str, count: i32) -> haste_core::entity::ItemStackData {
    haste_core::entity::ItemStackData {
        count,
        item: Some(haste_core::RegistryIndex(item_id(item))),
        components_raw: bytes::Bytes::from_static(&[0, 0]),
    }
}

#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM, data generator reports)"]
fn builtin_registries_match_the_data_generator() {
    let h = host();
    eprintln!("vanilla bootstrap: {}", h.proxy_summary());
    let java = h.gui().registry_sizes().unwrap();
    let reports = cache().join("datagen/26.3/reports/registries.json");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(reports).unwrap()).unwrap();
    let mut checked = 0;
    for (name, reg) in v.as_object().unwrap() {
        let entries = reg["entries"].as_object().unwrap();
        assert_eq!(
            java.get(name.as_str()).copied(),
            Some(entries.len()),
            "size of {name}"
        );
        // every entry has the same numeric id in Java as in the report
        for (entry, e) in entries {
            let id = e["protocol_id"].as_i64().unwrap() as i32;
            assert_eq!(
                h.gui().registry_name(name, id).unwrap().as_deref(),
                Some(entry.as_str()),
                "{name} id {id}"
            );
            checked += 1;
        }
    }
    eprintln!(
        "{} built-in registries, {checked} entries: same ids in Java and in the data generator reports",
        v.as_object().unwrap().len()
    );
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

/// `system_chat` with a plain string component (network NBT: a bare TAG_String).
fn system_chat(text: &str, overlay: bool) -> Vec<u8> {
    let mut b = vec![packet_id("minecraft:system_chat"), 8];
    b.extend_from_slice(&(text.len() as u16).to_be_bytes());
    b.extend_from_slice(text.as_bytes());
    b.push(u8::from(overlay));
    b
}

fn sprite_at(f: &haste_core::gui::GuiFrame, name: &str) -> Vec<(i32, i32, i32, i32)> {
    sprites(f)
        .into_iter()
        .filter(|s| s.0 == name)
        .map(|s| (s.1, s.2, s.3, s.4))
        .collect()
}

#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM, data generator reports)"]
fn hud_frame() {
    use haste_core::gui::GuiCommand;
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    let gui = h.gui();
    gui.close_screen();
    h.clear_unimplemented().unwrap();
    for slot in 0..41 {
        gui.set_inventory_slot(slot, &haste_jvm::empty_stack())
            .unwrap();
    }
    gui.set_inventory_slot(0, &stack("minecraft:stone", 64))
        .unwrap();
    gui.set_inventory_slot(2, &stack("minecraft:oak_planks", 7))
        .unwrap();
    gui.with_state(|s| {
        s.in_world = true;
        s.selected_slot = 2;
        s.game_mode = haste_core::game::GameMode::Survival;
        s.vitals.health = 15.0;
        s.vitals.food = 17;
        s.vitals.xp_level = 5;
        s.vitals.xp_progress = 0.5;
    });
    gui.packet(&system_chat("Hello from the server", false))
        .unwrap();
    gui.tick();
    let f = render(&h, (0.0, 0.0));
    dump(&f);
    // the hotbar, centred at the bottom of a 320x240 GUI
    assert_eq!(
        sprite_at(&f, "minecraft:hud/hotbar"),
        vec![(160 - 91, 240 - 22, 182, 22)]
    );
    // the selection frame over slot 2
    let sel = sprite_at(&f, "minecraft:hud/hotbar_selection");
    assert_eq!(sel.len(), 1, "{sel:?}");
    assert_eq!(sel[0].0, 160 - 91 - 1 + 2 * 20);
    let items: Vec<_> = f
        .commands
        .iter()
        .filter_map(|c| match c {
            GuiCommand::Item { item, x, y, count } => Some((item.0, *x, *y, *count)),
            _ => None,
        })
        .collect();
    assert!(
        items.contains(&(item_id("minecraft:stone"), 160 - 90 + 2, 240 - 16 - 3, 64)),
        "{items:?}"
    );
    assert!(
        items.contains(&(
            item_id("minecraft:oak_planks"),
            160 - 90 + 2 + 40,
            240 - 16 - 3,
            7
        )),
        "{items:?}"
    );
    // 15 health: 7 full hearts and a half one; 17 food: 8 full shanks and a half one
    assert_eq!(sprite_at(&f, "minecraft:hud/heart/full").len(), 7);
    assert_eq!(sprite_at(&f, "minecraft:hud/heart/half").len(), 1);
    assert_eq!(sprite_at(&f, "minecraft:hud/food_full").len(), 8);
    assert_eq!(sprite_at(&f, "minecraft:hud/food_half").len(), 1);
    // the experience bar and level
    assert!(!sprite_at(&f, "minecraft:hud/experience_bar_background").is_empty());
    let t = texts(&f);
    assert!(t.contains(&"5".to_string()), "xp level: {t:?}");
    // the chat line through vanilla's ChatComponent
    assert!(
        t.iter().any(|x| x.contains("Hello from the server")),
        "chat: {t:?}"
    );
    let unimpl = h.unimplemented().unwrap();
    assert!(
        unimpl.is_empty(),
        "Unimplemented log for the HUD: {unimpl:#?}"
    );
    gui.with_state(|s| s.in_world = false);
}

/// Serverbound play packet id from the data generator's `packets.json`.
fn serverbound_id(name: &str) -> i32 {
    let reports = cache().join("datagen/26.3/reports/packets.json");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(reports).unwrap()).unwrap();
    v["play"]["serverbound"][name]["protocol_id"]
        .as_i64()
        .unwrap() as i32
}

fn var_int(b: &[u8], i: &mut usize) -> i32 {
    let mut v: u32 = 0;
    for shift in (0..35).step_by(7) {
        let x = b[*i];
        *i += 1;
        v |= u32::from(x & 0x7f) << shift;
        if x & 0x80 == 0 {
            break;
        }
    }
    v as i32
}

/// Drains the serverbound packets vanilla code sent: (id, body).
fn outgoing(h: &JvmHost) -> Vec<(i32, Vec<u8>)> {
    h.gui()
        .with_state(|s| s.take_outgoing())
        .into_iter()
        .map(|p| {
            let mut i = 0;
            let id = var_int(&p, &mut i);
            (id, p[i..].to_vec())
        })
        .collect()
}

/// container_click body: window (VarInt), state id (VarInt), slot (i16), button (i8), mode (VarInt).
fn container_click_header(b: &[u8]) -> (i32, i32, i16, i8, i32) {
    let mut i = 0;
    let window = var_int(b, &mut i);
    let state = var_int(b, &mut i);
    let slot = i16::from_be_bytes([b[i], b[i + 1]]);
    let button = b[i + 2] as i8;
    i += 3;
    let mode = var_int(b, &mut i);
    (window, state, slot, button, mode)
}

/// Per-frame cost of the vanilla HUD plus an open screen (the pause menu), one JNI call each.
#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM); run with --release for the number"]
fn bench_hud_and_screen() {
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    let gui = h.gui();
    gui.set_inventory_slot(0, &stack("minecraft:stone", 64))
        .unwrap();
    gui.with_state(|s| {
        s.in_world = true;
        s.vitals.health = 15.0;
    });
    gui.packet(&system_chat("a chat line for the bench", false))
        .unwrap();
    for (what, screen) in [
        ("HUD", None),
        ("HUD + PauseScreen", Some(haste_jvm::ScreenKind::Pause)),
    ] {
        if let Some(k) = screen {
            open(&h, k);
        }
        for _ in 0..3000 {
            gui.render(320, 240, 2, (10.0, 10.0), 0.5).unwrap();
        }
        let n = 5000;
        let mut times: Vec<u128> = Vec::with_capacity(n);
        for i in 0..n {
            let t = Instant::now();
            let f = gui
                .render(320, 240, 2, (10.0 + (i % 7) as f64, 10.0), 0.5)
                .unwrap();
            times.push(t.elapsed().as_nanos());
            std::hint::black_box(f);
        }
        times.sort_unstable();
        eprintln!(
            "{what} render(): p50 {:.1} µs, p99 {:.1} µs ({n} frames after 3000 warm-up)",
            times[n / 2] as f64 / 1000.0,
            times[n * 99 / 100] as f64 / 1000.0
        );
    }
    gui.close_screen();
    gui.with_state(|s| s.in_world = false);
}
