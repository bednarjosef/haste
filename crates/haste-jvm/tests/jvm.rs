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

/// The JVM of the process (and everything vanilla state in it) is shared by every test below, so no
/// test starts from a clean slate unless it asks for one: `bench_hud_and_screen` leaves stone 64 in
/// hotbar slot 0 and a chat line, which would draw over the chat input this test measures.
fn reset(gui: &haste_jvm::GuiPipeline) {
    gui.reset_for_test();
    // Java's own `Inventory` keeps the stacks the mirror only follows: clear it there too.
    for slot in 0..haste_jvm::INVENTORY_SIZE {
        gui.set_inventory_slot(slot as i32, &haste_jvm::empty_stack())
            .unwrap();
    }
    gui.with_state(|s| {
        s.inventory = vec![haste_jvm::empty_stack(); haste_jvm::INVENTORY_SIZE];
        s.carried = haste_jvm::empty_stack();
        s.selected_slot = 0;
        s.sent.clear();
        s.outgoing.clear();
        s.vitals = haste_jvm::PlayerVitals::default();
        s.debug = haste_jvm::DebugInfo::default();
        s.game_mode = haste_core::game::GameMode::Survival;
        s.permission_level = 0;
        s.game_time = 0;
        s.day_time = 0;
        s.disconnect_requested = false;
        s.in_world = false;
    });
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

/// Where a text is drawn, in absolute GUI pixels (translations applied).
fn text_pos(f: &haste_core::gui::GuiFrame, want: &str) -> Option<(i32, i32)> {
    use haste_core::gui::GuiCommand;
    let mut stack: Vec<(f32, f32)> = vec![(0.0, 0.0)];
    for c in &f.commands {
        match c {
            GuiCommand::TranslatePush { dx, dy, .. } => {
                let (x, y) = *stack.last().unwrap();
                stack.push((x + dx, y + dy));
            }
            GuiCommand::TranslatePop => {
                stack.pop();
            }
            GuiCommand::Text { text, x, y, .. } | GuiCommand::StyledText { text, x, y, .. }
                if text == want =>
            {
                let (dx, dy) = *stack.last().unwrap();
                return Some((x + dx as i32, y + dy as i32));
            }
            _ => {}
        }
    }
    None
}

/// A pipeline Haste did not build, the way a mod registers one: a location of the mod's own and a blend
/// function of four blend factors (src colour, dst colour, src alpha, dst alpha), on vanilla's builder.
fn mod_pipeline<'local>(
    h: &JvmHost,
    env: &mut jni::Env<'local>,
    location: &str,
    factors: [&str; 4],
) -> Result<JObject<'local>, haste_jvm::JvmError> {
    let bf = h.load_class(env, "com.mojang.renderpearl.api.pipeline.BlendFactor")?;
    let function = h.load_class(env, "com.mojang.renderpearl.api.pipeline.BlendFunction")?;
    let target_class = h.load_class(env, "com.mojang.renderpearl.api.pipeline.ColorTargetState")?;
    let snippet = h.load_class(
        env,
        "com.mojang.renderpearl.api.pipeline.RenderPipeline$Snippet",
    )?;
    let pipeline = h.load_class(env, "com.mojang.renderpearl.api.pipeline.RenderPipeline")?;
    let topology_class =
        h.load_class(env, "com.mojang.renderpearl.api.pipeline.PrimitiveTopology")?;
    let mut factor_objects = Vec::with_capacity(4);
    for name in factors {
        let name = jni::strings::JNIString::new(name);
        factor_objects.push(
            env.get_static_field(
                &bf,
                &name,
                jni_sig!("Lcom/mojang/renderpearl/api/pipeline/BlendFactor;"),
            )?
            .l()?,
        );
    }
    let factor_args: Vec<JValue> = factor_objects.iter().map(JValue::Object).collect();
    let blend = env.new_object(
        &function,
        jni_sig!("(Lcom/mojang/renderpearl/api/pipeline/BlendFactor;Lcom/mojang/renderpearl/api/pipeline/BlendFactor;Lcom/mojang/renderpearl/api/pipeline/BlendFactor;Lcom/mojang/renderpearl/api/pipeline/BlendFactor;)V"),
        &factor_args,
    )?;
    let target = env.new_object(
        &target_class,
        jni_sig!("(Lcom/mojang/renderpearl/api/pipeline/BlendFunction;)V"),
        &[JValue::Object(&blend)],
    )?;
    let snippets = env.new_object_array(0, &snippet, JObject::null())?;
    let builder = env
        .call_static_method(
            &pipeline,
            jni_str!("builder"),
            jni_sig!("([Lcom/mojang/renderpearl/api/pipeline/RenderPipeline$Snippet;)Lcom/mojang/renderpearl/api/pipeline/RenderPipeline$Builder;"),
            &[JValue::Object(&snippets)],
        )?
        .l()?;
    let shader = JString::from_str(env, location)?;
    let _ = env.call_method(
        &builder,
        jni_str!("withLocation"),
        jni_sig!(
            "(Ljava/lang/String;)Lcom/mojang/renderpearl/api/pipeline/RenderPipeline$Builder;"
        ),
        &[JValue::Object(&shader)],
    )?;
    let _ = env.call_method(
        &builder,
        jni_str!("withVertexShader"),
        jni_sig!(
            "(Ljava/lang/String;)Lcom/mojang/renderpearl/api/pipeline/RenderPipeline$Builder;"
        ),
        &[JValue::Object(&shader)],
    )?;
    let _ = env.call_method(
        &builder,
        jni_str!("withFragmentShader"),
        jni_sig!(
            "(Ljava/lang/String;)Lcom/mojang/renderpearl/api/pipeline/RenderPipeline$Builder;"
        ),
        &[JValue::Object(&shader)],
    )?;
    let _ = env.call_method(
        &builder,
        jni_str!("withColorTargetState"),
        jni_sig!("(Lcom/mojang/renderpearl/api/pipeline/ColorTargetState;)Lcom/mojang/renderpearl/api/pipeline/RenderPipeline$Builder;"),
        &[JValue::Object(&target)],
    )?;
    let topology = env
        .get_static_field(
            &topology_class,
            jni_str!("QUADS"),
            jni_sig!("Lcom/mojang/renderpearl/api/pipeline/PrimitiveTopology;"),
        )?
        .l()?;
    let _ = env.call_method(
        &builder,
        jni_str!("withPrimitiveTopology"),
        jni_sig!("(Lcom/mojang/renderpearl/api/pipeline/PrimitiveTopology;)Lcom/mojang/renderpearl/api/pipeline/RenderPipeline$Builder;"),
        &[JValue::Object(&topology)],
    )?;
    let p = env
        .call_method(
            &builder,
            jni_str!("build"),
            jni_sig!("()Lcom/mojang/renderpearl/api/pipeline/RenderPipeline;"),
            &[],
        )?
        .l()?;
    Ok(unsafe { JObject::from_raw(env, p.into_raw()) })
}

/// The blend mode `haste.gui.Pipelines` maps a pipeline to (0 normal, 1 multiply, 2 invert).
fn blend_of(h: &JvmHost, env: &mut jni::Env, p: &JObject) -> i32 {
    let cls = h.load_class(env, "haste.gui.Pipelines").expect("Pipelines");
    env.call_static_method(
        &cls,
        jni_str!("blendOf"),
        jni_sig!("(Lcom/mojang/renderpearl/api/pipeline/RenderPipeline;)I"),
        &[JValue::Object(p)],
    )
    .expect("blendOf")
    .i()
    .expect("blendOf returns an int")
}

/// A mod's own pipeline maps by its blend function, not by its name: the vignette's multiplies, the
/// crosshair's inverts, the text selection highlight's additive (vanilla's GUI_TEXT_HIGHLIGHT,
/// BlendFunction.ADDITIVE).
#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM)"]
fn pipeline_blend_maps_by_blend_function() {
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    h.with_env(|env| -> Result<(), haste_jvm::JvmError> {
        let vignette = mod_pipeline(
            &h,
            env,
            "test_mod_vignette",
            ["ZERO", "ONE_MINUS_SRC_COLOR", "ZERO", "ONE"],
        )?;
        assert_eq!(blend_of(&h, env, &vignette), 1);
        let crosshair = mod_pipeline(
            &h,
            env,
            "test_mod_crosshair",
            ["ONE_MINUS_DST_COLOR", "ONE_MINUS_SRC_COLOR", "ONE", "ZERO"],
        )?;
        assert_eq!(blend_of(&h, env, &crosshair), 2);
        let highlight = mod_pipeline(
            &h,
            env,
            "test_mod_text_highlight",
            ["ONE", "ONE", "ONE", "ONE"],
        )?;
        assert_eq!(blend_of(&h, env, &highlight), 3);
        // vanilla's own additive pipelines: the edit box's text selection highlight and the nausea
        // overlay, both built with BlendFunction.ADDITIVE
        let pipelines = h.load_class(env, "net.minecraft.client.renderer.RenderPipelines")?;
        for field in ["GUI_TEXT_HIGHLIGHT", "GUI_NAUSEA_OVERLAY"] {
            let name = jni::strings::JNIString::new(field);
            let p = env
                .get_static_field(
                    &pipelines,
                    &name,
                    jni_sig!("Lcom/mojang/renderpearl/api/pipeline/RenderPipeline;"),
                )?
                .l()?;
            assert!(!p.is_null(), "the GUI's pipelines are installed");
            assert_eq!(blend_of(&h, env, &p), 3, "{field} blends additively");
        }
        Ok(())
    })
    .expect("pipeline blend mapping");
}

#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM)"]
fn pause_screen() {
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    let gui = h.gui();
    h.clear_unimplemented().unwrap();
    reset(gui);
    // the pause menu only exists in a world: closing it (`Gui.setScreen(null)`) must land on
    // no screen, not the no-level title-screen fallback
    gui.with_state(|s| s.in_world = true);
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
fn pause_screen_marks_the_blur() {
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    reset(h.gui());
    // The pause screen blurs the world behind it (vanilla: PauseScreen.extractBackground ->
    // extractBlurredBackground -> blurBeforeThisStratum); the marker lands between the HUD's
    // commands and the screen's own.
    open(&h, haste_jvm::ScreenKind::Pause);
    let f = render(&h, (0.0, 0.0));
    assert!(
        f.blur_after >= 0 && (f.blur_after as usize) < f.commands.len(),
        "pause screen should mark the blur: blur_after {} of {} commands",
        f.blur_after,
        f.commands.len()
    );
    // The chat screen draws no blurred background (vanilla: ChatScreen.extractBackground is
    // empty), and neither does the HUD-only frame with no screen open.
    h.gui().close_screen();
    let f = render(&h, (0.0, 0.0));
    assert_eq!(f.blur_after, -1, "no screen: no blur");
    open(&h, haste_jvm::ScreenKind::Chat);
    let f = render(&h, (0.0, 0.0));
    assert_eq!(f.blur_after, -1, "chat screen must not blur");
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
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM)"]
fn multiplayer_screen() {
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    let gui = h.gui();
    reset(gui);
    h.clear_unimplemented().unwrap();
    // One saved server (vanilla's ServerList reads servers.dat from the game directory, which
    // for tests is the JVM's cache/jvm-game): the entry shows and Join requests its address.
    let game_dir = h.config().game_dir.clone();
    let saved = game_dir.join("servers.dat");
    std::fs::write(&saved, servers_dat("Haste Test", "127.0.0.1:25691")).unwrap();
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
        // the saved entry
        "Haste Test",
    ] {
        assert!(t.iter().any(|x| x == want), "missing {want:?} in {t:?}");
    }
    let unimpl = h.unimplemented().unwrap();
    assert!(
        unimpl.is_empty(),
        "Unimplemented log for JoinMultiplayerScreen: {unimpl:#?}"
    );
    // Selecting the entry and clicking Join goes through vanilla's join() into Haste's
    // ConnectScreen replacement, which hands the address to Rust.
    let p = text_pos(&f, "Haste Test").expect("the saved entry's position");
    gui.mouse(f64::from(p.0 + 2), f64::from(p.1 + 2), Some((1, true)));
    gui.mouse(f64::from(p.0 + 2), f64::from(p.1 + 2), Some((1, false)));
    let join = text_pos(&render(&h, (0.0, 0.0)), "Join Server").expect("the Join button");
    gui.mouse(
        f64::from(join.0 + 2),
        f64::from(join.1 + 2),
        Some((1, true)),
    );
    gui.mouse(
        f64::from(join.0 + 2),
        f64::from(join.1 + 2),
        Some((1, false)),
    );
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.ConnectScreen"),
        "Haste's connecting screen is open"
    );
    if let Some(e) = gui.take_last_error() {
        panic!("clicking Join failed: {e}");
    }
    assert_eq!(
        gui.take_join_request().as_deref(),
        Some("127.0.0.1:25691"),
        "the address was handed to Rust"
    );
    assert_no_refusals(&h);
    gui.close_screen();
    std::fs::remove_file(&saved).unwrap();
}

/// The title screen's Quit Game asks Rust to quit (the window closes).
#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM)"]
fn title_screen_quit() {
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    let gui = h.gui();
    reset(gui);
    h.clear_unimplemented().unwrap();
    open(&h, haste_jvm::ScreenKind::Title);
    let f = render(&h, (0.0, 0.0));
    let p = text_pos(&f, "Quit Game").expect("the Quit button");
    gui.mouse(f64::from(p.0 + 2), f64::from(p.1 + 2), Some((1, true)));
    gui.mouse(f64::from(p.0 + 2), f64::from(p.1 + 2), Some((1, false)));
    assert!(gui.take_quit_request(), "Quit Game set the quit flag");
    assert!(!gui.take_quit_request(), "the flag is cleared by taking it");
    gui.close_screen();
}

/// Vanilla `Gui.setScreen(null)` outside a world opens a fresh title screen (the create
/// screen's Cancel after the empty world list auto-opened it closes to `setScreen(null)`):
/// closing the last screen must not leave the menu without one, and the title screen that
/// comes back has its widgets (a fresh instance, so init runs).
#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM)"]
fn close_screen_outside_world_returns_to_title() {
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    let gui = h.gui();
    reset(gui);
    h.clear_unimplemented().unwrap();
    // the empty world list auto-opens the create screen; its Cancel runs the vanilla
    // `setScreen(null)` fallback under test (an empty singleplayer dir, like a fresh install)
    let saves = h.config().singleplayer_dir.clone();
    let keep: Vec<_> = std::fs::read_dir(&saves)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    for p in &keep {
        let _ = std::fs::remove_dir_all(p);
    }
    open(&h, haste_jvm::ScreenKind::Title);
    let f = render(&h, (0.0, 0.0));
    let p = text_pos(&f, "Singleplayer").expect("the Singleplayer button");
    gui.mouse(f64::from(p.0 + 2), f64::from(p.1 + 2), Some((1, true)));
    gui.mouse(f64::from(p.0 + 2), f64::from(p.1 + 2), Some((1, false)));
    if let Some(e) = gui.take_last_error() {
        panic!("clicking Singleplayer failed: {e}");
    }
    // the empty list's auto-jump lands on the create screen (CreateWorldScreen.openFresh)
    let mut class = gui.screen_class();
    for _ in 0..100 {
        render(&h, (0.0, 0.0)); // the list polls on extractWidgetRenderState
        class = gui.screen_class();
        if class.as_deref() == Some(create_screen_class()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(
        class.as_deref(),
        Some(create_screen_class()),
        "the create screen"
    );
    // Cancel: vanilla closes to setScreen(null), which is the title screen outside a world
    let f = render(&h, (0.0, 0.0));
    let p = text_pos(&f, "Cancel").expect("the Cancel button");
    gui.mouse(f64::from(p.0 + 2), f64::from(p.1 + 2), Some((1, true)));
    gui.mouse(f64::from(p.0 + 2), f64::from(p.1 + 2), Some((1, false)));
    if let Some(e) = gui.take_last_error() {
        panic!("clicking Cancel failed: {e}");
    }
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.TitleScreen"),
        "closing the last screen outside a world returns to the title screen"
    );
    // the title screen that came back draws its buttons again (init re-ran on the fresh instance)
    let f = render(&h, (0.0, 0.0));
    let t = texts(&f);
    for want in ["Singleplayer", "Multiplayer", "Options...", "Quit Game"] {
        assert!(
            t.iter().any(|x| x == want),
            "the returned title screen is missing {want:?} in {t:?}"
        );
    }
    gui.close_screen();
    for p in keep {
        std::fs::create_dir_all(&p).unwrap();
    }
}

/// The create screen's class name (its tab screens share the package).
fn create_screen_class() -> &'static str {
    "net.minecraft.client.gui.screens.worldselection.CreateWorldScreen"
}

/// The singleplayer world list: SelectWorldScreen opens, lists the worlds of the singleplayer
/// dir (vanilla's own LevelStorageSource reading each world's `level.dat`), and "Play Selected
/// World" goes through Haste's WorldOpenFlows replacement, which hands the world to Rust.
#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM)"]
fn select_world_screen() {
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    let gui = h.gui();
    reset(gui);
    h.clear_unimplemented().unwrap();
    // One world in the JVM's singleplayer dir (the config dir in menu mode; cache/jvm-game here):
    // a level.dat with the fields the summary reads (name, game mode, version, last played).
    let saves = h.config().singleplayer_dir.clone();
    let world = saves.join("Haste Test World");
    std::fs::create_dir_all(&world).unwrap();
    std::fs::write(world.join("level.dat"), level_dat("Haste Test World", 1)).unwrap();
    open(&h, haste_jvm::ScreenKind::SelectWorld);
    assert!(gui.is_open());
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.worldselection.SelectWorldScreen")
    );
    // The list loads on the render poll: render until the entry shows (the summary is read on
    // a background future).
    let mut t = Vec::new();
    for _ in 0..100 {
        let f = render(&h, (0.0, 0.0));
        t = texts(&f);
        if t.iter().any(|x| x == "Haste Test World") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    dump(&render(&h, (0.0, 0.0)));
    for want in [
        "Select World",
        "Play Selected World",
        "Create New World",
        "Edit",
        "Delete",
        "Re-Create",
        "Back",
        // the world's entry: name, folder (with last-played date), game-mode line
        "Haste Test World",
        "Haste Test World (",
        "Creative Mode, Version: ",
    ] {
        assert!(
            t.iter().any(|x| x.starts_with(want)),
            "missing {want:?} in {t:?}"
        );
    }
    let unimpl = h.unimplemented().unwrap();
    assert!(
        unimpl.is_empty(),
        "Unimplemented log for SelectWorldScreen: {unimpl:#?}"
    );
    assert_no_refusals(&h);
    // Selecting the entry and clicking Play goes through vanilla's joinWorld into Haste's
    // WorldOpenFlows replacement, which hands the world folder to Rust.
    let f = render(&h, (0.0, 0.0));
    let p = text_pos(&f, "Haste Test World").expect("the world entry's position");
    gui.mouse(f64::from(p.0 + 2), f64::from(p.1 + 2), Some((1, true)));
    gui.mouse(f64::from(p.0 + 2), f64::from(p.1 + 2), Some((1, false)));
    let play = text_pos(&render(&h, (0.0, 0.0)), "Play Selected World").expect("the Play button");
    gui.mouse(
        f64::from(play.0 + 2),
        f64::from(play.1 + 2),
        Some((1, true)),
    );
    gui.mouse(
        f64::from(play.0 + 2),
        f64::from(play.1 + 2),
        Some((1, false)),
    );
    if let Some(e) = gui.take_last_error() {
        panic!("clicking Play failed: {e}");
    }
    assert_eq!(
        gui.take_open_world_request().as_deref(),
        Some("Haste Test World"),
        "the world was handed to Rust"
    );
    gui.close_screen();
    std::fs::remove_dir_all(&world).unwrap();
}

/// Every refused class of the screens this test covers is known and harmless.
fn assert_no_refusals(h: &JvmHost) {
    let known = [
        "com.mojang.renderpearl.api.pipeline.RenderPipeline", // a parameter type of draw calls
        "net.minecraft.client.multiplayer.ServerStatusPinger", // Haste's proxy (pings in Rust)
        "jakarta.servlet.Servlet",                            // log4j probes
        "javax.servlet.Servlet",
        "net.minecraft.client.sounds.MusicManager", // option callbacks (no music)
    ];
    let log = h.class_log().unwrap();
    let bad: Vec<_> = log
        .refused()
        .into_iter()
        .filter(|(n, _)| !known.contains(n))
        .collect();
    assert!(bad.is_empty(), "classes refused: {bad:#?}");
}

/// A minimal `servers.dat` (vanilla's format: uncompressed NBT, one entry) for the multiplayer
/// screen tests. Built by hand: TAG_Compound "": { TAG_List "servers": [ TAG_Compound { ... } ] }.
fn servers_dat(name: &str, ip: &str) -> Vec<u8> {
    fn string(bytes: &mut Vec<u8>, s: &str) {
        bytes.extend_from_slice(&(s.len() as u16).to_be_bytes());
        bytes.extend_from_slice(s.as_bytes());
    }
    let mut b = vec![0x0a];
    string(&mut b, "");
    b.extend_from_slice(&[0x09]); // TAG_List
    string(&mut b, "servers");
    b.push(0x0a); // of compounds
    b.extend_from_slice(&1i32.to_be_bytes()); // one entry
    b.extend_from_slice(&[0x08]); // TAG_String ip
    string(&mut b, "ip");
    string(&mut b, ip);
    b.extend_from_slice(&[0x08]); // TAG_String name
    string(&mut b, "name");
    string(&mut b, name);
    b.push(0x00); // end of the entry
    b.push(0x00); // end of the root
    b
}

/// The current world data version as Java sees it (`SharedConstants.getCurrentVersion()`, the
/// version.json of the player's jar): a world's `level.dat` carries it so the world list does
/// not mark a test world as needing a file fix.
fn world_data_version(h: &JvmHost) -> i32 {
    h.with_env(|env| {
        let cls = h.load_class(env, "net.minecraft.SharedConstants")?;
        let v = env
            .call_static_method(
                &cls,
                jni_str!("getCurrentVersion"),
                jni_sig!("()Lnet/minecraft/WorldVersion;"),
                &[],
            )?
            .l()?;
        let v = env
            .call_method(
                &v,
                jni_str!("dataVersion"),
                jni_sig!("()Lnet/minecraft/world/level/storage/DataVersion;"),
                &[],
            )?
            .l()?;
        let n = env
            .call_method(&v, jni_str!("version"), jni_sig!("()I"), &[])?
            .i()?;
        Ok(n)
    })
    .unwrap()
}

/// A minimal world `level.dat` (vanilla's format: gzip'd NBT) with the fields the world list's
/// summary reads: name, game mode, difficulty, the version block and last-played time.
fn level_dat(name: &str, game_type: i32) -> Vec<u8> {
    fn tag(bytes: &mut Vec<u8>, id: u8, name: &str) {
        bytes.push(id);
        bytes.extend_from_slice(&(name.len() as u16).to_be_bytes());
        bytes.extend_from_slice(name.as_bytes());
    }
    fn string(bytes: &mut Vec<u8>, id: u8, name: &str, value: &str) {
        tag(bytes, id, name);
        bytes.extend_from_slice(&(value.len() as u16).to_be_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
    fn int(bytes: &mut Vec<u8>, name: &str, value: i32) {
        tag(bytes, 0x03, name);
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    fn long(bytes: &mut Vec<u8>, name: &str, value: i64) {
        tag(bytes, 0x04, name);
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    let h = host();
    let version = world_data_version(&h);
    let mut b = Vec::new();
    tag(&mut b, 0x0a, ""); // root compound
    tag(&mut b, 0x0a, "Data");
    int(&mut b, "version", 19133);
    int(&mut b, "DataVersion", version);
    long(&mut b, "LastPlayed", 1_600_000_000_000);
    string(&mut b, 0x08, "LevelName", name);
    int(&mut b, "GameType", game_type);
    int(&mut b, "allowCommands", 0);
    tag(&mut b, 0x0a, "difficulty_settings");
    string(&mut b, 0x08, "difficulty", "normal");
    tag(&mut b, 0x01, "hardcore");
    b.push(0);
    tag(&mut b, 0x01, "locked");
    b.push(0);
    b.push(0x00); // end of difficulty_settings
    tag(&mut b, 0x0a, "Version");
    string(&mut b, 0x08, "Name", "26.3");
    int(&mut b, "Id", version);
    string(&mut b, 0x08, "Series", "main");
    tag(&mut b, 0x01, "Snapshot");
    b.push(0);
    b.push(0x00); // end of Version
    b.push(0x00); // end of Data
    b.push(0x00); // end of the root
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut gz, &b).unwrap();
    gz.finish().unwrap()
}

#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM); run with --release for the number"]
fn bench_pause_render() {
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    reset(h.gui());
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
    reset(gui);
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
                   // an inventory only exists in a world: closing it with E (`Gui.setScreen(null)`) must land
                   // on no screen, not the no-level title-screen fallback
    gui.with_state(|s| {
        s.in_world = true;
        s.outgoing.clear();
    });
    // Rust's mirror follows Java's inventory
    assert_eq!(
        gui.with_state(|s| s.slot(0).cloned()),
        Some(stack("minecraft:stone", 64))
    );
    open(&h, haste_jvm::ScreenKind::Inventory);
    let f = render(&h, (0.0, 0.0));
    dump(&f);
    assert!(f.commands.iter().any(|c| matches!(c, GuiCommand::Texture { texture, .. } if texture.path() == "textures/gui/container/inventory.png")));
    // the container is centred (176 px wide in a 320 px GUI) and slots are drawn relative to it;
    // the HUD's own translates come first (a world is joined), so the container's push is the
    // x=72 one, not the first
    let origin = f
        .commands
        .iter()
        .find_map(|c| match c {
            GuiCommand::TranslatePush { dx, dy, .. } if *dx == 72.0 => Some((*dx, *dy)),
            _ => None,
        })
        .expect("the container is centred at x=72");
    let items: Vec<_> = f
        .commands
        .iter()
        .filter_map(|c| match c {
            GuiCommand::Item { item, x, y, count } => Some((item.0, *x, *y, *count)),
            _ => None,
        })
        .collect();
    let (stone, planks) = (item_id("minecraft:stone"), item_id("minecraft:oak_planks"));
    // the HUD is drawn too (a world is joined): the hotbar's slot 0 holds the stone
    assert_eq!(
        items,
        vec![
            (stone, 72, 221, 64),
            (planks, 8, 84, 3),
            (stone, 8, 142, 64)
        ],
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

/// The creative inventory: vanilla's `CreativeModeInventoryScreen` over the connection's
/// feature flags and search trees. In creative mode E opens it, the tabs are built for the
/// server's enabled features (chest and oak_door appear), the search tab filters by typed
/// text, and picking an item from a tab sends `set_creative_slot` (which vanilla's kept
/// `handleCreativeModeItemAdd` gates on `isFeatureEnabled`).
#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM, data generator reports)"]
fn creative_inventory_and_search() {
    use haste_core::gui::GuiCommand;
    use haste_jvm::keys;
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    let gui = h.gui();
    h.clear_unimplemented().unwrap();
    reset(gui);
    gui.with_state(|s| {
        s.in_world = true;
        s.game_mode = haste_core::game::GameMode::Creative;
        s.permission_level = 0;
    });
    // One frame first: the mirror arrays (game mode) are filled per render/tick, and the
    // inventory screen's init asks hasInfiniteMaterials while opening.
    let _ = render(&h, (0.0, 0.0));
    // E opens the inventory, whose init swaps in the creative screen (vanilla:
    // InventoryScreen.init when hasInfiniteMaterials)
    open(&h, haste_jvm::ScreenKind::Inventory);
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.inventory.CreativeModeInventoryScreen"),
        "the creative screen replaces the inventory screen"
    );
    // The default tab is Building Blocks: its items are the ones the enabled features allow
    // (the 195x136 panel is centred in the 320x240 GUI: leftPos 62, topPos 52).
    let f = render(&h, (0.0, 0.0));
    let items = |f: &haste_core::gui::GuiFrame| {
        f.commands
            .iter()
            .filter_map(|c| match c {
                GuiCommand::Item { item, .. } => Some(item.0),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let tab_items = items(&f);
    assert!(
        tab_items.contains(&item_id("minecraft:chest")),
        "chest in the tab items (enabled features): first frame's {:?}",
        tab_items.iter().take(5).collect::<Vec<_>>()
    );
    // Click the search tab (the rightmost top tab: getTabX for the aligned-right column 6 is
    // 195-27+1 = 169, so x 62+169..62+195 = 231..257, y 52-32 = 20..52 in GUI pixels) and type
    // "stone": the results are the stone family.
    assert!(
        gui.mouse(244.0, 36.0, Some((keys::MOUSE_LEFT, true))),
        "clicking the search tab"
    );
    assert!(gui.mouse(244.0, 36.0, Some((keys::MOUSE_LEFT, false))));
    for c in "stone".chars() {
        assert!(
            gui.char_typed(c, 0),
            "typing {c:?}: {:?}",
            gui.take_last_error()
        );
    }
    let f = render(&h, (0.0, 0.0));
    let found = items(&f);
    assert!(
        found.contains(&item_id("minecraft:stone")),
        "stone in the search results: {found:?}"
    );
    assert!(
        found.contains(&item_id("minecraft:cobblestone")),
        "cobblestone in the search results: {found:?}"
    );
    assert!(
        !found.contains(&item_id("minecraft:oak_planks")),
        "oak planks not in the stone results: {found:?}"
    );
    // The tab strip is still drawn (the search tab's icon is the compass)
    assert!(found.contains(&item_id("minecraft:compass")));
    let unimpl = h.unimplemented().unwrap();
    assert!(
        unimpl.is_empty(),
        "Unimplemented log for CreativeModeInventoryScreen: {unimpl:#?}"
    );
    // Escape closes the screen through vanilla's onClose -> closeContainer, which restores the
    // player's containerMenu (the creative screen replaced it with its item picker menu) and
    // sends container_close for the picker's window id. (E would type into the focused search
    // box: the search tab captures it, so vanilla closes this screen with Escape.)
    assert!(press(&h, keys::ESCAPE));
    assert!(!gui.is_open());
    assert!(
        outgoing(&h)
            .iter()
            .any(|(id, _)| *id == serverbound_id("minecraft:container_close")),
        "container_close sent"
    );
    gui.with_state(|s| {
        s.game_mode = haste_core::game::GameMode::Survival;
        s.in_world = false;
    });
    // The mirror arrays only refresh on render/tick: update them so the game mode is back
    // before any later test opens a screen.
    gui.tick();
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

/// `minecraft:block_entity_type` registry id, from the data generator's report.
fn block_entity_id(name: &str) -> i32 {
    let reports = cache().join("datagen/26.3/reports/registries.json");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(reports).unwrap()).unwrap();
    v["minecraft:block_entity_type"]["entries"][name]["protocol_id"]
        .as_i64()
        .unwrap() as i32
}

/// `block_entity_data` for an empty entity of `kind` at `pos` (an empty network-NBT compound).
fn block_entity_data(pos: haste_core::BlockPos, kind: i32) -> Vec<u8> {
    let mut b = vec![packet_id("minecraft:block_entity_data")];
    b.extend_from_slice(&pos.to_packed().to_be_bytes());
    let mut k = kind as u32;
    loop {
        if k & !0x7f == 0 {
            b.push(k as u8);
            break;
        }
        b.push((k as u8 & 0x7f) | 0x80);
        k >>= 7;
    }
    b.extend_from_slice(&[0x0a, 0x00]); // TAG_Compound with no entries (TAG_End)
    b
}

/// `open_sign_editor` for the sign at `pos` (front slot).
fn open_sign_editor(pos: haste_core::BlockPos) -> Vec<u8> {
    let mut b = vec![packet_id("minecraft:open_sign_editor")];
    b.extend_from_slice(&pos.to_packed().to_be_bytes());
    b.push(1); // front slot (VarInt)
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
    h.clear_unimplemented().unwrap();
    reset(gui);
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

/// F3: the debug overlay is drawn over the HUD in every frame while it is on (vanilla
/// `Gui.extractRenderState` draws it after the screen, `Gui.java:200`), and stops when toggled off.
#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM, data generator reports)"]
fn debug_overlay_f3() {
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    let gui = h.gui();
    h.clear_unimplemented().unwrap();
    reset(gui);
    gui.with_state(|s| {
        s.in_world = true;
        s.game_mode = haste_core::game::GameMode::Survival;
    });
    let before = render(&h, (0.0, 0.0));
    let count_before = texts(&before).len();
    gui.toggle_debug();
    let after = render(&h, (0.0, 0.0));
    let t = texts(&after);
    eprintln!(
        "debug_overlay_f3: {count_before} text commands before F3, {} after",
        t.len()
    );
    // the overlay's vanilla-format lines (DebugScreenEntries' defaults)
    assert!(
        t.iter().any(|x| x.starts_with("Minecraft ")),
        "version line: {t:?}"
    );
    assert!(t.iter().any(|x| x.contains(" fps T: ")), "fps line: {t:?}");
    assert!(
        t.iter().any(|x| x.starts_with("XYZ: ")),
        "position line: {t:?}"
    );
    assert!(
        t.iter().any(|x| x.starts_with("Chunk: ")),
        "chunk line: {t:?}"
    );
    assert!(
        t.iter().any(|x| x.starts_with("Facing: ")),
        "facing line: {t:?}"
    );
    assert!(
        t.iter().any(|x| x.starts_with("Mem: ")),
        "memory line: {t:?}"
    );
    assert!(
        t.iter().any(|x| x.starts_with("Java: ")),
        "system spec line: {t:?}"
    );
    assert!(
        t.len() > count_before,
        "F3 added no text: {count_before} before, {} after",
        t.len()
    );
    // toggled off: the overlay's lines are gone
    gui.toggle_debug();
    let off = render(&h, (0.0, 0.0));
    assert_eq!(texts(&off).len(), count_before);
    let unimpl = h.unimplemented().unwrap();
    assert!(unimpl.is_empty(), "Unimplemented log for F3: {unimpl:#?}");
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
    reset(gui);
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

/// Every options sub-screen opens and closes without Unimplemented entries or Java exceptions,
/// like scripts/checks/options-all.txt does through the whole client (which needs a server; this
/// is the JVM-only half). GUI 480x270 at scale 2, the offscreen script client's size; the button
/// coordinates are vanilla's layout (verified against the dumped text positions).
#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM)"]
fn options_screens_open_and_close() {
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    let gui = h.gui();
    reset(gui);
    h.clear_unimplemented().unwrap();
    // the pause menu the options screens open from only exists in a world
    gui.with_state(|s| s.in_world = true);
    open(&h, haste_jvm::ScreenKind::Pause);
    // the script client renders at 480x270 (960x540 at gui scale 2); a first frame also
    // re-lays the pause menu out for that size before the clicks
    gui.render(480, 270, 2, (-1.0, -1.0), 0.5).unwrap();
    // "Options..." (pause menu row 5, left column)
    click(gui, 165.0, 158.0);
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.options.OptionsScreen"),
        "{:?}",
        gui.take_last_error()
    );
    // Every sub-screen: (button centre, class simple name). The Telemetry Data button stays out:
    // vanilla greys it out while telemetry is unavailable (Haste sends none) and its tooltip shows.
    for (x, y, want) in [
        (320.0, 44.0, "OnlineOptionsScreen"),
        (160.0, 102.0, "SkinCustomizationScreen"),
        (336.0, 102.0, "SoundOptionsScreen"),
        (160.0, 126.0, "VideoSettingsScreen"),
        (336.0, 126.0, "ControlsScreen"),
        (160.0, 150.0, "LanguageSelectScreen"),
        (336.0, 150.0, "ChatOptionsScreen"),
        (160.0, 174.0, "PackSelectionScreen"),
        (336.0, 174.0, "AccessibilityOptionsScreen"),
        (336.0, 198.0, "CreditsAndAttributionScreen"),
    ] {
        click(gui, x, y);
        let got = gui.screen_class();
        assert_eq!(
            got.as_deref().map(|c| c.rsplit('.').next().unwrap_or(c)),
            Some(want),
            "click at {x},{y}: {got:?} (last error {:?})",
            gui.take_last_error()
        );
        // one frame of it, with the mouse over the button (tooltip)
        gui.render(480, 270, 2, (x, y), 0.5).unwrap();
        escape(gui);
        assert_eq!(
            gui.screen_class().as_deref(),
            Some("net.minecraft.client.gui.screens.options.OptionsScreen"),
            "Escape from {want}: {:?} (last error {:?})",
            gui.screen_class(),
            gui.take_last_error()
        );
    }
    // Key Binds and Mouse Settings from the Controls screen (its top row). A list's buttons get
    // their positions during rendering (OptionsList.Entry.extractContent, like vanilla's frame
    // loop), so render one frame of the Controls screen before clicking them.
    click(gui, 336.0, 126.0);
    gui.render(480, 270, 2, (336.0, 126.0), 0.5).unwrap();
    for (x, want) in [(160.0, "MouseSettingsScreen"), (336.0, "KeyBindsScreen")] {
        click(gui, x, 48.0);
        let got = gui.screen_class();
        assert_eq!(
            got.as_deref().map(|c| c.rsplit('.').next().unwrap_or(c)),
            Some(want),
            "Controls sub-screen at {x}: {got:?} (last error {:?})",
            gui.take_last_error()
        );
        escape(gui);
    }
    escape(gui);
    // and the Options screen closes back to the pause menu, which closes
    escape(gui);
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.PauseScreen"),
        "{:?}",
        gui.take_last_error()
    );
    escape(gui);
    assert!(!gui.is_open(), "{:?}", gui.take_last_error());
    let unimpl = h.unimplemented().unwrap();
    assert!(
        unimpl.is_empty(),
        "Unimplemented log for the options screens: {unimpl:#?}"
    );
}

/// Every pause-menu screen opens and closes without Unimplemented entries or Java exceptions,
/// like scripts/checks/pause-all.txt does through the whole client (which needs a server). GUI
/// 480x270 at scale 2; the button coordinates are vanilla's layout (see the script).
#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM)"]
fn pause_screens_open_and_close() {
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    let gui = h.gui();
    reset(gui);
    h.clear_unimplemented().unwrap();
    // the pause menu only exists in a world (its final Escape closes to no screen)
    gui.with_state(|s| s.in_world = true);
    open(&h, haste_jvm::ScreenKind::Pause);
    // every frame at the script client's size: a render at another size re-lays the widgets out
    let frame = |mouse: (f64, f64)| -> haste_core::gui::GuiFrame {
        gui.render(480, 270, 2, mouse, 0.5)
            .unwrap_or_else(|e| panic!("render: {e}"))
    };
    frame((-1.0, -1.0));
    // Advancements (row 1 left): no server has sent the tree, so vanilla's empty label shows
    click(gui, 187.0, 110.0);
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.advancements.AdvancementsScreen"),
        "{:?}",
        gui.take_last_error()
    );
    let f = frame((0.0, 0.0));
    assert!(
        texts(&f).iter().any(|x| x == "Advancements"),
        "{:?}",
        texts(&f)
    );
    escape(gui);
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.PauseScreen"),
        "{:?}",
        gui.take_last_error()
    );
    // Statistics (row 1 right): the loading tabs (the server's values never arrive here)
    click(gui, 293.0, 110.0);
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.achievement.StatsScreen"),
        "{:?}",
        gui.take_last_error()
    );
    escape(gui);
    // the icon row: report bugs (a link confirmation), player reporting (social interactions)
    click(gui, 204.0, 134.0);
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.ConfirmLinkScreen"),
        "{:?}",
        gui.take_last_error()
    );
    escape(gui);
    click(gui, 276.0, 134.0);
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.social.SocialInteractionsScreen"),
        "{:?}",
        gui.take_last_error()
    );
    escape(gui);
    // World Options (row 5 right): the dedicated-server rows (no integrated server)
    click(gui, 293.0, 158.0);
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.WorldOptionsScreen"),
        "{:?}",
        gui.take_last_error()
    );
    let f = frame((0.0, 0.0));
    for want in ["Difficulty", "Edit Game Rules...", "Restrictions..."] {
        assert!(
            texts(&f).iter().any(|x| x.contains(want)),
            "missing {want:?} in {:?}",
            texts(&f)
        );
    }
    escape(gui);
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.PauseScreen"),
        "{:?}",
        gui.take_last_error()
    );
    escape(gui);
    assert!(!gui.is_open(), "{:?}", gui.take_last_error());
    let unimpl = h.unimplemented().unwrap();
    assert!(
        unimpl.is_empty(),
        "Unimplemented log for the pause screens: {unimpl:#?}"
    );
}

/// Picking German in Options > Language reloads the client's de_de translations into vanilla's
/// Language at once (LanguageManager's reload listener over the client's resources as
/// CLIENT_RESOURCES), so every screen draws in German: the pause menu shows "Zurück zum Spiel".
/// The whole-client version (with the saved options.txt) is scripts/checks/language.txt.
#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM)"]
fn language_switch_german() {
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    let gui = h.gui();
    reset(gui);
    h.clear_unimplemented().unwrap();
    open(&h, haste_jvm::ScreenKind::Pause);
    let frame = |mouse: (f64, f64)| -> haste_core::gui::GuiFrame {
        gui.render(480, 270, 2, mouse, 0.5)
            .unwrap_or_else(|e| panic!("render: {e}"))
    };
    frame((-1.0, -1.0));
    // Options... (row 5 left) then Language... (row 3 left), by coordinate (language-independent)
    click(gui, 165.0, 158.0);
    click(gui, 160.0, 150.0);
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.options.LanguageSelectScreen"),
        "{:?}",
        gui.take_last_error()
    );
    // the client's pack.mcmeta language section lists every language (the list opens on en_us)
    let f = frame((0.0, 0.0));
    assert!(
        texts(&f).iter().any(|x| x == "English (US)"),
        "language list: {:?}",
        texts(&f)
    );
    // the search box has initial focus: filter to German and pick Germany
    for c in "deutsch".chars() {
        gui.char_typed(c, 0);
    }
    let f = frame((0.0, 0.0));
    let (ex, ey) = text_pos(&f, "Deutsch (Deutschland)").expect("German entry after the filter");
    click(gui, f64::from(ex), f64::from(ey));
    let f = frame((0.0, 0.0));
    let (dx, dy) = text_pos(&f, "Done").expect("Done button");
    click(gui, f64::from(dx), f64::from(dy));
    // Done applied and reloaded; back to the pause menu, now German
    escape(gui);
    let f = frame((0.0, 0.0));
    assert!(
        texts(&f).iter().any(|x| x == "Zurück zum Spiel"),
        "pause menu after switching to German: {:?}",
        texts(&f)
    );
    // switch back to English so the shared JVM (and the saved options.txt) stay en_us
    click(gui, 165.0, 158.0);
    click(gui, 160.0, 150.0);
    for c in "english".chars() {
        gui.char_typed(c, 0);
    }
    let f = frame((0.0, 0.0));
    let (ex, ey) = text_pos(&f, "English (US)").expect("English entry");
    click(gui, f64::from(ex), f64::from(ey));
    let f = frame((0.0, 0.0));
    let (dx, dy) = text_pos(&f, "Fertig").expect("German Done button");
    click(gui, f64::from(dx), f64::from(dy));
    escape(gui);
    let unimpl = h.unimplemented().unwrap();
    assert!(
        unimpl.is_empty(),
        "Unimplemented for the language switch: {unimpl:#?}"
    );
}

fn click(gui: &haste_jvm::GuiPipeline, x: f64, y: f64) {
    gui.mouse(x, y, Some((haste_jvm::keys::MOUSE_LEFT, true)));
    gui.mouse(x, y, Some((haste_jvm::keys::MOUSE_LEFT, false)));
}

#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM)"]
fn pack_selection_screen_lists_folder_packs() {
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    let gui = h.gui();
    reset(gui);
    h.clear_unimplemented().unwrap();
    // A folder pack in the JVM game dir's resourcepacks (vanilla's FolderRepositorySource
    // discovers it): the screen must list it under Available, and Done must commit the
    // selection through Minecraft.reloadResourcePacks (the ClientImpl proxy).
    let game_dir = h.config().game_dir.clone();
    let pack = game_dir.join("resourcepacks/jvm-probe-pack");
    std::fs::create_dir_all(pack.join("assets/minecraft/textures/block")).unwrap();
    std::fs::write(
        pack.join("pack.mcmeta"),
        r#"{"pack":{"min_format":97,"max_format":97,"description":"probe"}}"#,
    )
    .unwrap();
    open(&h, haste_jvm::ScreenKind::Packs);
    assert!(gui.is_open());
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.packs.PackSelectionScreen")
    );
    let f = render(&h, (0.0, 0.0));
    let t = texts(&f);
    for want in ["Select Resource Packs", "Done"] {
        assert!(t.iter().any(|x| x == want), "missing {want:?} in {t:?}");
    }
    assert!(
        t.iter().any(|x| x == "jvm-probe-pack"),
        "the folder pack should be listed: {t:?}"
    );
    // The entry's list by its x position: at 320 wide the Available (left) list renders
    // around x < 160 and the Selected (right) one above it.
    let entry_selected =
        |f: &haste_core::gui::GuiFrame| text_pos(f, "jvm-probe-pack").is_some_and(|(x, _)| x > 160);
    // Clicking the entry's 32x32 icon selects or unselects it (the title itself does
    // nothing, TransferableSelectionList.PackEntry.mouseClicked); the title renders at
    // the icon's right + 2, one pixel lower. Selecting takes a click anywhere on the
    // icon; unselecting takes its left half (SelectableEntry.mouseOverLeftHalf).
    let click_entry =
        |gui: &haste_jvm::GuiPipeline, f: &haste_core::gui::GuiFrame, unselect: bool| {
            let (x, y) = text_pos(f, "jvm-probe-pack").expect("the entry renders");
            let dx = if unselect { -26 } else { -18 };
            click(gui, f64::from(x + dx), f64::from(y + 15));
        };
    // A previous run may have committed the same selection already (the JVM's Options
    // object persists for the process, and options.txt in its game dir across runs):
    // unselect and commit first when needed, so the selection this test commits is a
    // change (vanilla's Options.updateResourcePacks only reloads on a changed list).
    if entry_selected(&f) {
        click_entry(gui, &f, true);
        let f = render(&h, (0.0, 0.0));
        assert!(!entry_selected(&f), "the unselect click should move it out");
        let (x, y) = text_pos(&f, "Done").expect("the Done button renders");
        click(gui, x as f64, y as f64);
        // Done closes to the null parent, and outside a world that is the title screen
        // (vanilla Gui.setScreen's no-level fallback)
        assert_eq!(
            gui.screen_class().as_deref(),
            Some("net.minecraft.client.gui.screens.TitleScreen"),
            "Done should close the screen to the title screen"
        );
        assert_eq!(
            gui.take_resource_pack_ids(),
            Some(vec!["vanilla".to_string()]),
            "the unselected commit should reload too"
        );
        open(&h, haste_jvm::ScreenKind::Packs);
    }
    let f = render(&h, (0.0, 0.0));
    assert!(!entry_selected(&f), "the pack should start unselected");
    click_entry(gui, &f, false);
    let f = render(&h, (0.0, 0.0));
    assert!(entry_selected(&f), "the entry click should select the pack");
    let (x, y) = text_pos(&f, "Done").expect("the Done button renders");
    click(gui, x as f64, y as f64);
    let selected = gui.take_resource_pack_ids();
    assert_eq!(
        selected,
        Some(vec![
            "vanilla".to_string(),
            "file/jvm-probe-pack".to_string()
        ]),
        "Done should commit the selection (bottom first)"
    );
    let unimpl = h.unimplemented().unwrap();
    assert!(
        unimpl.is_empty(),
        "Unimplemented log for PackSelectionScreen: {unimpl:#?}"
    );
}

fn escape(gui: &haste_jvm::GuiPipeline) {
    gui.key(
        haste_jvm::keys::ESCAPE,
        haste_jvm::keys::keycode(haste_jvm::keys::ESCAPE),
        0,
        true,
    );
    gui.key(
        haste_jvm::keys::ESCAPE,
        haste_jvm::keys::keycode(haste_jvm::keys::ESCAPE),
        0,
        false,
    );
}

/// The sign editor: `block_entity_data` + `open_sign_editor` open vanilla's `SignEditScreen`,
/// typing fills lines (Down moves to the next), and closing sends `sign_update` with the text.
/// Mirrors scripts/checks/edit-sign.txt without a server (the packets are the same wire bytes).
#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM, data generator reports)"]
fn sign_editor_screen() {
    use haste_jvm::keys;
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    let gui = h.gui();
    reset(gui);
    h.clear_unimplemented().unwrap();
    gui.with_state(|s| {
        s.in_world = true;
        s.vitals.health = 20.0;
    });
    // a sign block entity exists (block_entity_data), and the server opens its editor
    let pos = haste_core::BlockPos::new(1, 65, 2);
    gui.packet(&block_entity_data(pos, block_entity_id("minecraft:sign")))
        .unwrap();
    gui.packet(&open_sign_editor(pos)).unwrap();
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.inventory.SignEditScreen"),
        "{:?}",
        gui.take_last_error()
    );
    // the editor draws its title and the Done button
    let f = render(&h, (0.0, 0.0));
    assert!(
        texts(&f).contains(&"Edit Sign Message".to_string()),
        "title: {:?}",
        texts(&f)
    );
    // typing fills line 0; Down moves to line 1 and typing fills that
    type_text(&h, "first line");
    assert!(
        texts(&render(&h, (0.0, 0.0))).contains(&"first line".to_string()),
        "line 0 typed"
    );
    press(&h, keys::DOWN);
    type_text(&h, "second line");
    let t = texts(&render(&h, (0.0, 0.0)));
    assert!(
        t.contains(&"first line".to_string()) && t.contains(&"second line".to_string()),
        "both lines: {t:?}"
    );
    // closing (Escape is Done) sends sign_update with the four lines and the front slot
    press(&h, keys::ESCAPE);
    assert!(!gui.is_open(), "{:?}", gui.take_last_error());
    let out = outgoing(&h);
    let sign_update = serverbound_id("minecraft:sign_update");
    let (_, body) = out
        .iter()
        .find(|(id, _)| *id == sign_update)
        .expect("sign_update sent");
    let mut i = 0;
    let got_pos = i64::from_be_bytes(body[i..i + 8].try_into().unwrap());
    i += 8;
    assert_eq!(haste_core::BlockPos::from_packed(got_pos), pos, "sign pos");
    let line = |i: &mut usize| {
        let len = var_int(body, i) as usize;
        let s = String::from_utf8(body[*i..*i + len].to_vec()).unwrap();
        *i += len;
        s
    };
    assert_eq!(line(&mut i), "first line");
    assert_eq!(line(&mut i), "second line");
    assert_eq!(line(&mut i), "");
    assert_eq!(line(&mut i), "");
    assert_eq!(var_int(body, &mut i), 1, "front slot");
    let unimpl = h.unimplemented().unwrap();
    assert!(
        unimpl.is_empty(),
        "Unimplemented log for the sign editor: {unimpl:#?}"
    );
    gui.with_state(|s| s.in_world = false);
}

/// The book editor: `openItemGui` (a use of the held writable book) opens vanilla's
/// BookEditScreen, typing fills the page, "Sign" opens BookSignScreen, the title is typed and
/// "Sign and Close" sends the edit_book packet (slot, pages, title). Mirrors
/// scripts/checks/edit-book.txt without a server.
#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM, data generator reports)"]
fn book_editor_screen() {
    let _g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    let h = host();
    let gui = h.gui();
    reset(gui);
    h.clear_unimplemented().unwrap();
    gui.with_state(|s| {
        s.in_world = true;
        s.selected_slot = 0;
    });
    // the local player as the login would have introduced it (the book's sign screen names
    // the author from the profile)
    gui.set_player(1, "SignAuthor", 0x0102_0304_0506_0708_090a_0b0c_0d0e)
        .unwrap();
    gui.set_inventory_slot(0, &stack("minecraft:writable_book", 1))
        .unwrap();
    // the use of the held writable book opens the editor (vanilla WritableBookItem.use)
    gui.open_item_gui(0).unwrap();
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.inventory.BookEditScreen"),
        "{:?}",
        gui.take_last_error()
    );
    // the editor draws the page (page indicator "Page 1 of 1") and the buttons
    type_text(&h, "Page one text");
    let t = texts(&render(&h, (0.0, 0.0)));
    assert!(
        t.contains(&"Page one text".to_string()),
        "typed page: {t:?}"
    );
    // "Sign" opens the sign screen (button bottom-left of the book gui)
    let f = render(&h, (0.0, 0.0));
    let sign = f
        .commands
        .iter()
        .find_map(|c| match c {
            haste_core::gui::GuiCommand::Text { text, x, y, .. } if text == "Sign" => {
                Some((*x as f64, *y as f64))
            }
            _ => None,
        })
        .expect("the Sign button's label");
    click(gui, sign.0, sign.1);
    gui.render(320, 240, 2, sign, 0.5).unwrap();
    assert_eq!(
        gui.screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.inventory.BookSignScreen"),
        "{:?}",
        gui.take_last_error()
    );
    // the title is typed into the title box
    type_text(&h, "MyBook");
    let t = texts(&render(&h, (0.0, 0.0)));
    assert!(t.contains(&"MyBook".to_string()), "typed title: {t:?}");
    // "Sign and Close" sends the edit_book packet with slot, the page and the title
    let f = render(&h, (0.0, 0.0));
    let finalize = f
        .commands
        .iter()
        .find_map(|c| match c {
            haste_core::gui::GuiCommand::Text { text, x, y, .. } if text == "Sign and Close" => {
                Some((*x as f64, *y as f64))
            }
            _ => None,
        })
        .expect("the Sign and Close button's label");
    click(gui, finalize.0, finalize.1);
    gui.render(320, 240, 2, finalize, 0.5).unwrap();
    assert!(!gui.is_open(), "{:?}", gui.take_last_error());
    let out = outgoing(&h);
    let edit_book = serverbound_id("minecraft:edit_book");
    let (_, body) = out
        .iter()
        .find(|(id, _)| *id == edit_book)
        .expect("edit_book sent");
    let mut i = 0;
    assert_eq!(
        var_int(body, &mut i),
        0,
        "slot 0 (the selected hotbar slot)"
    );
    let pages = var_int(body, &mut i);
    assert_eq!(pages, 1, "one page");
    let len = var_int(body, &mut i) as usize;
    let page = String::from_utf8(body[i..i + len].to_vec()).unwrap();
    i += len;
    assert_eq!(page, "Page one text");
    // the title is present (an Optional: a boolean byte, then the string)
    assert_eq!(body[i], 1, "the title is present");
    i += 1;
    let tlen = var_int(body, &mut i) as usize;
    let title = String::from_utf8(body[i..i + tlen].to_vec()).unwrap();
    assert_eq!(title, "MyBook");
    let unimpl = h.unimplemented().unwrap();
    assert!(
        unimpl.is_empty(),
        "Unimplemented log for the book editor: {unimpl:#?}"
    );
    gui.with_state(|s| s.in_world = false);
}
