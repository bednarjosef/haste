//! `haste setup` and `haste singleplayer` subcommands (see docs/briefs/setup-cli.md).
//! Self-contained: `main.rs` wires it in with `mod launch;` and a dispatch line when
//! `Args.command` is `Some`.

use clap::Subcommand;
use std::time::Instant;

/// The `haste` subcommands. `main.rs` owns the top-level flags (`--server`, `--headless`, ...);
/// these are the actions that manage the installation and run the embedded server.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Download the 26.3 client/server jars, assets, libraries, JDK and run the data generator.
    /// Idempotent: a second run with everything present is fast.
    Setup,
    /// Start the embedded singleplayer server and print the address to join.
    Singleplayer {
        /// World name; the world lives under the config dir.
        #[arg(long, default_value = "world")]
        world: String,
    },
}

/// Runs one subcommand. Blocking (own tokio runtime inside for the async steps).
pub fn run(cmd: Command) -> anyhow::Result<()> {
    match cmd {
        Command::Setup => run_setup(),
        Command::Singleplayer { world } => tokio::runtime::Runtime::new()?.block_on(async move {
            let (server, address) = start_singleplayer(&world).await?;
            println!("singleplayer server on {address}; join with: haste --server {address}");
            wait_for_ctrl_c().await;
            server.stop().await;
            println!("server stopped");
            Ok(())
        }),
    }
}

/// `haste setup`: `ensure_installed` with a readable progress line per step. Byte progress is
/// throttled to one line per second; per-file events print one line each.
fn run_setup() -> anyhow::Result<()> {
    tokio::runtime::Runtime::new()?.block_on(async {
        let start = Instant::now();
        let mut last_bytes = Instant::now();
        let install = haste_setup::install::ensure_installed(move |p| match p {
            haste_setup::download::Progress::File { done, total } => {
                println!("  files {done}/{total}");
            }
            haste_setup::download::Progress::Bytes { done, total } => {
                if last_bytes.elapsed().as_millis() >= 1000 && done < total {
                    last_bytes = Instant::now();
                    println!("  {done}/{total} bytes");
                }
            }
        })
        .await?;
        println!(
            "installation ready in {:.1}s (client: {}, jdk: {})",
            start.elapsed().as_secs_f32(),
            install.client_jar.display(),
            install.jdk_java.display()
        );
        Ok(())
    })
}

/// Starts the embedded singleplayer server on a free port with the world under the config dir.
/// Returns the running server and its `host:port` address; the caller owns the server (dropping
/// the `LocalServer` does not stop it, call `stop`).
pub async fn start_singleplayer(
    world: &str,
) -> anyhow::Result<(haste_setup::server::LocalServer, String)> {
    let world_dir = singleplayer_world_dir(world);
    let port = haste_setup::server::free_port()?;
    let server = haste_setup::server::LocalServer::start(&world_dir, port).await?;
    let address = server.address();
    Ok((server, address))
}

/// Where singleplayer worlds live: `<config dir>/singleplayer/<world>`.
fn singleplayer_world_dir(world: &str) -> std::path::PathBuf {
    haste_setup::config::config_path()
        .expect("platform config dir exists on this system")
        .parent()
        .expect("config.toml always has a parent")
        .join("singleplayer")
        .join(world)
}

/// Ctrl-C handler: stopping the server cleanly on exit lets the world save.
async fn wait_for_ctrl_c() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_dir_is_under_config_singleplayer() {
        let dir = singleplayer_world_dir("gltest");
        let expected = std::path::Path::new("singleplayer").join("gltest");
        assert!(dir.ends_with(&expected), "world dir is {dir:?}");
        // a different world gets its own dir
        assert_ne!(singleplayer_world_dir("other"), dir);
    }
}
