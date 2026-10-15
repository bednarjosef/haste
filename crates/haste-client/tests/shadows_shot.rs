//! Dev-only helper (ignored test, no production paths): starts a private test server, summons a
//! pig and a named zombie on the flat ground in front of the test camera through the server
//! console (the same command source as RCON), and takes one offscreen screenshot with haste.
//! Prints the client's summary line, whose shadow-quad, entity-instance and draw-call counts are
//! what the shadows/name-tags brief asks for.
//!
//! usage:
//!   cargo test -q -p haste-client --test shadows_shot -- --ignored --nocapture
//!
//! env: SHADOWS_SHOT_SERVER (server copy name, default shadows1), SHADOWS_SHOT_WAIT (client
//!       wait bound, default 120).

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Frozen noon, nothing spawning; a pig (shadowRadius 0.7) at (1.5, -60, 2.5) and a named
/// zombie (0.5) at (-1.5, -60, 2.5), both facing the camera at (0, -60, -2) looking +z.
const SETUP_COMMANDS: &[&str] = &[
    "gamerule advance_time false",
    "gamerule advance_weather false",
    "gamerule spawn_mobs false",
    "gamerule spawn_monsters false",
    "gamerule send_command_feedback false",
    "time set 6000",
    "weather clear",
    "forceload add -32 -32 32 32",
    "kill @e[type=!player]",
    "summon pig 1.5 -60.0 2.5 {Rotation:[0f,0f]}",
    "summon zombie -1.5 -60.0 2.5 {CustomName:'\"Bob\"',CustomNameVisible:1b,Rotation:[0f,0f]}",
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
fn screenshot_with_server_entity_shadows() {
    let port: u16 = 25_721;
    let name = std::env::var("SHADOWS_SHOT_SERVER").unwrap_or_else(|_| "shadows1".to_string());
    let wait_secs: u64 = std::env::var("SHADOWS_SHOT_WAIT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);
    let root = repo_root();
    let shot = root.join("target/shadows-shot.png");

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
            "shadows",
            "--screenshot",
            shot.to_str().unwrap(),
            "--seconds",
            &wait_secs.to_string(),
            "--pos",
            "0.0",
            "-60.0",
            "-2.0",
            "--yaw",
            "0",
            "--pitch",
            "6",
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

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match lines.recv_timeout(Duration::from_secs(5)) {
            Ok(line) if line.contains("shadows joined the game") => {
                writeln!(console, "tp shadows 0.0 -60.0 -2.0").expect("tp");
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
