//! `--bench-window`: the benchmark in a real window, holding a still camera.

use super::*;

/// The windowed bench's state (`--bench-window`): the recorded samples, the still camera
/// (the settled frame's camera with `--yaw`/`--pitch`), and the run's metadata.
pub(crate) struct WindowBench {
    pub(crate) samples: Vec<BenchSample>,
    /// The camera every measured frame uses (the settle frame's, yaw/pitch from the args).
    pub(crate) camera: Option<Camera>,
    /// The still camera's view distance (the settled view's; the summary reports it).
    pub(crate) view_distance: i32,
    /// Total measured seconds; the run ends when it has elapsed since `started`.
    pub(crate) seconds: f32,
    pub(crate) started: Option<Instant>,
    pub(crate) feed_ms: f32,
    /// The window's exact inner size in physical pixels (`--width`x`--height`).
    pub(crate) size: (u32, u32),
    /// The look the bench camera holds (`--yaw`/`--pitch`; `None` keeps the player's own).
    pub(crate) look: (Option<f32>, Option<f32>),
    /// `--pos`: the settle also waits until the server has teleported the player here.
    pub(crate) want_pos: Option<DVec3>,
}

/// `--bench-window SECONDS`: the windowed twin of `bench`. Joins like the windowed client
/// (a real window at `--width`x`--height`, vsync off unless `--vsync`), waits for the world
/// to settle, then renders the still camera for SECONDS while recording the same
/// `BenchSample`s as the offscreen bench. Prints the same summary block (marked `windowed`)
/// and writes the same JSON to `--bench-out`; the window closes by itself.
pub fn bench_window(args: &Args, seconds: u64) -> anyhow::Result<()> {
    let Some(server) = args.server.as_deref() else {
        anyhow::bail!("--bench-window needs --server <host[:port]>");
    };
    let session = Session::join(&args.username, args.render_distance, server, false, None)?;
    let shared = session.shared.clone();
    // The bench camera is first person: the game thread's camera effects take that perspective.
    shared.input.lock().perspective = game::Perspective::FirstPerson;
    // `--x11`: the same XWayland window vanilla's client gets (framebuffer exactly
    // `--width`x`--height`; a Wayland window's is the desktop scale multiple).
    if args.x11 {
        std::env::set_var("WINIT_UNIX_BACKEND", "x11");
        std::env::remove_var("WAYLAND_DISPLAY");
    }
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let options = session.options.clone();
    let mut app = App {
        username: args.username.clone(),
        render_distance: args.render_distance,
        session: Some(session),
        menu: None,
        joining: None,
        local_server: None,
        // Present, not vsync: the windowed number must carry the present cost but not be
        // pinned to the display rate (vanilla's comparison run had vsync off too).
        vsync: args.vsync.unwrap_or(false),
        vsync_forced: args.vsync.or(Some(false)),
        options,
        options_version: 0,
        perspective: game::Perspective::FirstPerson,
        window: None,
        renderer: None,
        feed: WorldFeed::default(),
        ui: None,
        gui: GuiFrame::default(),
        input: InputRouter::default(),
        grabbed: false,
        screen_was_open: false,
        focused: true,
        breaking: Vec::new(),
        frame_stats: FrameStats::default(),
        fps: FpsCounter::new(),
        error: None,
        seconds: None,
        screenshot: false,
        scope: 0.5,
        frame_at: Instant::now(),
        started: Instant::now(),
        menu_next: Instant::now(),
        quitting: false,
        bench: Some(WindowBench {
            samples: Vec::with_capacity(seconds as usize * 1000),
            camera: None,
            view_distance: 12,
            seconds: seconds as f32,
            started: None,
            feed_ms: 0.0,
            size: (args.width, args.height),
            look: (args.yaw, args.pitch),
            want_pos: args
                .pos
                .as_ref()
                .filter(|v| v.len() == 3)
                .map(|v| DVec3::new(v[0], v[1], v[2])),
        }),
        weather: haste_render::WeatherFrame::default(),
        weather_cache: WeatherColumnCache::default(),
    };
    app.input.yaw = args.yaw.unwrap_or(0.0);
    app.input.pitch = args.pitch.unwrap_or(10.0);
    event_loop.run_app(&mut app)?;
    if let Some(e) = app.error {
        return Err(e);
    }
    let Some(bench) = app.bench else {
        anyhow::bail!("the windowed bench ended without samples");
    };
    let (width, height) = app
        .renderer
        .as_ref()
        .map_or((args.width, args.height), |r| r.size());
    let meta = BenchMeta {
        seconds: bench.seconds,
        render_distance: bench.view_distance,
        requested_render_distance: app.render_distance.unwrap_or(12),
        width,
        height,
        adapter: app
            .renderer
            .as_ref()
            .map(|r| {
                let info = r.adapter_info();
                format!("{} ({})", info.name, info.backend)
            })
            .unwrap_or_default(),
    };
    let summary = bench_summary(&bench.samples);
    println!("windowed bench at {}, {}", meta.width, meta.height);
    println!("{}", summary.text(&meta));
    if let Some(out) = &args.bench_out {
        let json = serde_json::json!({
            "windowed": true,
            "frames": summary.frames,
            "seconds": meta.seconds,
            "avg_ms": summary.avg_ms,
            "fps": summary.fps(),
            "p99_ms": summary.p99_ms,
            "worst_ms": summary.worst_ms,
            "sections_drawn": summary.sections_drawn,
            "sections_total": summary.sections_total,
            "draw_calls": summary.draw_calls,
            "indices_drawn": summary.indices_drawn,
            "prepare_ms": summary.prepare_ms,
            "acquire_ms": summary.acquire_ms,
            "cull_ms": summary.cull_ms,
            "encode_ms": summary.encode_ms,
            "submit_ms": summary.submit_ms,
            "present_ms": summary.present_ms,
            "gpu_main_ms": summary.gpu_ms[0],
            "gpu_hand_ms": summary.gpu_ms[1],
            "gpu_gui_ms": summary.gpu_ms[2],
            "render_distance": meta.render_distance,
            "requested_render_distance": meta.requested_render_distance,
            "width": meta.width,
            "height": meta.height,
            "adapter": meta.adapter,
        });
        std::fs::write(out, serde_json::to_string_pretty(&json)?)?;
        println!("bench summary written to {}", out.display());
    }
    Ok(())
}
