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

mod ambient_particles;
mod app;
mod block_particles;
mod boat;
mod game;
mod hand;
mod headless;
mod horse;
mod item_based_steering;
mod item_particles;
mod launch;
mod net;
mod pig;
mod player;
mod screen_effects;
mod screenshots;
mod script;
mod script_frame;
mod sound;
mod strider;
mod travel;
mod ui;
mod world_border;

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
    /// The HUD, hand and block outline (vanilla's F1, `Hud.toggle`): `off` hides them for
    /// this run — the whole JVM stays down, so offscreen shots match vanilla's F1 reference.
    #[arg(long, default_value = "on")]
    pub hud: String,
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
    /// Windowed bench mode (needs `--server`): like `--bench`, but a real window at
    /// `--width`x`--height` with vsync off (unless `--vsync`), so the number includes
    /// the present and is comparable with a windowed vanilla client. The window opens
    /// for the duration of the run and closes by itself.
    #[arg(long)]
    pub bench_window: Option<u64>,
    /// Force the window onto XWayland (the window system vanilla's client uses:
    /// an X11 window gets a framebuffer of exactly `--width`x`--height`, no
    /// desktop-scale doubling). Applies to the windowed bench and the normal
    /// window.
    #[arg(long)]
    pub x11: bool,
    /// Bench mode: hold the settle camera still instead of the flight path (isolates one view,
    /// e.g. the screenshot comparison spot).
    #[arg(long)]
    pub bench_fixed: bool,
    /// Pre-mesh mode (turn prediction for the mesh gate), overriding the saved option:
    /// `off` is exactly vanilla's visible-only meshing, `cone` widens the mesh gate's
    /// frustum toward the turn, `predict` drives that widening with an online yaw
    /// predictor. The default comes from the config's `[options] premesh`.
    #[arg(long)]
    pub premesh: Option<String>,
    /// Terrain GPU format: `vertex` (four 20-byte vertices plus six indices per
    /// quad) or `quad` (vertex pulling: one 32-byte record per axis-aligned quad,
    /// no index buffer). Set before the renderer and mesher start.
    #[arg(long, default_value = "vertex")]
    pub terrain_format: String,
    /// Windowed bench camera path: `still` (default) holds the settle camera, `turns`
    /// holds, then does 180-degree turns at a fixed speed every few seconds plus a
    /// straight flight leg — the same motion every run, for the pre-mesh comparison.
    #[arg(long, default_value = "still")]
    pub bench_path: String,
    /// Render distance in chunks, overriding the saved option (like `--vsync`).
    #[arg(long)]
    pub render_distance: Option<u8>,
    /// Where singleplayer worlds live, overriding the config dir's `singleplayer/`
    /// (the feature checks point this at a scratch dir so their worlds never land in the
    /// player's real saves).
    #[arg(long)]
    pub singleplayer_dir: Option<std::path::PathBuf>,
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
    if let Some(dir) = &args.singleplayer_dir {
        haste_setup::worlds::set_dir(dir.clone());
    }
    app::set_hud_from_args(&args.hud)?;
    match args.terrain_format.as_str() {
        "vertex" => {}
        "quad" => haste_render::set_terrain_format_quad(true),
        other => anyhow::bail!("--terrain-format must be `vertex` or `quad`, got `{other}`"),
    }
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
    if let Some(seconds) = args.bench_window {
        return app::bench_window(&args, seconds);
    }
    app::run(&args)
}
