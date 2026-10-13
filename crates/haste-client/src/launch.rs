//! `haste setup` and `haste singleplayer` subcommands (see docs/briefs/setup-cli.md).
//! Self-contained: `main.rs` wires it in with `mod launch;` and a dispatch line when
//! `Args.command` is `Some`.

use anyhow::Context;
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
    /// Launch the real vanilla 26.3 client (side-by-side comparison with haste).
    Vanilla {
        /// Install and launch Fabric Loader instead of the vanilla main class.
        #[arg(long)]
        fabric: bool,
        /// `host:port` to join immediately (quick play).
        #[arg(long)]
        server: Option<String>,
        /// Offline-mode username the vanilla client plays under.
        #[arg(long, default_value = "HasteRef")]
        username: String,
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
        Command::Vanilla {
            fabric,
            server,
            username,
        } => run_vanilla(fabric, server, username),
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

/// `haste vanilla`: launch the real client through the haste-setup launcher and wait for it.
/// The game dir and client log stay behind after the client exits (haste-setup owns both).
fn run_vanilla(fabric: bool, server: Option<String>, username: String) -> anyhow::Result<()> {
    tokio::runtime::Runtime::new()?.block_on(async move {
        ensure_vanilla_installation()?;
        let opts = vanilla_launch_options(fabric, server, username);
        let mut client = haste_setup::launch::launch_vanilla(opts).await?;
        println!(
            "vanilla client started (pid {}, game dir {})",
            client.child.id(),
            client.game_dir.display()
        );
        let status = client
            .child
            .wait()
            .context("waiting for the vanilla client to exit")?;
        if !status.success() {
            return Err(anyhow::anyhow!("vanilla client exited with {status}"));
        }
        println!("vanilla client exited cleanly");
        Ok(())
    })
}

/// The launcher options for `haste vanilla`: offline username, optional quick-play join,
/// the Fabric loader when asked for.
fn vanilla_launch_options(
    fabric: bool,
    server: Option<String>,
    username: String,
) -> haste_setup::launch::LaunchOptions {
    haste_setup::launch::LaunchOptions {
        username,
        quick_play_server: server,
        fabric,
        ..Default::default()
    }
}

/// Fails with a clear error when the installation the vanilla launcher needs is absent
/// (client jar, libraries, assets, JDK), pointing at `haste setup` before any launch happens.
/// Purely local checks: no network, no process, before the client is ever started.
fn ensure_vanilla_installation() -> anyhow::Result<()> {
    let missing: Vec<String> = [
        haste_core::paths::client_jar(),
        haste_core::paths::assets_dir(),
        haste_setup::install::libraries_dir(),
    ]
    .into_iter()
    .filter(|p| !p.exists())
    .map(|p| p.display().to_string())
    .collect();
    if !missing.is_empty() {
        return Err(anyhow::anyhow!(
            "missing installation for the vanilla client ({}); run `haste setup` first",
            missing.join(", ")
        ));
    }
    if haste_setup::install::resolve_cached_java().is_none() {
        return Err(anyhow::anyhow!(
            "no java in {}; run `haste setup` first",
            haste_core::paths::cache_dir().display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanilla_launch_options_from_flags() {
        let opts = vanilla_launch_options(false, None, "HasteRef".into());
        assert_eq!(opts.username, "HasteRef");
        assert_eq!(opts.quick_play_server, None);
        assert!(!opts.fabric);
        assert_eq!(opts.user_type, "legacy");

        let opts = vanilla_launch_options(true, Some("localhost:25565".into()), "Steve".into());
        assert!(opts.fabric);
        assert_eq!(opts.quick_play_server.as_deref(), Some("localhost:25565"));
        assert_eq!(opts.username, "Steve");
        // the username flows into the offline UUID, so different names give different UUIDs
        let a = haste_setup::launch::offline_uuid(&opts.username);
        let b = haste_setup::launch::offline_uuid("HasteRef");
        assert_ne!(a, b);
    }

    #[test]
    fn vanilla_missing_installation_is_a_clear_error() {
        std::env::set_var("HASTE_CACHE", "/nonexistent-haste-cache-for-test");
        let err = ensure_vanilla_installation().expect_err("nothing exists under the empty cache");
        let msg = format!("{err:#}");
        assert!(msg.contains("run `haste setup`"), "message was: {msg}");
        assert!(msg.contains("client.jar"), "message was: {msg}");
    }

    #[test]
    fn world_dir_is_under_config_singleplayer() {
        let dir = singleplayer_world_dir("gltest");
        let expected = std::path::Path::new("singleplayer").join("gltest");
        assert!(dir.ends_with(&expected), "world dir is {dir:?}");
        // a different world gets its own dir
        assert_ne!(singleplayer_world_dir("other"), dir);
    }
}
