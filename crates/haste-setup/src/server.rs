//! The embedded vanilla dedicated server (not Fabric): offline mode on 127.0.0.1 with a random
//! free port, started from `cache/versions/26.3/server.jar` inside `cache/singleplayer/<world>/`,
//! waiting for the "Done" log line, and stopped via `stop` on stdin.

use anyhow::{anyhow, Context};
use haste_core::paths;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

/// How long to wait for the server's "Done" line.
pub const START_TIMEOUT: Duration = Duration::from_secs(120);

pub struct LocalServer {
    child: Child,
    addr: std::net::SocketAddr,
    #[allow(dead_code)]
    log_path: PathBuf,
}

impl LocalServer {
    /// Starts the vanilla server jar in `cache/singleplayer/<world-name>/` (created if missing):
    /// `eula=true`, `online-mode=false`, `server-ip=127.0.0.1`, a random free port,
    /// `pause-when-empty-seconds=-1`, `nogui`. The world data lands directly in `world_dir`
    /// (`--universe ..` with `level-name=<world-name>`: the server puts the level at
    /// `<universe>/<level-name>`, so the level dir is `world_dir` itself, and the world list
    /// (`LevelStorageSource` over the parent) sees one folder per world). Logs are captured next
    /// to the world dir and also scanned for the `Done` line (timeout 120 s).
    pub async fn start(world_dir: &Path, port: u16) -> anyhow::Result<LocalServer> {
        Self::start_with(world_dir, port, &WorldSettings::for_dir(world_dir)?).await
    }

    /// [`LocalServer::start`] with explicit world settings (the create-world screen's choices:
    /// name, game mode, difficulty, cheats, seed, world type). They go into `server.properties`,
    /// which the server reads when it creates a missing world.
    pub async fn start_with(
        world_dir: &Path,
        port: u16,
        settings: &WorldSettings,
    ) -> anyhow::Result<LocalServer> {
        let server_jar = paths::server_jar();
        anyhow::ensure!(
            server_jar.is_file(),
            "server jar missing at {} (run ensure_installed)",
            server_jar.display()
        );
        let java = crate::install::resolve_cached_java()
            .ok_or_else(|| anyhow!("no cache/jdk/bin/java; run ensure_installed first"))?;
        tokio::fs::create_dir_all(world_dir).await?;
        let world_name = world_dir
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| anyhow!("world dir must have a final component"))?
            .to_string();
        anyhow::ensure!(
            settings.level_name == world_name,
            "level-name must be the world dir's name ({world_name}), got {}",
            settings.level_name
        );

        tokio::fs::write(world_dir.join("eula.txt"), "eula=true\n").await?;
        let properties = server_properties(settings, port);
        tokio::fs::write(world_dir.join("server.properties"), properties).await?;

        let log_path = world_dir.join("server.log");
        let log_file = tokio::fs::File::create(&log_path)
            .await
            .with_context(|| format!("creating {}", log_path.display()))?;

        let mut child = Command::new(&java)
            .arg("-Xms512M")
            .arg("-Xmx2G")
            .arg("-jar")
            .arg(&server_jar)
            // The level lands at `<universe>/<level-name>`; `..` makes that `world_dir` itself
            // (see `start`), so the world data is directly in the world's folder.
            .arg("--universe")
            .arg("..")
            .arg("nogui")
            .current_dir(world_dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("spawning server with {}", java.display()))?;

        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");
        // tee logs to the file and scan for the Done line
        let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
        // unbuffered: the log must be complete even if the caller inspects it right after Done
        // or the process is killed, and the server's stdout volume is modest
        let mut log_writer = log_file;
        let scan = async move {
            let mut lines = BufReader::new(stdout).lines();
            let mut done_tx = Some(done_tx);
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tokio::io::AsyncWriteExt::write_all(&mut log_writer, line.as_bytes()).await;
                let _ = tokio::io::AsyncWriteExt::write_all(&mut log_writer, b"\n").await;
                if line.contains("Done (") {
                    if let Some(tx) = done_tx.take() {
                        let _ = tx.send(());
                    }
                    // keep draining so the pipe does not fill up
                }
            }
        };
        let drain_err = async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(_line)) = lines.next_line().await {}
        };
        tokio::spawn(scan);
        tokio::spawn(drain_err);

        let started = tokio::time::timeout(START_TIMEOUT, done_rx).await;

        match started {
            Ok(Ok(())) => Ok(LocalServer {
                child,
                addr: std::net::SocketAddr::from(([127, 0, 0, 1], port)),
                log_path,
            }),
            // the scan task dropped the sender without a Done line: the server died early
            Ok(Err(_)) | Err(_) => {
                let _ = child.kill().await;
                Err(anyhow!(
                    "server did not print 'Done' within {} s (log: {})",
                    START_TIMEOUT.as_secs(),
                    log_path.display()
                ))
            }
        }
    }

    /// `host:port` to connect a client to.
    pub fn address(&self) -> String {
        self.addr.to_string()
    }
    /// Sends `stop` on stdin and waits for the process to exit.
    pub async fn stop(mut self) {
        if let Some(mut stdin) = self.child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(b"stop\n").await;
            let _ = stdin.flush().await;
        }
        let _ = tokio::time::timeout(Duration::from_secs(60), self.child.wait()).await;
    }
}

/// Finds a free TCP port on 127.0.0.1 (bound to port 0 and immediately released; there is a
/// small race window before the server binds, which is why callers retry on bind failure).
pub fn free_port() -> anyhow::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

/// The settings a singleplayer world is created with (vanilla's CreateWorldScreen choices that
/// the dedicated server can express in `server.properties`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldSettings {
    /// `level-name`: the world folder's name (the server creates the level at
    /// `<universe>/<level-name>`); also the name a fresh world's `level.dat` gets.
    pub level_name: String,
    /// `gamemode` (0 survival, 1 creative, 2 adventure, 3 spectator).
    pub game_mode: i32,
    /// `difficulty` (0 peaceful .. 3 hard).
    pub difficulty: i32,
    /// `hardcore`.
    pub hardcore: bool,
    /// Cheats (`allowCommands` in vanilla): the joining player is written into `ops.json` with
    /// level 4 so they may use commands, like vanilla singleplayer with cheats on.
    pub allow_commands: bool,
    /// `level-seed` (empty = random).
    pub seed: String,
    /// `generate-structures`.
    pub generate_structures: bool,
    /// `level-type` (a world-preset id like `minecraft:normal`, or a legacy name).
    pub level_type: String,
}

impl WorldSettings {
    /// The defaults for playing an existing world: the server reads the real settings from the
    /// world's `level.dat`; these only fill the properties it needs anyway.
    pub fn for_dir(world_dir: &Path) -> anyhow::Result<WorldSettings> {
        let level_name = world_dir
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| anyhow!("world dir must have a final component"))?
            .to_string();
        Ok(WorldSettings {
            level_name,
            game_mode: 0,
            difficulty: 1,
            hardcore: false,
            allow_commands: false,
            seed: String::new(),
            generate_structures: true,
            level_type: "minecraft:normal".into(),
        })
    }
}

/// The `server.properties` contents for a singleplayer world. Vanilla singleplayer has no
/// whitelist, and the 26.3 server defaults `white-list=true` when the property is absent, so it
/// is set to false explicitly (the server would otherwise refuse every join with
/// `multiplayer.disconnect.not_whitelisted`).
fn server_properties(settings: &WorldSettings, port: u16) -> String {
    format!(
        "online-mode=false\n\
         server-ip=127.0.0.1\n\
         server-port={port}\n\
         level-name={}\n\
         gamemode={}\n\
         difficulty={}\n\
         hardcore={}\n\
         level-seed={}\n\
         generate-structures={}\n\
         level-type={}\n\
         pause-when-empty-seconds=-1\n\
         spawn-protection=0\n\
         sync-chunk-writes=false\n\
         white-list=false\n\
         enforce-whitelist=false\n\
         motd=haste\n",
        settings.level_name,
        settings.game_mode,
        settings.difficulty,
        settings.hardcore,
        settings.seed,
        settings.generate_structures,
        settings.level_type
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn properties_disable_the_whitelist() {
        let properties = server_properties(
            &WorldSettings::for_dir(Path::new("saves/gltest")).unwrap(),
            25565,
        );
        assert!(properties.contains("white-list=false\n"));
        assert!(properties.contains("enforce-whitelist=false\n"));
        assert!(!properties.contains("white-list=true"));
        // the settings LocalServer has always written stay as they were
        assert!(properties.contains("online-mode=false\n"));
        assert!(properties.contains("server-port=25565\n"));
        assert!(properties.contains("level-name=gltest\n"));
    }

    #[test]
    fn properties_carry_the_create_screen_choices() {
        let settings = WorldSettings {
            level_name: "Haste_World".into(),
            game_mode: 1,
            difficulty: 3,
            hardcore: false,
            allow_commands: true,
            seed: "12345".into(),
            generate_structures: false,
            level_type: "minecraft:flat".into(),
        };
        let properties = server_properties(&settings, 25600);
        assert!(properties.contains("gamemode=1\n"));
        assert!(properties.contains("difficulty=3\n"));
        assert!(properties.contains("level-seed=12345\n"));
        assert!(properties.contains("generate-structures=false\n"));
        assert!(properties.contains("level-type=minecraft:flat\n"));
        assert!(properties.contains("level-name=Haste_World\n"));
    }

    #[test]
    fn free_port_is_bindable_and_random() {
        let a = free_port().unwrap();
        let b = free_port().unwrap();
        assert_ne!(a, b, "two consecutive free ports should differ (usually)");
        assert!(std::net::TcpListener::bind(("127.0.0.1", a)).is_ok());
    }

    /// Starts the real vanilla server in a temp dir under the cache, waits for "Done", connects a
    /// TCP socket, then stops it. Run with `cargo test -p haste-setup -- --ignored`.
    #[tokio::test]
    #[ignore = "needs cache/jdk + the 26.3 server jar; takes ~30 s"]
    async fn starts_and_stops_real_server() {
        if crate::install::resolve_cached_java().is_none() {
            eprintln!("skipping: no cache/jdk");
            return;
        }
        let world = paths::cache_dir().join("singleplayer").join("test-start");
        let port = free_port().unwrap();
        let server = LocalServer::start(&world, port)
            .await
            .expect("server started");
        assert_eq!(server.address(), format!("127.0.0.1:{port}"));
        // the port should now accept connections
        let conn = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::TcpStream::connect(("127.0.0.1", port)),
        )
        .await;
        assert!(conn.is_ok(), "server port not accepting connections");
        server.stop().await;
        // after stop, the port closes eventually; give it a moment and check the process is gone
        tokio::time::sleep(Duration::from_secs(2)).await;
        let refused = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_err();
        assert!(refused, "server still accepting after stop");
    }
}
