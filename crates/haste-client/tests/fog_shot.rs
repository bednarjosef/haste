//! Dev-only helper (ignored test, no production paths): starts a private test server, builds a
//! water pool and a Nether platform through the server console (the same command source as
//! RCON), teleports the test camera into each and takes one offscreen screenshot per phase with
//! haste. It prints the client's `fog environment ...` line for each phase (the fog environment,
//! sRGB colour and distances of that frame) so the values can be checked without opening the
//! images.
//!
//! usage:
//!   cargo test -q -p haste-client --test fog_shot -- --ignored --nocapture
//!
//! env: FOG_SHOT_SERVER (server copy name, default fog1), FOG_SHOT_WAIT (client wait bound,
//!       default 90).

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Stops the server even when the test panics.
struct ServerGuard(Child);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn repo_root() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crate inside the repo")
        .to_path_buf()
}

/// One `--screenshot` run against the already-running server. `tp` is the console command that
/// teleports the player to `pos` once it has joined (feet coordinates). Returns the client's
/// `fog environment ...` line.
#[allow(clippy::too_many_arguments)]
fn run_shot(
    root: &std::path::Path,
    port: u16,
    username: &str,
    shot: &std::path::Path,
    pos: [&str; 3],
    tp: &str,
    wait_secs: u64,
    console: &mut ChildStdin,
    server_lines: &mpsc::Receiver<String>,
) -> String {
    println!("== taking the {username} screenshot");
    let mut client = Command::new(root.join("target/debug/haste"))
        .args([
            "--server",
            &format!("127.0.0.1:{port}"),
            "--username",
            username,
            "--screenshot",
            shot.to_str().unwrap(),
            "--seconds",
            &wait_secs.to_string(),
            "--pos",
            pos[0],
            pos[1],
            pos[2],
            "--yaw",
            "0",
            "--pitch",
            "0",
        ])
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the screenshot client");
    let (fog_tx, fog_rx) = mpsc::channel();
    if let Some(out) = client.stdout.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if line.starts_with("fog environment") {
                    let _ = fog_tx.send(line.clone());
                }
                println!("client stdout: {line}");
            }
        });
    }
    if let Some(err) = client.stderr.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                println!("client stderr: {line}");
            }
        });
    }

    // Teleport as soon as the player has joined (the same source RCON uses).
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match server_lines.recv_timeout(Duration::from_secs(5)) {
            Ok(line) if line.contains(&format!("{username} joined the game")) => {
                writeln!(console, "{tp}").expect("tp");
                console.flush().ok();
                break;
            }
            Ok(_) => {}
            Err(_) if Instant::now() > deadline => break,
            Err(_) => {}
        }
    }

    let status = client.wait().expect("client exit");
    println!("client exit: {status:?}");
    fog_rx.try_recv().unwrap_or_default()
}

fn main_console(server: &mut Child) -> ChildStdin {
    server.stdin.take().expect("server console")
}

#[test]
#[ignore = "needs the local test server cache and a Vulkan driver (dev screenshot helper)"]
fn fog_underwater_and_nether() {
    let port: u16 = 25_740;
    let name = std::env::var("FOG_SHOT_SERVER").unwrap_or_else(|_| "fog1".to_string());
    let wait_secs: u64 = std::env::var("FOG_SHOT_WAIT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(90);
    let root = repo_root();

    println!("== starting the test server `{name}` on port {port}");
    let mut server = ServerGuard(
        Command::new(root.join("scripts/test-server.sh"))
            .args([&name, &port.to_string(), "600"])
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the test server"),
    );
    let stdout = server.0.stdout.take().expect("server stdout");
    let (line_tx, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            println!("server: {line}");
            let _ = line_tx.send(line);
        }
    });
    let ready = loop {
        match lines.recv_timeout(Duration::from_secs(240)) {
            Ok(line) if line.contains("Done (") => break true,
            Ok(_) => {}
            Err(_) => break false,
        }
    };
    assert!(ready, "test server did not load the world in 240 s");
    let mut console = main_console(&mut server.0);

    // A stone floor at y=99 with water from y=100 up, so the camera stands still with its eye
    // (y ~101.6) in a water block.
    for cmd in [
        "gamerule advance_time false",
        "gamerule send_command_feedback false",
        "time set 6000",
        "weather clear",
        "forceload add -16 -16 16 16",
        "fill -4 99 -4 4 99 4 minecraft:stone",
        "fill -4 100 -4 4 104 4 minecraft:water",
    ] {
        writeln!(console, "{cmd}").expect("setup command");
        console.flush().ok();
        std::thread::sleep(Duration::from_millis(150));
    }
    let water = run_shot(
        &root,
        port,
        "fogwater",
        &root.join("target/fog-water.png"),
        ["0.5", "100.0", "0.5"],
        "tp fogwater 0.5 100.0 0.5",
        wait_secs,
        &mut console,
        &lines,
    );

    // A Nether platform at y=69, then teleport the player to y=70 in the Nether.
    for cmd in [
        "execute in minecraft:the_nether run forceload add -16 -16 16 16",
        "execute in minecraft:the_nether run fill -4 69 -4 4 69 4 minecraft:netherrack",
    ] {
        writeln!(console, "{cmd}").expect("setup command");
        console.flush().ok();
        std::thread::sleep(Duration::from_millis(300));
    }
    let nether = run_shot(
        &root,
        port,
        "fognether",
        &root.join("target/fog-nether.png"),
        ["0.5", "70.0", "0.5"],
        "execute in minecraft:the_nether run tp fognether 0.5 70.0 0.5",
        wait_secs,
        &mut console,
        &lines,
    );

    drop(console);
    drop(server);
    println!("UNDERWATER FOG: {water}");
    println!("NETHER FOG: {nether}");
    assert!(
        water.contains("environment Water"),
        "underwater fog environment: {water}"
    );
    // The water fog colour is blue-dominant (the water-fog colour), and the water-vision ramp
    // keeps the end at or below the 96-block water fog end.
    let w = color_of(&water);
    assert!(w[2] > w[0], "underwater fog is blue: {water}");
    assert!(
        end_of(&water) <= 96.0,
        "underwater fog end is the water ramp: {water}"
    );
    // The Nether is the atmosphere again (`AtmosphericFogEnvironment`), a dark red: the
    // nether_wastes `FOG_COLOR` 0x330808 mixed toward the black Nether sky.
    assert!(
        nether.contains("environment Atmospheric"),
        "nether fog environment: {nether}"
    );
    let n = color_of(&nether);
    assert!(
        n[0] > n[1] && n[0] > n[2] && n[0] < 0.5,
        "nether fog is a dark red: {nether}"
    );
}

/// The `(r, g, b)` after `color (` in a client fog line.
fn color_of(line: &str) -> [f32; 3] {
    let inside = line.split("color (").nth(1).and_then(|s| s.split(')').next());
    let v: Vec<f32> = inside
        .unwrap_or("")
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    assert_eq!(v.len(), 3, "no colour in {line:?}");
    [v[0], v[1], v[2]]
}

/// The environmental end distance after `env ` in a client fog line.
fn end_of(line: &str) -> f32 {
    let inside = line.split("env ").nth(1).and_then(|s| s.split_whitespace().next());
    inside
        .and_then(|s| s.split("..").nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(f32::NAN)
}
