//! The server resource pack prompt is vanilla's own `ConfirmScreen` with
//! `ClientCommonPacketListenerImpl.PackConfirmScreen`'s wording, opened over
//! the open screen through the bridge; its buttons answer to Rust and return
//! to that screen. Ignored unless the cache (JDK, client jar) exists.

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

fn texts(f: &haste_core::gui::GuiFrame) -> Vec<String> {
    use haste_core::gui::GuiCommand;
    f.commands
        .iter()
        .filter_map(|c| match c {
            GuiCommand::Text { text, .. } | GuiCommand::StyledText { text, .. } => {
                Some(text.clone())
            }
            _ => None,
        })
        .collect()
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

fn render(h: &JvmHost) -> haste_core::gui::GuiFrame {
    h.gui()
        .render(320, 240, 2, (0.0, 0.0), 0.0)
        .unwrap_or_else(|e| panic!("render: {e}\n{:#?}", h.unimplemented().unwrap()))
}

fn click_text(h: &JvmHost, label: &str) {
    let f = render(h);
    let (x, y) = text_pos(&f, label).unwrap_or_else(|| panic!("no {label:?} in {:?}", texts(&f)));
    let (x, y) = (f64::from(x) + 2.0, f64::from(y) + 2.0);
    h.gui()
        .mouse(x, y, Some((haste_jvm::keys::MOUSE_LEFT, true)));
    h.gui()
        .mouse(x, y, Some((haste_jvm::keys::MOUSE_LEFT, false)));
}

#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM)"]
fn pack_prompt_is_vanillas_confirm_screen() {
    let h = JvmHost::start(JvmConfig::from_cache(&cache()).expect("cache layout")).expect("JVM");
    let gui = h.gui();
    gui.load_jar_registries().expect("registries from the jar");
    gui.reset_for_test();
    gui.open_screen(haste_jvm::ScreenKind::Title)
        .expect("title");
    let parent = gui.screen_class();

    // Optional, with a server prompt: vanilla's "recommends" wording, the prompt
    // under "Message from server:", Yes / No.
    gui.show_pack_prompt(false, Some(r#"{"text":"Our textures, please"}"#));
    let f = render(&h);
    let t = texts(&f);
    eprintln!("optional prompt: {t:?}");
    for want in [
        "This server recommends the use of a custom resource pack.",
        "Message from server:",
        "Our textures, please",
        "Yes",
        "No",
    ] {
        assert!(t.iter().any(|s| s == want), "{want:?} missing from {t:?}");
    }
    assert!(gui
        .screen_class()
        .is_some_and(|c| c.contains("PackPromptScreen")));
    assert_eq!(gui.take_pack_prompt_answer(), None, "no answer yet");
    click_text(&h, "Yes");
    assert_eq!(gui.take_pack_prompt_answer(), Some(true));
    assert_eq!(gui.take_pack_prompt_answer(), None, "taken once");
    assert_eq!(
        gui.screen_class(),
        parent,
        "back to the screen it opened over"
    );

    // A second push joins an open prompt: the screen is rebuilt in the newest
    // push's (required) wording and still returns to the first parent.
    gui.show_pack_prompt(false, None);
    gui.show_pack_prompt(true, None);
    let t = texts(&render(&h));
    eprintln!("required prompt: {t:?}");
    for want in [
        "This server requires the use of a custom resource pack.",
        "Proceed",
        "Disconnect",
    ] {
        assert!(t.iter().any(|s| s == want), "{want:?} missing from {t:?}");
    }
    assert!(
        !t.iter().any(|s| s == "Message from server:"),
        "no prompt text"
    );
    click_text(&h, "Disconnect");
    assert_eq!(gui.take_pack_prompt_answer(), Some(false));
    assert_eq!(gui.screen_class(), parent);
    let unimpl = h.unimplemented().unwrap();
    assert!(unimpl.is_empty(), "Unimplemented: {unimpl:#?}");
}
