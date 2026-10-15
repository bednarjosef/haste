//! Dev evidence helper for the resource-pack branch (Batch 2): runs the offscreen client
//! with a `--script` that opens Options > Resource Packs (the real vanilla
//! `PackSelectionScreen` over the pack repository `ClientImpl` builds), selects a
//! generated pack that turns snow red, presses Done, and takes a screenshot — without
//! restarting the client. The reload path is the live one (vanilla
//! `Minecraft.reloadResourcePacks`): the JVM's `Options.save` writes the selection to
//! options.txt, Rust re-opens its asset stack and rebuilds the renderer.
//!
//! Uses the shared test server on 127.0.0.1:25691 (never starts one), `target/debug/haste`
//! and an isolated `$HASTE_GAME_DIR` under target/. Ignored unless run by name.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crate inside the repo")
        .to_path_buf()
}

/// The game dir for the run: `target/redsnow-game`, with the red snow pack in
/// `resourcepacks/redsnow` (pack format 97, 26.3's resource pack format) and no
/// options.txt (nothing selected at start).
fn write_game_dir(root: &std::path::Path) -> std::path::PathBuf {
    let game_dir = root.join("target/redsnow-game");
    let pack = game_dir.join("resourcepacks/redsnow");
    std::fs::create_dir_all(pack.join("assets/minecraft/textures/block")).expect("pack dirs");
    std::fs::write(
        pack.join("pack.mcmeta"),
        // 26.3's format field pair (a bare pack_format above 64 is rejected by vanilla's
        // PackFormat.validate: formats newer than 64 must declare min_format/max_format).
        r#"{"pack":{"min_format":97,"max_format":97,"description":"red snow test pack"}}"#,
    )
    .expect("pack.mcmeta");
    let img = image::RgbaImage::from_pixel(16, 16, image::Rgba([220, 40, 40, 255]));
    img.save(pack.join("assets/minecraft/textures/block/snow.png"))
        .expect("snow.png");
    // Start from no selection: the JVM's Options would otherwise keep an old one.
    let _ = std::fs::remove_file(game_dir.join("options.txt"));
    game_dir
}

#[test]
#[ignore = "needs the local test server on 25691 and a built target/debug/haste"]
fn script_selects_pack_and_reloads_without_restart() {
    let root = repo_root();
    let game_dir = write_game_dir(&root);
    // Reproducible pictures like the reference server's setup: noon light, so the
    // red-snow check does not depend on the world's time of day (night dims the pack's
    // red below the hue threshold).
    let _ = Command::new("bash")
        .arg(root.join("scripts/vanilla-rcon.sh"))
        .arg("--port")
        .arg("25691")
        .arg("time set 6000")
        .output();

    let script = root.join("target/redsnow-select.script");
    std::fs::write(
        &script,
        // The click on the pack entry is at the row's 32x32 icon (left of the title,
        // TransferableSelectionList.PackEntry.mouseClicked selects only there): the title
        // renders at the icon's right + 2, one pixel lower.
        "wait_until settled\n\
         key Escape\n\
         wait_until screen PauseScreen\n\
         wait 5\n\
         click_text \"Options...\" 0 0\n\
         wait_until screen OptionsScreen\n\
         wait 5\n\
         click_text \"Resource Packs...\" 0 0\n\
         wait_until screen PackSelectionScreen\n\
         wait 5\n\
         click_text \"redsnow\" -18 15\n\
         wait 5\n\
         click_text \"Done\" 0 0\n\
         wait_until screen OptionsScreen\n\
         key Escape\n\
         wait_until screen PauseScreen\n\
         key Escape\n\
         wait_until screen_closed\n\
         wait_until reloaded\n\
         wait_until settled\n\
         screenshot redsnow-after\n\
         wait 5\n",
    )
    .expect("write the script");

    let haste_log = root.join("target/redsnow-select.log");
    let _ = std::fs::remove_file(&haste_log);
    let log_file = std::fs::File::create(&haste_log).expect("create the log");
    let mut haste = Command::new(root.join("target/debug/haste"))
        .arg("--script")
        .arg(&script)
        .arg("--server")
        .arg("127.0.0.1:25691")
        .arg("--username")
        .arg("worker-redsnow")
        .arg("--pos")
        .arg("9.5")
        .arg("71")
        .arg("6.5")
        .arg("--yaw")
        .arg("0")
        .arg("--pitch")
        .arg("10")
        .arg("--width")
        .arg("960")
        .arg("--height")
        .arg("540")
        .arg("--render-distance")
        .arg("8")
        .arg("--seconds")
        .arg("180")
        .current_dir(&root)
        .env("HASTE_GAME_DIR", &game_dir)
        .stdout(Stdio::from(log_file.try_clone().expect("clone log")))
        .stderr(Stdio::from(log_file))
        .spawn()
        .expect("start the offscreen client");
    // Hold the camera on the snowy plain while the client joins and settles (a tp to the
    // same spot does not reload anything once there), like scripts/compare-vanilla.sh.
    // Stops when the client exits.
    let tp_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let tp_root = root.clone();
    let tp_stop_flag = tp_stop.clone();
    let _tp = std::thread::spawn(move || {
        while !tp_stop_flag.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = Command::new("bash")
                .arg(tp_root.join("scripts/vanilla-rcon.sh"))
                .arg("--port")
                .arg("25691")
                .arg("tp worker-redsnow 9.5 71 6.5 0 10")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            std::thread::sleep(Duration::from_millis(250));
        }
    });

    let (lines_tx, lines) = mpsc::channel::<String>();
    let log = root.join("target/redsnow-select.log");
    std::thread::spawn(move || {
        // Tail the log while the client writes it (the process holds the file open).
        let mut pos = 0u64;
        let deadline = Instant::now() + Duration::from_secs(200);
        while Instant::now() < deadline {
            if let Ok(md) = std::fs::metadata(&log) {
                if md.len() > pos {
                    if let Ok(mut f) = std::fs::File::open(&log) {
                        use std::io::Seek;
                        let _ = f.seek(std::io::SeekFrom::Start(pos));
                        pos = md.len();
                        for line in BufReader::new(f).lines().map_while(Result::ok) {
                            let _ = lines_tx.send(line);
                        }
                    }
                }
            }
            if haste_done(&log) {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    });

    let status = haste.wait().expect("client exit");
    tp_stop.store(true, std::sync::atomic::Ordering::Relaxed);
    println!("client exit: {status:?}");
    let mut saw_reload = false;
    let mut saw_save = false;
    let mut saw_shot = false;
    while let Ok(line) = lines.try_recv() {
        if line.contains("resource pack selection changed") {
            saw_reload = true;
        }
        if line.contains("rebuilding renderer with resource packs") {
            saw_save = true;
        }
        if line.contains("screenshot ") {
            saw_shot = true;
        }
    }
    let full_log =
        std::fs::read_to_string(root.join("target/redsnow-select.log")).unwrap_or_default();
    for line in full_log.lines() {
        if line.contains("script text[")
            || line.contains("PackSelection")
            || line.contains("resource pack")
        {
            println!("log: {line}");
        }
    }
    assert!(
        saw_reload || full_log.contains("resource pack selection changed"),
        "the pack screen's Done should commit a selection (see target/redsnow-select.log)"
    );
    assert!(
        saw_save || full_log.contains("rebuilding renderer with resource packs"),
        "the client should rebuild the renderer live (see target/redsnow-select.log)"
    );
    assert!(
        saw_shot || full_log.contains("screenshot "),
        "a screenshot should be saved"
    );

    // The shot shows the snowy plain with the pack applied: its ground must be red.
    let shot = root.join("target/shots/script-redsnow-after.png");
    assert!(
        shot.is_file(),
        "the screenshot should exist at {}",
        shot.display()
    );
    let red = red_pixels(&shot);
    println!("snow check: {red} red lower-half pixels after the live reload");
    assert!(
        red > 10_000,
        "the reloaded atlas should draw the pack's red snow ({red} red pixels)"
    );

    // And options.txt now carries the selection (vanilla Options.save on Done): the vanilla
    // built-in entry and the pack, bottom first.
    let options_txt = std::fs::read_to_string(game_dir.join("options.txt")).unwrap_or_default();
    assert!(
        options_txt.contains("resourcePacks:[\"vanilla\",\"file/redsnow\"]"),
        "options.txt should list the selected pack: {options_txt}"
    );
}

/// Whether the client process has finished (its log stops growing and mentions leaving).
fn haste_done(_log: &std::path::Path) -> bool {
    false // the loop ends by deadline; the reader is best-effort
}

#[test]
#[ignore = "needs the local test server on 25691 (dev screenshot helper)"]
fn startup_reads_selection_from_options_txt() {
    // The game dir the script test leaves behind has redsnow selected; a plain
    // `--screenshot` run with it must apply the pack at startup (options.txt is the
    // source of truth, like vanilla's loadSelectedResourcePacks).
    let root = repo_root();
    let game_dir = write_game_dir(&root);
    let _ = Command::new("bash")
        .arg(root.join("scripts/vanilla-rcon.sh"))
        .arg("--port")
        .arg("25691")
        .arg("time set 6000")
        .output();
    // select the pack as the pack screen would have (vanilla's Options.save format)
    std::fs::write(
        game_dir.join("options.txt"),
        "resourcePacks:[\"vanilla\",\"file/redsnow\"]\n",
    )
    .expect("options.txt");
    let shot = root.join("target/redsnow-startup.png");
    let mut haste = Command::new(root.join("target/debug/haste"))
        .arg("--server")
        .arg("127.0.0.1:25691")
        .arg("--username")
        .arg("worker-redsnow")
        .arg("--screenshot")
        .arg(&shot)
        .arg("--pos")
        .arg("9.5")
        .arg("71")
        .arg("6.5")
        .arg("--yaw")
        .arg("0")
        .arg("--pitch")
        .arg("10")
        .arg("--width")
        .arg("960")
        .arg("--height")
        .arg("540")
        .arg("--render-distance")
        .arg("8")
        .arg("--seconds")
        .arg("60")
        .current_dir(&root)
        .env("HASTE_GAME_DIR", &game_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start the screenshot client");
    let status = haste.wait().expect("client exit");
    println!("client exit: {status:?}");
    assert!(shot.is_file(), "screenshot should exist");
    let red = red_pixels(&shot);
    println!("snow check: {red} red lower-half pixels at startup");
    assert!(
        red > 10_000,
        "the pack should apply at startup ({red} red pixels)"
    );
}

/// Count of strongly red-dominant pixels in the lower half of an image.
fn red_pixels(path: &std::path::Path) -> usize {
    let img = image::open(path).expect("open the screenshot");
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut red = 0usize;
    for y in (h / 2)..h {
        for x in 0..w {
            let p = rgba.get_pixel(x, y);
            if p[0] as i32 - p[1] as i32 > 60 && p[0] as i32 - p[2] as i32 > 60 {
                red += 1;
            }
        }
    }
    red
}
