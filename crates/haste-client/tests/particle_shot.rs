//! Dev-only helper (ignored test, no production paths): starts a private test server, feeds
//! a `/particle` command to its console while haste takes one offscreen screenshot, and
//! prints the client's summary line (whose particle drawn count is what the particles brief
//! asks for).
//!
//! usage:
//!   cargo test -q -p haste-client --test particle_shot -- --ignored --nocapture
//!
//! env: PARTICLE_SHOT_SERVER (server copy name, default shot1), PARTICLE_SHOT_SECS (how long
//!       the particle loop runs, default 90), PARTICLE_SHOT_WAIT (client wait bound, 120).

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The flame loop three blocks in front of the test camera: the client spawns at feet
/// (-1.5, -60, 3.5) looking +z (yaw 0), so the flames stand at (-1.5, -58.5, 6.5).
const PARTICLE_COMMAND: &str = "particle flame -1.5 -58.5 6.5 0.05 0.05 0.05 0.05 60 force";

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
fn screenshot_with_server_particles() {
    let port: u16 = 25_700;
    // A fresh server copy per run: a leftover world session.lock would refuse the start.
    let name = std::env::var("PARTICLE_SHOT_SERVER").unwrap_or_else(|_| "shot1".to_string());
    let loop_secs: u64 = std::env::var("PARTICLE_SHOT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(90);
    let wait_secs: u64 = std::env::var("PARTICLE_SHOT_WAIT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);
    let root = repo_root();
    let shot = root.join("target/particles-shot.png");

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
    // Drain the server's own log lines on a thread (its pipe must never block) and tell us
    // when the world is loaded.
    let (line_tx, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            println!("server: {line}");
            let _ = line_tx.send(line);
        }
    });
    // Wait for the world to load (each receive is bounded by the timeout).
    let ready = loop {
        match lines.recv_timeout(Duration::from_secs(180)) {
            Ok(line) if line.contains("Done (") => break true,
            Ok(_) => {}
            Err(_) => break false,
        }
    };
    assert!(ready, "test server did not load the world in 180 s");

    println!("== taking the screenshot with target/debug/haste");
    let mut client = Command::new(root.join("target/debug/haste"))
        .args([
            "--server",
            &format!("127.0.0.1:{port}"),
            "--username",
            "particles",
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
            "10",
        ])
        .current_dir(&root)
        .env("RUST_LOG", "haste_render=debug")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the screenshot client");
    // Print the client's own log lines from threads, so neither pipe blocks.
    if let Some(out) = client.stdout.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
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

    // Repeat the particle spawn while the client settles, meshes and renders its one frame.
    let mut console = server.0.stdin.take().expect("server console");
    let deadline = Instant::now() + Duration::from_secs(loop_secs);
    let mut sent = 0;
    while Instant::now() < deadline {
        if writeln!(console, "{PARTICLE_COMMAND}").is_err() {
            break;
        }
        console.flush().ok();
        sent += 1;
        std::thread::sleep(Duration::from_millis(400));
    }
    println!("particle commands sent: {sent}");

    let status = client.wait().expect("client exit");
    println!("client exit: {status:?}");
    drop(console);
    drop(server);
}
