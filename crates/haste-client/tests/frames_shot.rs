//! Dev-only helper (ignored test, no production paths): starts a private test server, summons
//! an item frame holding a diamond (rotation 1), a glow item frame and a 2x2 painting in front
//! of the test camera through the server console (the same command source as RCON), and takes
//! one offscreen screenshot with haste. Prints the client's summary line, whose entity-instance
//! and draw-call counts are what the frames/paintings brief asks for.
//!
//! usage:
//!   cargo test -q -p haste-client --test frames_shot -- --ignored --nocapture
//!
//! env: FRAMES_SHOT_SERVER (server copy name, default frames1), FRAMES_SHOT_WAIT (client wait
//!       bound, default 120).

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The wall is force-loaded so it can be built before the player arrives; the test camera is
/// then teleported to feet (-1.5, -60, 3.5) looking +z (yaw 0, pitch 8). A stone wall at z = 7
/// carries a north-facing item frame at (-2, -59, 6) holding a diamond (rotation 1), a glow
/// item frame at (0, -59, 6) and a 2x2 `match` painting at (2, -59, 6).
const SETUP_COMMANDS: &[&str] = &[
    "gamerule advance_time false",
    "gamerule send_command_feedback false",
    "time set 6000",
    "weather clear",
    "forceload add -32 -32 32 32",
    "kill @e[type=!player]",
    "fill -6 -62 7 4 -53 7 minecraft:stone",
    "summon item_frame -2 -59 6 {Facing:2b,Item:{id:\"minecraft:diamond\",count:1b},ItemRotation:1b}",
    "summon glow_item_frame 0 -59 6 {Facing:2b}",
    "summon painting 2 -59 6 {facing:2b,variant:\"minecraft:match\"}",
];

/// Stops the server even when the test panics.
struct ServerGuard(Child);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crate inside the repo")
        .to_path_buf()
}

#[test]
#[ignore = "needs the local test server cache (dev screenshot helper)"]
fn screenshot_with_server_frames_and_paintings() {
    let port: u16 = 25_720;
    let name = std::env::var("FRAMES_SHOT_SERVER").unwrap_or_else(|_| "frames1".to_string());
    let wait_secs: u64 = std::env::var("FRAMES_SHOT_WAIT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);
    let root = repo_root();
    let shot = root.join("target/frames-shot.png");

    println!("== starting the test server `{name}` on port {port}");
    let mut server = ServerGuard(
        Command::new(root.join("scripts/test-server.sh"))
            .args([&name, &port.to_string(), "300"])
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
        match lines.recv_timeout(Duration::from_secs(180)) {
            Ok(line) if line.contains("Done (") => break true,
            Ok(_) => {}
            Err(_) => break false,
        }
    };
    assert!(ready, "test server did not load the world in 180 s");

    // Build the wall and summon the decorations through the console (one command per line,
    // exactly as RCON would send them). Force-loading first makes the chunks writable.
    let mut console = server.0.stdin.take().expect("server console");
    for cmd in SETUP_COMMANDS {
        writeln!(console, "{cmd}").expect("send setup command");
        console.flush().ok();
        std::thread::sleep(Duration::from_millis(200));
    }

    println!("== taking the screenshot with target/debug/haste");
    let mut client = Command::new(root.join("target/debug/haste"))
        .args([
            "--server",
            &format!("127.0.0.1:{port}"),
            "--username",
            "frames",
            "--screenshot",
            shot.to_str().unwrap(),
            "--seconds",
            &wait_secs.to_string(),
            "--pos",
            "-1.5",
            "-60.0",
            "3.5",
            "--yaw",
            "0",
            "--pitch",
            "8",
        ])
        .current_dir(&root)
        .env("RUST_LOG", "haste_render=debug")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the screenshot client");
    let (summary_tx, summary) = mpsc::channel();
    if let Some(out) = client.stdout.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if line.contains("screenshot ") {
                    let _ = summary_tx.send(line.clone());
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

    // The client's `--pos` waits for the server to teleport it; do that as soon as it joins.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match lines.recv_timeout(Duration::from_secs(5)) {
            Ok(line) if line.contains("frames joined the game") => {
                writeln!(console, "tp frames -1.5 -60.0 3.5").expect("tp");
                console.flush().ok();
                break;
            }
            Ok(_) => {}
            Err(_) if Instant::now() > deadline => break,
            Err(_) => {}
        }
    }

    let status = client.wait().expect("client exit");
    drop(console);
    drop(server);
    println!("client exit: {status:?}");
    println!("screenshot: {}", shot.display());
    match summary.try_recv() {
        Ok(line) => println!("SUMMARY: {line}"),
        Err(_) => println!("SUMMARY: no screenshot summary line captured"),
    }
}
