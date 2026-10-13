//! `haste setup`, `haste singleplayer`, `haste login` and `haste logout` subcommands
//! (see docs/briefs/setup-cli.md). Self-contained: `main.rs` wires it in with `mod launch;`
//! and a dispatch line when `Args.command` is `Some`.

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
    /// Sign in with a Microsoft account (device-code flow) and store it in the config.
    Login,
    /// Remove the stored Microsoft account from the config.
    Logout,
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
        Command::Login => tokio::runtime::Runtime::new()?.block_on(run_login()),
        Command::Logout => run_logout(),
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

/// `haste login`: the Microsoft device-code flow. Needs `HASTE_MSA_CLIENT_ID` (none is shipped);
/// checked before any network call. Stores tokens + profile in the config.
pub async fn run_login() -> anyhow::Result<()> {
    // begin_device_code is the first thing that would use the client id, so this check is what
    // keeps the unset-env error before any network call.
    require_client_id(haste_setup::msa::client_id())?;
    let tokens = haste_setup::msa::device_code_login(|device| {
        println!(
            "to sign in, open {} and enter the code {}",
            device.verification_uri, device.user_code
        );
        println!("waiting for sign-in...");
    })
    .await?;
    let name = tokens
        .profile
        .as_ref()
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let mut config = haste_setup::config::Config::load()?;
    config.msa = Some(tokens);
    config.save()?;
    println!("logged in as {name}");
    Ok(())
}

/// The `haste login` pre-flight: a client id must be configured (tests pass their own value
/// instead of the env var; `run_login` passes `msa::client_id()`).
fn require_client_id(client_id: Option<String>) -> anyhow::Result<String> {
    client_id.ok_or_else(|| {
        anyhow::anyhow!(
            "HASTE_MSA_CLIENT_ID is not set; haste ships no Microsoft client id. \
             Set it to an Azure application (client) id and retry."
        )
    })
}

/// `haste logout`: drops the stored Microsoft account (if any) and saves.
fn run_logout() -> anyhow::Result<()> {
    let path = haste_setup::config::config_path()
        .ok_or_else(|| anyhow::anyhow!("no platform config dir on this system"))?;
    run_logout_at(&path)
}

/// `haste logout` against an explicit config path; tests use temp files, the command uses the
/// platform config.
fn run_logout_at(path: &std::path::Path) -> anyhow::Result<()> {
    let mut config = haste_setup::config::Config::load_from(path)?;
    match config.msa.take() {
        Some(tokens) => {
            let name = tokens
                .profile
                .as_ref()
                .map(|p| p.name.as_str())
                .unwrap_or("unknown account");
            config.save_to(path)?;
            println!("logged out {name}");
        }
        None => println!("no account was logged in"),
    }
    Ok(())
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

    /// A fresh config path in the system temp dir (haste-client has no tempfile dev-dependency;
    /// the dir holds only fake tokens). Each call gets its own path, so parallel tests do not
    /// share state. No process environment variables are touched.
    fn temp_config_path(label: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("haste-launch-tests-{}-{label}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("config.toml")
    }

    #[test]
    fn login_without_client_id_is_a_clear_error() {
        let err = require_client_id(None).expect_err("no client id must be an error");
        assert!(
            err.to_string().contains("HASTE_MSA_CLIENT_ID"),
            "error should name the env var: {err}"
        );
        // and a present id passes straight through
        assert_eq!(require_client_id(Some("id".into())).unwrap(), "id");
    }

    #[test]
    fn logout_removes_a_stored_account() {
        let path = temp_config_path("logout-removes");
        let mut config = haste_setup::config::Config::load_from(&path).unwrap();
        config.msa = Some(haste_setup::config::MsaTokens {
            access_token: "tok".into(),
            refresh_token: "ref".into(),
            expires_at: Some(1900000000),
            profile: Some(haste_setup::config::MsaProfile {
                uuid: "u".into(),
                name: "Notch".into(),
            }),
        });
        config.save_to(&path).unwrap();
        run_logout_at(&path).unwrap();
        assert!(haste_setup::config::Config::load_from(&path)
            .unwrap()
            .msa
            .is_none());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn logout_without_account_is_ok() {
        let path = temp_config_path("logout-empty");
        haste_setup::config::Config::load_from(&path).unwrap(); // creates a config with no msa
        run_logout_at(&path).unwrap();
        assert!(haste_setup::config::Config::load_from(&path)
            .unwrap()
            .msa
            .is_none());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
