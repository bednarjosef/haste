//! Dev evidence helper for the cloud-option branch: proves the `renderClouds` option
//! end to end. Three offscreen screenshots at the shared test-server spot, one per cloud
//! mode, each with its own throwaway `$XDG_CONFIG_HOME` config (the user's real config
//! is never touched); one `--script` run that cycles the real vanilla `Clouds` button in
//! the video settings screen and shows the change applies live (the JVM's
//! `OptionBridge` -> `Native.optionsChanged` -> `apply_option_changes` chain); and the
//! offscreen bench at render distance 12 with clouds on and off.
//!
//! Uses the shared test server on 127.0.0.1:25691 (never starts one) and the built
//! `target/debug/haste` (the bench uses `target/release/haste`). Ignored unless run by
//! name:
//!   cargo test -q -p haste-client --test clouds_mode_shot -- --ignored --nocapture

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crate inside the repo")
        .to_path_buf()
}

/// A config directory holding a fresh config whose only non-default option is the cloud
/// mode, so a spawned client runs with exactly that `renderClouds` value. The MSA fields
/// stay absent: the run joins the offline test server.
fn write_config(root: &std::path::Path, clouds: &str, render_distance: u8) -> std::path::PathBuf {
    let dir = root.join(format!("target/clouds-option/xdg-{clouds}"));
    let haste = dir.join("haste");
    std::fs::create_dir_all(&haste).expect("config dir");
    std::fs::write(
        haste.join("config.toml"),
        format!(
            "username = \"cloudopt\"\nrender_distance = {render_distance}\ngui_scale = 2\n\
             fov = 70.0\n\n[options]\nrender_distance = {render_distance}\nclouds = \"{clouds}\"\n"
        ),
    )
    .expect("config.toml");
    dir
}

/// Holds the shared-spot tp loop for one client run (stops on drop), like the
/// resource-pack helper: the camera stays at the snowy plain while the client joins.
struct TpLoop {
    stop: Arc<AtomicBool>,
}

impl TpLoop {
    fn start(root: std::path::PathBuf, username: &str, pitch: &str) -> TpLoop {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let username = username.to_owned();
        let pitch = pitch.to_owned();
        std::thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                let _ = Command::new("bash")
                    .arg(root.join("scripts/vanilla-rcon.sh"))
                    .arg("--port")
                    .arg("25691")
                    .arg(format!("tp {username} 9.5 71 6.5 0 {pitch}"))
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                std::thread::sleep(Duration::from_millis(250));
            }
        });
        TpLoop { stop }
    }
}

impl Drop for TpLoop {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Runs one client command to completion, tailing its log (the client writes stdout and
/// stderr there) and printing every line. Returns the exit status and the lines.
fn run_client(
    root: &std::path::Path,
    bin: &str,
    args: &[&str],
    env: &[(&str, &std::path::Path)],
    log_name: &str,
) -> (std::process::ExitStatus, Vec<String>) {
    let log = root.join(format!("target/clouds-option/{log_name}.log"));
    std::fs::create_dir_all(log.parent().unwrap()).expect("log dir");
    let file = std::fs::File::create(&log).expect("create the client log");
    let mut cmd = Command::new(root.join(bin));
    cmd.args(args)
        .current_dir(root)
        .stdout(Stdio::from(file.try_clone().expect("clone the log")))
        .stderr(Stdio::from(file));
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("start the client");
    let (tx, rx) = mpsc::channel::<String>();
    let tail_log = log.clone();
    std::thread::spawn(move || {
        // Tail the log while the client writes it (the process holds the file open).
        let mut pos = 0u64;
        let deadline = Instant::now() + Duration::from_secs(300);
        while Instant::now() < deadline {
            if let Ok(md) = std::fs::metadata(&tail_log) {
                if md.len() > pos {
                    if let Ok(mut f) = std::fs::File::open(&tail_log) {
                        use std::io::Seek;
                        let _ = f.seek(std::io::SeekFrom::Start(pos));
                        pos = md.len();
                        for line in BufReader::new(f).lines().map_while(Result::ok) {
                            println!("client: {line}");
                            let _ = tx.send(line);
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    });
    let status = child.wait().expect("client exit");
    // Give the tail one beat to read what the exit flushed, then collect.
    std::thread::sleep(Duration::from_millis(600));
    let mut lines = Vec::new();
    while let Ok(line) = rx.try_recv() {
        lines.push(line);
    }
    (status, lines)
}

/// Pixels that differ between two PNGs of the same size.
fn differing_pixels(a: &std::path::Path, b: &std::path::Path) -> usize {
    let a = image::open(a).expect("open the first shot").to_rgba8();
    let b = image::open(b).expect("open the second shot").to_rgba8();
    a.as_raw()
        .iter()
        .zip(b.as_raw().iter())
        .filter(|(x, y)| x != y)
        .count()
}

/// The three cloud modes at the shared screenshot spot, camera tilted up at the cloud
/// layer (positive pitch looks down, so -40 looks at the sky): FANCY and FAST draw the
/// layer, OFF leaves plain sky, and the two drawn modes differ (flat cells against
/// extruded boxes). Each mode runs with its own config, so the evidence covers the
/// config -> renderer path; the `video_settings_cycle_applies_live` test covers the
/// live path.
#[test]
#[ignore = "needs the local test server on 25691 and a built target/debug/haste"]
fn screenshot_each_cloud_mode() {
    let root = repo_root();
    let out = root.join("target/clouds-option");
    std::fs::create_dir_all(&out).expect("output dir");
    for mode in ["true", "fast", "false"] {
        let shot = out.join(format!("clouds-{mode}.png"));
        let _ = std::fs::remove_file(&shot);
        let xdg = write_config(&root, mode, 8);
        let tp = TpLoop::start(root.clone(), "cloudopt", "-40");
        let (status, _) = run_client(
            &root,
            "target/debug/haste",
            &[
                "--server",
                "127.0.0.1:25691",
                "--username",
                "cloudopt",
                "--screenshot",
                shot.to_str().unwrap(),
                "--hud",
                "off",
                "--pos",
                "9.5",
                "71",
                "6.5",
                "--yaw",
                "0",
                "--pitch",
                "-40",
                "--width",
                "960",
                "--height",
                "540",
                "--render-distance",
                "8",
                "--seconds",
                "120",
            ],
            &[("XDG_CONFIG_HOME", &xdg)],
            &format!("shot-{mode}"),
        );
        drop(tp);
        assert!(status.success(), "the {mode} screenshot run failed");
        assert!(shot.exists(), "the {mode} shot was not written");
    }
    let fancy = out.join("clouds-true.png");
    let fast = out.join("clouds-fast.png");
    let off = out.join("clouds-false.png");
    // OFF leaves plain sky: the whole cloud area differs.
    let off_diff = differing_pixels(&fancy, &off);
    println!("fancy vs off: {off_diff} differing pixels");
    assert!(
        off_diff > 5000,
        "turning the clouds off barely changed the frame"
    );
    // FAST draws the flat cells instead of the boxes.
    let fast_diff = differing_pixels(&fancy, &fast);
    println!("fancy vs fast: {fast_diff} differing pixels");
    assert!(
        fast_diff > 1000,
        "the flat cells barely differ from the extruded boxes"
    );
    // And FAST is not OFF: the layer is there, drawn flat.
    let fast_off = differing_pixels(&fast, &off);
    println!("fast vs off: {fast_off} differing pixels");
    assert!(fast_off > 1000, "the fast clouds are missing");
}

/// Cycles the real vanilla `Clouds` button in the video settings screen (`CloudStatus`
/// FANCY -> OFF -> FAST from the FANCY default): the JVM's option bridge must report
/// each change and the running renderer must apply it without a restart. The shots are
/// taken from inside the options screen stack, so the dimmed world shows the sky
/// behind it. The script does not close the screens afterwards: `key Escape` from the
/// video settings screen hits a JVM pipeline gap (`GuiPipeline::key` ->
/// `NoClassDefFoundError: RenderSystem`), and the run ends with the script anyway.
#[test]
#[ignore = "needs the local test server on 25691 and a built target/debug/haste"]
fn video_settings_cycle_applies_live() {
    let root = repo_root();
    let out = root.join("target/clouds-option");
    std::fs::create_dir_all(&out).expect("output dir");
    let script = out.join("cycle.script");
    std::fs::write(
        &script,
        "wait_until settled\n\
         key Escape\n\
         wait_until screen PauseScreen\n\
         wait 5\n\
         click_text \"Options...\" 0 0\n\
         wait_until screen OptionsScreen\n\
         wait 5\n\
         click_text \"Video Settings...\" 0 0\n\
         wait_until screen VideoSettingsScreen\n\
         wait 5\n\
         click_text \"Clouds\" 0 0\n\
         wait 10\n\
         screenshot clouds-live-off\n\
         click_text \"Clouds\" 0 0\n\
         wait 10\n\
         screenshot clouds-live-fast\n\
         wait 5\n",
    )
    .expect("write the script");
    let xdg = write_config(&root, "true", 8);
    // The JVM's own `Options` loads and saves `<game dir>/options.txt` (vanilla
    // `Options.load`/`save`): a fresh game dir, so a previous run's saved value cannot
    // leak in and shift the button's cycle.
    let game_dir = out.join("cycle-gamedir");
    std::fs::create_dir_all(&game_dir).expect("game dir");
    let _ = std::fs::remove_file(game_dir.join("options.txt"));
    let tp = TpLoop::start(root.clone(), "cloudopt", "-40");
    let (status, lines) = run_client(
        &root,
        "target/debug/haste",
        &[
            "--script",
            script.to_str().unwrap(),
            "--server",
            "127.0.0.1:25691",
            "--username",
            "cloudopt",
            "--pos",
            "9.5",
            "71",
            "6.5",
            "--yaw",
            "0",
            "--pitch",
            "-40",
            "--width",
            "960",
            // Tall enough that the video settings list's `Clouds` row (a few sections
            // down qualityOptions) is on screen without scrolling.
            "--height",
            "1080",
            "--render-distance",
            "8",
            "--seconds",
            "180",
        ],
        &[("XDG_CONFIG_HOME", &xdg), ("HASTE_GAME_DIR", &game_dir)],
        "cycle",
    );
    drop(tp);
    assert!(status.success(), "the cloud-cycle script run failed");
    // The bridge reported both changes, as options.txt pairs.
    let saw_off = lines
        .iter()
        .any(|l| l.contains("applied option renderClouds = OFF"));
    let saw_fast = lines
        .iter()
        .any(|l| l.contains("applied option renderClouds = FAST"));
    assert!(saw_off, "the OFF change never reached the renderer");
    assert!(saw_fast, "the FAST change never reached the renderer");
    // And the two states drew different frames (the world behind the options screen).
    let shots = root.join("target/shots");
    let off = shots.join("script-clouds-live-off.png");
    let fast = shots.join("script-clouds-live-fast.png");
    assert!(
        off.exists() && fast.exists(),
        "the script shots are missing"
    );
    let diff = differing_pixels(&off, &fast);
    println!("live off vs fast: {diff} differing pixels");
    assert!(
        diff > 1000,
        "the live cloud change did not change the frame"
    );
}

/// The offscreen bench at render distance 12 with clouds on (FANCY, the default) and
/// off: how much frame time the cloud pass costs. Uses `target/release/haste`.
#[test]
#[ignore = "needs the local test server on 25691 and a built target/release/haste"]
fn bench_rd12_clouds_on_vs_off() {
    let root = repo_root();
    let bin = root.join("target/release/haste");
    assert!(
        bin.exists(),
        "build it first: cargo build --release -p haste-client"
    );
    for mode in ["true", "false"] {
        let json = root.join(format!("target/clouds-option/bench-{mode}.json"));
        let _ = std::fs::remove_file(&json);
        let xdg = write_config(&root, mode, 12);
        let (status, _) = run_client(
            &root,
            "target/release/haste",
            &[
                "--bench",
                "20",
                "--bench-out",
                json.to_str().unwrap(),
                "--server",
                "127.0.0.1:25691",
                "--username",
                "cloudbench",
                "--render-distance",
                "12",
                "--width",
                "1920",
                "--height",
                "1080",
                "--seconds",
                "150",
            ],
            &[("XDG_CONFIG_HOME", &xdg)],
            &format!("bench-{mode}"),
        );
        assert!(status.success(), "the {mode} bench run failed");
        assert!(json.exists(), "the {mode} bench summary is missing");
        // The summary lines are the evidence; print them from the log too.
        let text =
            std::fs::read_to_string(root.join(format!("target/clouds-option/bench-{mode}.log")))
                .expect("the bench log");
        for line in text.lines() {
            if line.contains("bench summary")
                || line.contains("avg")
                || line.contains("worst")
                || line.contains("frame")
            {
                println!("{line}");
            }
        }
    }
}
