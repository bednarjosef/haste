//! `haste setup` and `haste singleplayer` subcommands (see docs/briefs/setup-cli.md).
//! Self-contained: `main.rs` wires it in with `mod launch;` and a dispatch line when
//! `Args.command` is `Some`.

use anyhow::Context;
use clap::{Parser, Subcommand};
use std::time::Instant;

/// The `haste` subcommands. `main.rs` owns the top-level flags (`--server`, `--headless`, ...);
/// these are the actions that manage the installation and run the embedded server.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Download the 26.3 client/server jars, assets, libraries, JDK and run the data generator.
    /// Idempotent: a second run with everything present is fast.
    Setup,
    /// Start the embedded singleplayer server and open the game window joined to it.
    Singleplayer {
        /// World name; the world lives under the config dir.
        #[arg(long, default_value = "world")]
        world: String,
        /// Do not open the game window: print the address and wait for Ctrl-C (scripts, tests).
        #[arg(long)]
        no_window: bool,
        /// Close the game window after this many seconds (unattended runs).
        #[arg(long)]
        window_seconds: Option<u64>,
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
pub fn run(cmd: Command, username: &str) -> anyhow::Result<()> {
    match cmd {
        Command::Setup => run_setup(),
        Command::Singleplayer {
            world,
            no_window,
            window_seconds,
        } => {
            let rt = tokio::runtime::Runtime::new()?;
            let (server, address) = rt.block_on(start_singleplayer(&world, username))?;
            if no_window {
                println!("singleplayer server on {address}; join with: haste --server {address}");
                rt.block_on(wait_for_ctrl_c());
                rt.block_on(server.stop());
                println!("server stopped");
                return Ok(());
            }
            println!("singleplayer server on {address}; opening the game window");
            // Outside `block_on` on the main thread: `app::run` builds its own runtime to
            // join the server, which panics ("cannot start a runtime within a runtime")
            // if it were called from inside this one.
            let exit = crate::app::run(&client_args(address, username.to_string(), window_seconds));
            // Whatever closed the window (user, `--window-seconds`, disconnect), the
            // server comes down with it so no java process is left behind.
            rt.block_on(server.stop());
            println!("server stopped");
            exit
        }
        Command::Vanilla {
            fabric,
            server,
            username,
        } => run_vanilla(fabric, server, username),
    }
}

/// The client invocation for the singleplayer window: the plain defaults (`haste` with no
/// flags, parsed so there is one source of truth for them) with the local server, the user's
/// username and the window auto-close bound filled in.
fn client_args(address: String, username: String, window_seconds: Option<u64>) -> crate::Args {
    let mut args = crate::Args::parse_from(["haste"]);
    args.server = Some(address);
    args.username = username;
    args.window_seconds = window_seconds;
    args
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

/// Starts the embedded singleplayer server on a free port with the world under the config dir,
/// with `username` allowed through the server's whitelist (the 26.3 server defaults
/// `white-list=true`, which would refuse every join). Returns the running server and its
/// `host:port` address; the caller owns the server (dropping the `LocalServer` does not stop it,
/// call `stop`).
pub async fn start_singleplayer(
    world: &str,
    username: &str,
) -> anyhow::Result<(haste_setup::server::LocalServer, String)> {
    let world_dir = singleplayer_world_dir(world);
    write_whitelist(&world_dir, username)?;
    let port = haste_setup::server::free_port()?;
    let server = haste_setup::server::LocalServer::start(&world_dir, port).await?;
    let address = server.address();
    Ok((server, address))
}

/// Writes `whitelist.json` with the single joining player, so the server lets the local client
/// in. Pure: builds the file text for a username.
fn whitelist_json(username: &str) -> String {
    let uuid = haste_setup::launch::offline_uuid(username);
    format!("[\n  {{\n    \"uuid\": \"{uuid}\",\n    \"name\": \"{username}\"\n  }}\n]\n")
}

/// Writes `whitelist.json` (and its directory) into the world dir before the server starts.
fn write_whitelist(world_dir: &std::path::Path, username: &str) -> anyhow::Result<()> {
    tokio::task::block_in_place(|| {
        std::fs::create_dir_all(world_dir)
            .with_context(|| format!("creating {}", world_dir.display()))?;
        let path = world_dir.join("whitelist.json");
        std::fs::write(&path, whitelist_json(username))
            .with_context(|| format!("writing {}", path.display()))
    })
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
        ensure_vanilla_installation(haste_core::paths::cache_dir())?;
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
/// Purely local checks on the given cache root: no network, no process, before the client is
/// ever started. The cache root is a parameter so tests can pass an empty temp dir instead of
/// mutating the process-wide `HASTE_CACHE`.
fn ensure_vanilla_installation(cache: impl AsRef<std::path::Path>) -> anyhow::Result<()> {
    let cache = cache.as_ref();
    let missing: Vec<String> = [
        cache
            .join("versions")
            .join(haste_core::paths::MC_VERSION)
            .join("client.jar"),
        cache.join("assets"),
        cache.join("libraries"),
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
    let java = if std::env::consts::OS == "windows" {
        cache.join("jdk").join("bin").join("java.exe")
    } else {
        cache.join("jdk").join("bin").join("java")
    };
    if !java.is_file() {
        return Err(anyhow::anyhow!(
            "no java in {}; run `haste setup` first",
            cache.display()
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
        // an empty temp dir as the cache root: every check fails, no env var is touched
        let cache = std::env::temp_dir().join("haste-test-empty-cache");
        std::fs::create_dir_all(&cache).expect("create temp dir");
        let err = ensure_vanilla_installation(&cache).expect_err("nothing exists under the cache");
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

    #[test]
    fn client_args_join_the_local_server() {
        let args = client_args("127.0.0.1:25565".into(), "Steve".into(), Some(20));
        assert_eq!(args.server.as_deref(), Some("127.0.0.1:25565"));
        assert_eq!(args.username, "Steve");
        assert_eq!(args.window_seconds, Some(20));
        // nothing else is implied: windowed join, no screenshot, no ping
        assert!(!args.headless);
        assert!(!args.ping);
        assert!(args.screenshot.is_none());
        assert!(!args.vsync);
    }

    #[test]
    fn client_args_default_username_when_unnamed() {
        let args = client_args("127.0.0.1:1".into(), "haste".into(), None);
        assert_eq!(args.username, "haste");
        assert_eq!(args.window_seconds, None);
    }

    #[test]
    fn whitelist_json_names_the_joining_player() {
        let json = whitelist_json("haste");
        assert!(json.contains("\"name\": \"haste\""));
        // the offline UUID haste-setup computes for "haste", the one vanilla servers expect
        assert!(json.contains(&format!(
            "\"uuid\": \"{}\"",
            haste_setup::launch::offline_uuid("haste")
        )));
        // a different username must give a different whitelist entry
        assert_ne!(whitelist_json("steve"), json);
    }
}
