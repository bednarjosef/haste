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
    /// `pause-when-empty-seconds=-1`, `nogui`. Logs are captured next to the world dir and also
    /// scanned for the `Done` line (timeout 120 s).
    pub async fn start(world_dir: &Path, port: u16) -> anyhow::Result<LocalServer> {
        let server_jar = paths::server_jar();
        anyhow::ensure!(server_jar.is_file(), "server jar missing at {} (run ensure_installed)", server_jar.display());
        let java = crate::install::resolve_cached_java().ok_or_else(|| {
            anyhow!("no cache/jdk/bin/java; run ensure_installed first")
        })?;
        tokio::fs::create_dir_all(world_dir).await?;
        let world_name = world_dir
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| anyhow!("world dir must have a final component"))?
            .to_string();

        tokio::fs::write(world_dir.join("eula.txt"), "eula=true\n").await?;
        let properties = format!(
            "online-mode=false\n\
             server-ip=127.0.0.1\n\
             server-port={port}\n\
             level-name={world_name}\n\
             pause-when-empty-seconds=-1\n\
             spawn-protection=0\n\
             sync-chunk-writes=false\n\
             motd=haste\n"
        );
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let server = LocalServer::start(&world, port).await.expect("server started");
        assert_eq!(server.address(), format!("127.0.0.1:{port}"));
        // the port should now accept connections
        let conn = tokio::time::timeout(Duration::from_secs(5), tokio::net::TcpStream::connect(("127.0.0.1", port))).await;
        assert!(conn.is_ok(), "server port not accepting connections");
        server.stop().await;
        // after stop, the port closes eventually; give it a moment and check the process is gone
        tokio::time::sleep(Duration::from_secs(2)).await;
        let refused = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_err();
        assert!(refused, "server still accepting after stop");
    }
}
