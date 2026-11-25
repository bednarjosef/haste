//! The Singleplayer click lag: the world-selection screen tree (105 classes on 26.3) loads on
//! the click's thread, costing the player the first click (~half a second on a cold page cache).
//! `JvmHost::warm_screen_classes` loads them on a background thread once the title screen is up;
//! this test proves the warm-up covers the whole click path. It needs the cache (JDK, client
//! jar, libraries, ASM) and is ignored by default:
//! `cargo test -p haste-jvm -- --ignored --test click_lag --test-threads=1`.
use std::path::PathBuf;
use std::time::{Duration, Instant};

use haste_jvm::{JvmConfig, JvmHost, ScreenKind};
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
    let h =
        JvmHost::start(JvmConfig::from_cache(&cache()).expect("cache layout")).expect("JVM start");
    h.gui()
        .load_jar_registries()
        .expect("registries from the jar");
    h
}

/// The world-selection tree the Singleplayer click needs is warmed ahead of the click:
/// after `warm_screen_classes` every entry of `WARM_CLASSES` (and the nested classes the
/// click instantiates, like the world-list entries) is already in the class log, and the
/// click itself — open SelectWorldScreen, render until the world's entry lists — loads
/// nothing. A class the click still loads would stall the click's thread, exactly the bug
/// this guards against; the failure names it so it can be added to `WARM_CLASSES`.
#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM)"]
fn warm_screen_classes_covers_the_singleplayer_click() {
    let h = host();
    let gui = h.gui();
    gui.reset_for_test();
    // one world in the singleplayer dir (the player's case: the list stays open, no auto-jump
    // to the create screen), so the click path is open + render until the entry lists
    let world = h.config().singleplayer_dir.join("Haste Click Lag World");
    std::fs::create_dir_all(&world).unwrap();
    std::fs::write(
        world.join("level.dat"),
        level_dat("Haste Click Lag World", 1),
    )
    .unwrap();

    // the title screen, like the player's menu; then the warm-up on its background thread
    h.gui().open_screen(ScreenKind::Title).unwrap();
    h.gui().render(320, 240, 2, (0.0, 0.0), 0.0).unwrap();
    let cold = {
        let log = h.class_log().expect("class log");
        assert!(
            !log.classes
                .contains_key("net.minecraft.client.gui.screens.worldselection.SelectWorldScreen"),
            "the title screen already loaded the world-selection tree; the test needs a cold JVM \
             (run this test alone: -- --ignored warm_screen_classes)"
        );
        log
    };
    h.warm_screen_classes();
    // the warm-up loads on its own thread; wait until the class log stops growing
    let mut last = cold.classes.len();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        std::thread::sleep(Duration::from_millis(50));
        let now = h.class_log().expect("class log").classes.len();
        if now == last || Instant::now() > deadline {
            break;
        }
        last = now;
    }
    let warm = h.class_log().expect("class log");
    for name in haste_jvm::WARM_CLASSES {
        assert!(
            warm.classes.contains_key(*name),
            "the warm-up did not load {name}"
        );
    }
    // the nested classes the click instantiates (loaded through getDeclaredClasses)
    for nested in [
        "net.minecraft.client.gui.screens.worldselection.WorldSelectionList$WorldListEntry",
        "net.minecraft.client.gui.screens.worldselection.WorldSelectionList$LoadingHeader",
        "net.minecraft.client.gui.components.AbstractSelectionList$Entry",
    ] {
        assert!(
            warm.classes.contains_key(nested),
            "the warm-up did not load {nested}"
        );
    }

    // the click: nothing it touches may load a class (each load would stall the click's thread)
    let before = h.class_log().expect("class log");
    let t = Instant::now();
    gui.open_screen(ScreenKind::SelectWorld).unwrap();
    let open_t = t.elapsed();
    let t = Instant::now();
    let mut listed = None;
    for _ in 0..100 {
        let f = gui.render(320, 240, 2, (0.0, 0.0), 0.0).unwrap();
        if texts(&f).contains(&"Haste Click Lag World") {
            listed = Some(t.elapsed());
            break;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    let listed = listed.expect("the world entry lists");
    let after = h.class_log().expect("class log");
    let loaded: Vec<String> = after
        .classes
        .keys()
        .filter(|k| !before.classes.contains_key(*k))
        .cloned()
        .collect();
    eprintln!("open(SelectWorld) {open_t:?}, world listed after {listed:?}");
    assert!(
        loaded.is_empty(),
        "the click still loaded classes (add them to WARM_CLASSES): {loaded:#?}"
    );
    gui.close_screen();
    std::fs::remove_dir_all(&world).unwrap();
}

/// Every text the frame draws (in draw order).
fn texts(f: &haste_core::gui::GuiFrame) -> Vec<&str> {
    use haste_core::gui::GuiCommand;
    f.commands
        .iter()
        .filter_map(|c| match c {
            GuiCommand::Text { text, .. } | GuiCommand::StyledText { text, .. } => {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect()
}

/// A minimal world `level.dat` (vanilla's format: gzip'd NBT) with the fields the world list's
/// summary reads: name, game mode, the version block and last-played time. Same shape as
/// `jvm.rs`'s helper (a separate test binary cannot share it).
fn level_dat(name: &str, game_type: i32) -> Vec<u8> {
    fn tag(bytes: &mut Vec<u8>, id: u8, name: &str) {
        bytes.push(id);
        bytes.extend_from_slice(&(name.len() as u16).to_be_bytes());
        bytes.extend_from_slice(name.as_bytes());
    }
    fn string(bytes: &mut Vec<u8>, name: &str, value: &str) {
        tag(bytes, 0x08, name);
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
    let h =
        JvmHost::start(JvmConfig::from_cache(&cache()).expect("cache layout")).expect("JVM start");
    let version = world_data_version(&h);
    let mut b = Vec::new();
    tag(&mut b, 0x0a, ""); // root compound
    tag(&mut b, 0x0a, "Data");
    int(&mut b, "version", 19133);
    int(&mut b, "DataVersion", version);
    long(&mut b, "LastPlayed", 1_600_000_000_000);
    string(&mut b, "LevelName", name);
    int(&mut b, "GameType", game_type);
    int(&mut b, "allowCommands", 0);
    tag(&mut b, 0x0a, "difficulty_settings");
    string(&mut b, "difficulty", "normal");
    tag(&mut b, 0x01, "hardcore");
    b.push(0);
    tag(&mut b, 0x01, "locked");
    b.push(0);
    b.push(0x00); // end of difficulty_settings
    tag(&mut b, 0x0a, "Version");
    string(&mut b, "Name", "26.3");
    int(&mut b, "Id", version);
    string(&mut b, "Series", "main");
    tag(&mut b, 0x01, "Snapshot");
    b.push(0);
    b.push(0);
    b.push(0x00); // end of Version
    b.push(0x00); // end of Data
    b.push(0x00); // end of the root
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut gz, &b).unwrap();
    gz.finish().unwrap()
}

/// The current world data version as Java sees it (`SharedConstants.getCurrentVersion()`, the
/// version.json of the player's jar): a world's `level.dat` carries it so the world list does
/// not mark a test world as needing a file fix. Same as `jvm.rs`'s helper.
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
