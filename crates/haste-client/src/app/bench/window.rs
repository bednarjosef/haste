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
    /// The camera path (`--bench-path`): the still settle camera, or the fixed turn and
    /// flight schedule (`turns`) the pre-mesh benchmark replays every run.
    pub(crate) path: BenchPath,
    /// `--pos`: the settle also waits until the server has teleported the player here.
    pub(crate) want_pos: Option<DVec3>,
    /// The wall time between one measured frame's end and the next one's: the whole frame
    /// (game view, GUI, feed, render, present), what vanilla's frame-to-frame time measures.
    /// A sample's `frame_ms` is only the renderer's part.
    pub(crate) intervals: Vec<f32>,
    pub(crate) last_frame: Option<Instant>,
}

impl WindowBench {
    /// Records one measured frame and the interval since the previous one; returns true
    /// on the frame that exhausts the measured seconds (the run's last). On that frame
    /// the memory table is printed while the session and renderer still exist — the
    /// window and both go away right after.
    pub(crate) fn record(
        &mut self,
        sample: BenchSample,
        session: &Session,
        renderer: &Renderer,
    ) -> bool {
        let now = Instant::now();
        if let Some(last) = self.last_frame {
            self.intervals.push((now - last).as_secs_f32() * 1000.0);
        }
        self.last_frame = Some(now);
        self.samples.push(sample);
        if self
            .started
            .is_some_and(|s| s.elapsed().as_secs_f32() >= self.seconds)
        {
            print_memory(session, renderer);
            true
        } else {
            false
        }
    }
}

/// The frame-to-frame interval statistics, in milliseconds: average, p50, p90, p99, p99.9 and
/// worst (the same fields as the vanilla recorder's `frametimes` step).
pub(super) fn interval_stats(intervals: &[f32]) -> [f32; 6] {
    let mut sorted = intervals.to_vec();
    sorted.sort_by(f32::total_cmp);
    let avg = sorted.iter().sum::<f32>() / sorted.len().max(1) as f32;
    let at = |p: f32| {
        sorted
            .get(((p * sorted.len() as f32) as usize).min(sorted.len().saturating_sub(1)))
            .copied()
            .unwrap_or(0.0)
    };
    [
        avg,
        at(0.5),
        at(0.9),
        at(0.99),
        at(0.999),
        sorted.last().copied().unwrap_or(0.0),
    ]
}

/// The memory table, the windowed twin of the offscreen bench's: printed when the measured
/// seconds elapse (from the frame loop, while the session and renderer still exist — the
/// window closes right after). Peak and settled RSS, the arenas' GPU buffers, the world's
/// chunk storage and the JVM's pools, so the memory-diet work has the numbers a real
/// window produces.
pub(crate) fn print_memory(session: &Session, renderer: &Renderer) {
    let (arenas, arena_bytes, arena_used, sections) = renderer.arena_memory();
    let cpu = renderer.cpu_memory();
    let world = session.shared.world.read().memory_stats();
    let jvm = session
        .shared
        .ui
        .lock()
        .as_ref()
        .map(|ui| ui.jvm_memory())
        .unwrap_or_default();
    let mib = memory::mib;
    println!(
        "bench memory: peak RSS {:.1} MiB, {arenas} arenas ({:.1} MiB vertex+index, {:.1} used), {sections} resident sections",
        mib(memory::peak_rss_kib() * 1024),
        mib(arena_bytes),
        mib(arena_used),
    );
    let (rss, anon, file) = memory::smaps_rollup_kib();
    println!(
        "bench memory: settled Rss {:.1} MiB (Pss_Anon {:.1}, Pss_File {:.1}), gpu resident {:.1} MiB",
        mib(rss * 1024),
        mib(anon * 1024),
        mib(file * 1024),
        mib(memory::gpu_resident_bytes()),
    );
    println!(
        "bench memory: world {:.1} MiB (sections {:.1}, heightmaps {:.1}, light {:.1}, block entities {:.1}, {} non-empty), atlas+models {:.1} MiB, mesher queue {}",
        mib(world.chunks + world.sections + world.heightmaps + world.light + world.block_entities),
        mib(world.sections),
        mib(world.heightmaps),
        mib(world.light),
        mib(world.block_entities),
        world.non_empty_sections,
        mib(cpu.atlas_bytes + cpu.baked_bytes),
        cpu.mesh_pending,
    );
    if !jvm.is_empty() {
        let mut heap_c = 0;
        let mut heap_u = 0;
        let mut meta = 0;
        let mut code = 0;
        for line in jvm.lines() {
            let (k, v) = line.split_once('=').unwrap_or(("", ""));
            let v: u64 = v.parse().unwrap_or(0);
            match k {
                "heap_committed" => heap_c = v,
                "heap_used" => heap_u = v,
                "metaspace_used" => meta = v,
                "code_cache_used" => code = v,
                _ => {}
            }
        }
        println!(
            "bench memory: JVM heap {:.1}/{:.1} MiB, metaspace {:.1} MiB, code cache {:.1} MiB",
            mib(heap_u),
            mib(heap_c),
            mib(meta),
            mib(code),
        );
    }
    memory::dump_smaps();
    memory::print_top_anon_mappings();
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
    let path = match args.bench_path.as_str() {
        "turns" => BenchPath::Turns,
        "still" => BenchPath::Still,
        other => anyhow::bail!("--bench-path must be `still` or `turns`, got `{other}`"),
    };
    // `--premesh` overrides the saved option for the run (the comparisons run one
    // binary with the mode switched).
    let premesh_forced = crate::app::frame::forced_premesh(args)?;
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
        premesh_forced,
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
        hitches: HitchTrace::new(),
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
            intervals: Vec::with_capacity(seconds as usize * 1000),
            last_frame: None,
            camera: None,
            view_distance: 12,
            seconds: seconds as f32,
            started: None,
            feed_ms: 0.0,
            size: (args.width, args.height),
            look: (args.yaw, args.pitch),
            path,
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
    let summary = bench_summary(
        &bench.samples,
        app.renderer
            .as_ref()
            .map_or((0, 0, 0), |r| r.premesh_summary()),
    );
    println!("windowed bench at {}, {}", meta.width, meta.height);
    println!("{}", summary.text(&meta));
    let [i_avg, i_p50, i_p90, i_p99, i_p999, i_worst] = interval_stats(&bench.intervals);
    println!(
        "frame interval (whole frame): avg {i_avg:.3} ms ({:.0} FPS), p50 {i_p50:.3}, p90 {i_p90:.3}, p99 {i_p99:.3}, p99.9 {i_p999:.3}, worst {i_worst:.3} ms",
        if i_avg > 0.0 { 1000.0 / i_avg } else { 0.0 }
    );
    if let Some(out) = &args.bench_out {
        let mut json = serde_json::json!({
            "windowed": true,
            "frames": summary.frames,
            "seconds": meta.seconds,
            "avg_ms": summary.avg_ms,
            "fps": summary.fps(),
            "p99_ms": summary.p99_ms,
            "worst_ms": summary.worst_ms,
            "interval_avg_ms": i_avg,
            "interval_fps": if i_avg > 0.0 { 1000.0 / i_avg } else { 0.0 },
            "interval_p50_ms": i_p50,
            "interval_p90_ms": i_p90,
            "interval_p99_ms": i_p99,
            "interval_p999_ms": i_p999,
            "interval_worst_ms": i_worst,
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
        // The pop-in, pre-mesh and memory numbers go in as separate keys: one giant
        // `json!` invocation overflows the macro recursion limit.
        let popin = summary.popin.stats();
        json["bench_path"] = match bench.path {
            BenchPath::Still => "still".into(),
            BenchPath::Turns => "turns".into(),
        };
        json["premesh"] = app
            .renderer
            .as_ref()
            .map_or("off", |r| match r.premesh_mode() {
                haste_render::PremeshMode::Off => "off",
                haste_render::PremeshMode::Cone => "cone",
                haste_render::PremeshMode::Predict => "predict",
            })
            .into();
        json["popin_episodes"] = popin.0.into();
        json["popin_episode_frames_avg"] = popin.1.into();
        json["popin_episode_frames_worst"] = popin.2.into();
        json["popin_frames_with_unmeshed"] = summary.popin.frames_with_unmeshed.into();
        json["visible_unmeshed_section_frames"] = summary.popin.unmeshed_section_frames.into();
        json["premeshed"] = summary.premeshed.into();
        json["premesh_hits"] = summary.premesh_hits.into();
        json["premesh_waste"] = summary.premesh_waste.into();
        json["arena_bytes"] = app
            .renderer
            .as_ref()
            .map_or(0, |r| r.arena_memory().1)
            .into();
        json["resident_sections"] = app
            .renderer
            .as_ref()
            .map_or(0, |r| r.arena_memory().3)
            .into();
        json["peak_rss_mib"] = memory::mib(memory::peak_rss_kib() * 1024).into();
        std::fs::write(out, serde_json::to_string_pretty(&json)?)?;
        println!("bench summary written to {}", out.display());
    }
    Ok(())
}
