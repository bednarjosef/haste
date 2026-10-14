//! `haste`: the client binary. Thin: each module belongs to one workstream (see AGENTS.md).
//!
//! - `net`     (protocol): connection task, packet -> world/entity/player calls, outgoing packets.
//! - `app`     (renderer): window, render loop, frame timing.
//! - `player`  (physics): input sampling, physics tick, movement packets.
//! - `screen_effects`: the overlays vanilla's `ScreenEffectRenderer` and `Hud` draw over the
//!   world (water, fire, in-wall, pumpkin, powder snow, spyglass, portal, nausea), driven by
//!   the real state.
//! - `ui`      (jvm-host): HUD + tier-3 screens through the batched GUI pipeline.
//! - `launch`  (setup): CLI args, config, setup checks, singleplayer server lifecycle.
//! - `headless`: join milestone mode, no window: joins, tracks world/entities, prints stats.
//! - `sound`  (sound): packets and music into `haste-audio`'s output, 20 Hz on the game thread.
//!
//! The lead owns this file. Add a module by adding one line here and your own file.
use clap::Parser;

mod app;
mod game;
mod hand;
mod headless;
mod launch;
mod net;
mod player;
mod screen_effects;
mod screenshots;
mod script;
mod sound;
mod ui;

#[derive(Parser, Debug)]
#[command(
    name = "haste",
    version,
    about = "Haste: a Minecraft 26.3 client in Rust"
)]
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
    /// Join, wait until the world is meshed, render one frame offscreen to this PNG and exit
    /// (`--seconds` bounds the wait).
    #[arg(long)]
    pub screenshot: Option<std::path::PathBuf>,
    /// Input script for the offscreen client: runs the steps every game tick through the
    /// same routing as the window (needs `--server`; `--seconds` bounds the run).
    #[arg(long)]
    pub script: Option<std::path::PathBuf>,
    /// Bench mode (needs `--server`): after the world settles (as `--screenshot`), render
    /// offscreen along a fixed camera path for this many seconds and print a frame-time
    /// summary (`--seconds` bounds the settle wait).
    #[arg(long)]
    pub bench: Option<u64>,
    /// Bench mode: write the summary as JSON to this file.
    #[arg(long)]
    pub bench_out: Option<std::path::PathBuf>,
    /// Render distance in chunks, overriding the saved option (like `--vsync`).
    #[arg(long)]
    pub render_distance: Option<u8>,
    /// Screenshot width in pixels.
    #[arg(long, default_value_t = 960)]
    pub width: u32,
    /// Screenshot height in pixels.
    #[arg(long, default_value_t = 540)]
    pub height: u32,
    /// Screenshot camera yaw in degrees (default: the server's spawn rotation).
    #[arg(long, allow_hyphen_values = true)]
    pub yaw: Option<f32>,
    /// Screenshot camera pitch in degrees (default: the server's spawn rotation).
    #[arg(long, allow_hyphen_values = true)]
    pub pitch: Option<f32>,
    /// Screenshot: feet position to wait for and render from (camera at eye height above it).
    #[arg(long, num_args = 3, value_names = ["X", "Y", "Z"], allow_hyphen_values = true)]
    pub pos: Option<Vec<f64>>,
    /// Screenshot perspective: first, back (third person behind) or front (third person in
    /// front, looking back at the player).
    #[arg(long, default_value = "first")]
    pub perspective: String,
    /// Present with vsync; overrides the saved option when given.
    #[arg(long)]
    pub vsync: Option<bool>,
    /// Close the window after this many seconds (unattended runs).
    #[arg(long)]
    pub window_seconds: Option<u64>,
    /// Subcommand (setup, singleplayer). Omit for the normal launcher flow.
    #[command(subcommand)]
    pub command: Option<launch::Command>,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?),
        )
        .with_writer(std::io::stderr)
        .init();
    let args = Args::parse();
    if let Some(cmd) = args.command {
        return launch::run(cmd, &args.username);
    }
    if args.ping {
        let addr = args
            .server
            .clone()
            .ok_or_else(|| anyhow::anyhow!("--ping needs --server"))?;
        let status = tokio::runtime::Runtime::new()?.block_on(haste_protocol::ping(&addr, None))?;
        println!("{status:#?}");
        return Ok(());
    }
    if args.headless {
        return headless::run(&args);
    }
    if let Some(file) = args.screenshot.clone() {
        return app::screenshot(&args, &file);
    }
    if let Some(file) = args.script.clone() {
        return app::script_run(&args, &file);
    }
    if let Some(seconds) = args.bench {
        return app::bench(&args, seconds);
    }
    app::run(&args)
}
