//! End-to-end regression test for the game mode reaching the HUD (the play-test report: joined
//! in creative, the server switched the player to survival, the HUD still showed no health and
//! no hunger).
//!
//! Vanilla keeps the local game mode in `MultiPlayerGameMode.localPlayerMode`, written by
//! `ClientPacketListener.handleGameEvent` (CHANGE_GAME_MODE) and by the login and respawn
//! packets; `Hud.extractHotbarAndDecorations` draws the hearts and the hunger bar only while
//! `gameMode.canHurtPlayer()`. The client's equivalent store is
//! `haste_jvm::GameState::game_mode`, which the HUD's Java reads through the per-frame mirror.
//!
//! This test does what the play-test did: it starts the private test server, joins it with the
//! offscreen client (`--script`), and switches the player's game mode over RCON *while the HUD's
//! JVM is still starting* — the window in which a mode change used to be lost and the stale
//! `login` mode used to win. The client logs `hud frame: <mode> hearts N food M` per frame at
//! debug level (`Ui::frame`), so the heart and food sprite draw commands are counted without
//! opening any image; the screenshots themselves are not inspected.
//!
//! needs cache/ (server copy, JDK, client jar and libraries for the HUD's JVM) and a built
//! `target/debug/haste`; it starts its own server on a private port.
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const PORT: u16 = 25699;
const RCON_PORT: u16 = PORT + 1000;
const PASSWORD: &str = "haste-local";

fn copy_dir(from: &Path, to: &Path, skip: &[&str]) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy().to_string();
        if skip.contains(&name.as_str()) {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target, skip)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Replaces (or appends) a `key=value` line of a `server.properties` text.
fn set_property(text: &str, key: &str, value: &str) -> String {
    let mut out = String::new();
    let mut done = false;
    for line in text.lines() {
        if line.starts_with(&format!("{key}=")) {
            out.push_str(&format!("{key}={value}\n"));
            done = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !done {
        out.push_str(&format!("{key}={value}\n"));
    }
    out
}

/// One RCON packet, written in a single `write_all`: the 26.3 server closes the connection when
/// the length prefix and the body arrive as separate TCP segments.
fn rcon_send(s: &mut TcpStream, id: i32, kind: i32, body: &str) -> std::io::Result<()> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&id.to_le_bytes());
    payload.extend_from_slice(&kind.to_le_bytes());
    payload.extend_from_slice(body.as_bytes());
    payload.extend_from_slice(&[0, 0]);
    let mut packet = (payload.len() as i32).to_le_bytes().to_vec();
    packet.extend_from_slice(&payload);
    s.write_all(&packet)
}

fn rcon_recv(s: &mut TcpStream) -> std::io::Result<i32> {
    let mut len = [0u8; 4];
    s.read_exact(&mut len)?;
    let n = i32::from_le_bytes(len) as usize;
    let mut buf = vec![0u8; n];
    s.read_exact(&mut buf)?;
    Ok(i32::from_le_bytes(buf[0..4].try_into().unwrap()))
}

/// One RCON command, retried until the server answers it (RCON comes up a moment after the port
/// opens, and a connection made too early is closed without a response).
fn rcon_command(command: &str) {
    for _ in 0..60 {
        let mut s = match TcpStream::connect_timeout(
            &format!("127.0.0.1:{RCON_PORT}").parse().unwrap(),
            Duration::from_secs(2),
        ) {
            Ok(s) => s,
            Err(_) => {
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
        };
        let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
        let _ = s.set_write_timeout(Some(Duration::from_secs(5)));
        rcon_send(&mut s, 1, 3, PASSWORD).unwrap();
        match rcon_recv(&mut s) {
            Ok(-1) => panic!("rcon auth failed"),
            Ok(_) => {}
            Err(_) => {
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
        }
        rcon_send(&mut s, 2, 2, command).unwrap();
        if rcon_recv(&mut s).is_err() {
            std::thread::sleep(Duration::from_millis(500));
            continue;
        }
        return;
    }
    panic!("rcon never answered {command:?}");
}

/// (mode, hearts, food) of every rendered frame, in order, from the client's debug log.
fn frames(log: &Path) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    if let Ok(text) = std::fs::read_to_string(log) {
        for line in text.lines() {
            let Some(rest) = line.split("hud frame: ").nth(1) else {
                continue;
            };
            let mut parts = rest.split_whitespace();
            let mode = parts.next().unwrap_or_default().to_string();
            let hearts = parts
                .find(|p| *p == "hearts")
                .and_then(|_| parts.next())
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
            let food = parts
                .find(|p| *p == "food")
                .and_then(|_| parts.next())
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
            out.push((mode, hearts, food));
        }
    }
    out
}

fn of_mode(log: &Path, mode: &str) -> Vec<(usize, usize)> {
    frames(log)
        .into_iter()
        .filter(|(m, _, _)| m == mode)
        .map(|(_, h, f)| (h, f))
        .collect()
}

/// Waits until the log contains at least one frame of `mode`.
fn wait_for_mode(log: &Path, mode: &str, deadline: Instant, what: &str) -> Vec<(usize, usize)> {
    loop {
        let found = of_mode(log, mode);
        if !found.is_empty() {
            return found;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for {what}; latest frames: {:?}", {
                let mut all = frames(log);
                all.reverse();
                all.truncate(3);
                all
            });
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Waits until the log's text contains `needle` (the server's log lines).
fn wait_for(needle: &str, log: &Path, deadline: Instant, what: &str) {
    loop {
        if std::fs::read_to_string(log)
            .unwrap_or_default()
            .contains(needle)
        {
            return;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for {what}");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
#[ignore = "needs cache/ (server copy, JDK, client jar, libraries) and a built target/debug/haste"]
fn game_mode_change_during_the_huds_startup_reaches_the_hud() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = root.parent().unwrap().parent().unwrap().to_path_buf();
    let cache = haste_core::paths::cache_dir();
    // A private server copy under target/, from the project's server copy (the same source
    // scripts/test-server.sh uses), with RCON on so the test can switch the game mode.
    let server_dir = root.join("target/servers/gamemode-test");
    let _ = std::fs::remove_dir_all(&server_dir);
    copy_dir(&cache.join("server-spike"), &server_dir, &["logs", "world"]).unwrap();
    let properties = std::fs::read_to_string(server_dir.join("server.properties")).unwrap();
    let properties = set_property(&properties, "server-port", &PORT.to_string());
    let properties = set_property(&properties, "query.port", &PORT.to_string());
    let properties = set_property(&properties, "rcon.port", &RCON_PORT.to_string());
    let properties = set_property(&properties, "enable-rcon", "true");
    let properties = set_property(&properties, "rcon.password", PASSWORD);
    let properties = set_property(&properties, "gamemode", "creative");
    let properties = set_property(&properties, "white-list", "false");
    let properties = set_property(&properties, "level-type", "minecraft:flat");
    std::fs::write(server_dir.join("server.properties"), properties).unwrap();
    let server_log = server_dir.join("logs/latest.log");
    let server: Child = Command::new(cache.join("jdk/bin/java"))
        .arg("-Xmx1500M")
        .arg("-jar")
        .arg("fabric-server-launch.jar")
        .arg("nogui")
        .current_dir(&server_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the test server");
    struct Guard {
        server: Option<Child>,
        haste: Option<Child>,
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            for child in [&mut self.haste, &mut self.server] {
                if let Some(mut c) = child.take() {
                    let _ = c.kill();
                    let _ = c.wait();
                }
            }
        }
    }
    let mut guard = Guard {
        server: Some(server),
        haste: None,
    };
    wait_for(
        "Done (",
        &server_log,
        Instant::now() + Duration::from_secs(150),
        "the test server to start",
    );
    let script = root.join("target/gamemode-test.script");
    std::fs::write(&script, "wait_until settled\nwait 400\n").unwrap();
    let haste_log = root.join("target/haste-gamemode.err");
    let _ = std::fs::remove_file(&haste_log);
    let haste_out = std::fs::File::create(&haste_log).unwrap();
    let haste: Child = Command::new("target/debug/haste")
        .arg("--script")
        .arg("target/gamemode-test.script")
        .arg("--server")
        .arg(format!("127.0.0.1:{PORT}"))
        .arg("--username")
        .arg("haste")
        .arg("--seconds")
        .arg("120")
        .current_dir(&root)
        .env("RUST_LOG", "haste::ui=debug")
        .stdout(Stdio::from(haste_out.try_clone().unwrap()))
        .stderr(Stdio::from(haste_out))
        .spawn()
        .expect("spawn the offscreen client");
    guard.haste = Some(haste);
    // switch the player to survival the moment the server has them, before the HUD's JVM has
    // finished starting (it takes seconds): the change must still reach the Java side.
    wait_for(
        "logged in with entity id",
        &server_log,
        Instant::now() + Duration::from_secs(60),
        "the client to join",
    );
    rcon_command("gamemode survival haste");
    // the HUD's frames show survival, with the hearts and the hunger bar drawn
    let survival = wait_for_mode(
        &haste_log,
        "Survival",
        Instant::now() + Duration::from_secs(90),
        "the HUD to render survival frames",
    );
    assert!(
        survival.iter().all(|(h, f)| *h >= 10 && *f >= 10),
        "survival frames without the vitals: {survival:?}"
    );
    // and back to creative: the hearts and the hunger bar go away again
    rcon_command("gamemode creative haste");
    let creative = wait_for_mode(
        &haste_log,
        "Creative",
        Instant::now() + Duration::from_secs(30),
        "the HUD to render creative frames again",
    );
    assert!(
        creative.iter().all(|(h, _)| *h == 0),
        "creative frames with hearts: {creative:?}"
    );
}
