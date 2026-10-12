//! `haste`: the client binary. Thin: each module belongs to one workstream (see AGENTS.md).
//!
//! - `net`     (WS1): connection task, packet -> world/entity/player calls, outgoing packets.
//! - `app`     (WS3): window, render loop, frame timing.
//! - `player`  (WS5): input sampling, physics tick, movement packets.
//! - `ui`      (WS6): HUD + tier-3 screens through the batched GUI pipeline.
//! - `launch`  (WS7): CLI args, config, setup checks, singleplayer server lifecycle.
//! - `headless`: M1 mode, no window: joins, tracks world/entities, prints stats.
//!
//! The lead owns this file. Add a module by adding one line here and your own file.
use clap::Parser;

mod headless;
mod net;

#[derive(Parser, Debug)]
#[command(name = "haste", version, about = "Haste: a Minecraft 26.3 client in Rust")]
pub struct Args {
    /// Server address (host or host:port). Omit for the launcher flow.
    #[arg(long)]
    pub server: Option<String>,
    /// Offline-mode username (no Microsoft login).
    #[arg(long, default_value = "haste")]
    pub username: String,
    /// Run without a window: join, track world and entities, print stats.
    #[arg(long)]
    pub headless: bool,
    /// Seconds to stay connected in headless mode.
    #[arg(long, default_value_t = 30)]
    pub seconds: u64,
    /// Server list ping only: print the status JSON and exit.
    #[arg(long)]
    pub ping: bool,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?)).init();
    let args = Args::parse();
    if args.ping {
        let addr = args.server.clone().ok_or_else(|| anyhow::anyhow!("--ping needs --server"))?;
        let status = tokio::runtime::Runtime::new()?.block_on(haste_protocol::ping(&addr, None))?;
        println!("{status:#?}");
        return Ok(());
    }
    if args.headless {
        return headless::run(&args);
    }
    anyhow::bail!("windowed mode arrives with M2 (see TASKS.md); use --headless for M1")
}
