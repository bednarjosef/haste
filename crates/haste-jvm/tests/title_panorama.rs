//! The title panorama through the GUI bridge. Needs the cache (JDK, client jar, libraries, ASM)
//! and is ignored by default: `cargo test -p haste-jvm --test title_panorama -- --ignored`.
use std::path::PathBuf;
use std::time::Duration;

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

#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM)"]
fn title_screen_extracts_a_spinning_panorama() {
    let h =
        JvmHost::start(JvmConfig::from_cache(&cache()).expect("cache layout")).expect("JVM start");
    h.gui()
        .load_jar_registries()
        .expect("registries from the jar");
    let gui = h.gui();
    gui.open_screen(ScreenKind::Title)
        .expect("open the title screen");
    let render = || gui.render(320, 240, 2, (0.0, 0.0), 0.0).expect("render");

    // TitleScreen.extractRenderState -> extractPanorama: the frame carries the panorama and
    // the overlay blit over the whole screen (Panorama.extractRenderState).
    let f = render();
    let first = gui
        .panorama()
        .expect("the title screen extracts the panorama");
    let overlay = f.commands.iter().find_map(|c| match c {
        GuiCommand::Texture {
            texture,
            w,
            h,
            tex_w,
            tex_h,
            ..
        } if texture.path() == "textures/gui/title/background/panorama_overlay.png" => {
            Some((*w, *h, *tex_w, *tex_h))
        }
        _ => None,
    });
    assert_eq!(overlay, Some((320, 240, 16, 128)), "{:#?}", f.commands);
    // The title screen draws no blurred background of its own (TitleScreen.extractBackground
    // is empty).
    assert_eq!(f.blur_after, -1);

    // The spin follows real time: a tenth of a degree per tick at the default speed, so
    // 300 ms turn it by about 0.6 degrees (the state is the negated spin).
    std::thread::sleep(Duration::from_millis(300));
    render();
    let second = gui.panorama().unwrap();
    let turned = first - second;
    assert!(
        (0.4..1.0).contains(&turned),
        "spin {first} -> {second}: turned {turned} degrees in 300 ms"
    );

    // No screen: no panorama.
    gui.close_screen();
    render();
    assert_eq!(gui.panorama(), None);
}
