//! `--bench`: the offscreen benchmark along the fixed camera path.

use super::*;

/// `--bench SECONDS`: join, wait until the world settles (screenshot's logic), then render
/// offscreen at `--width`x`--height` along the fixed `bench_camera` path for SECONDS while
/// recording per-frame `FrameStats` and `FrameTimings`. Prints one summary block, writes the
/// same as JSON to `--bench-out` when given, and disconnects.
pub fn bench(args: &Args, seconds: u64) -> anyhow::Result<()> {
    let Some(server) = args.server.as_deref() else {
        anyhow::bail!("--bench needs --server <host[:port]>");
    };
    let session = Session::join(&args.username, args.render_distance, server, false, None)?;
    let shared = session.shared.clone();
    // The bench camera is first person: the game thread's camera effects take that perspective.
    shared.input.lock().perspective = game::Perspective::FirstPerson;
    let mut assets = open_assets_with(&session.server_packs)?;
    let mut renderer =
        Renderer::new_offscreen(args.width, args.height, &mut assets, &shared.data.blocks)?;
    renderer.set_item_registry(&shared.data.registries);
    renderer.set_shapes(&shared.data.blocks, &shared.data.shapes);
    renderer.set_biomes(&session.biomes);
    renderer.set_particle_types(&session.particle_types);
    renderer.set_dimension(shared.world.read().dimension());
    let gui = GuiFrame::default();
    let mut feed = WorldFeed::default();
    // The render thread's weather extraction buffer and per-tick column cache (both
    // reused, no allocation after warmup).
    let mut weather_frame = haste_render::WeatherFrame::default();
    let mut weather_cache = WeatherColumnCache::default();
    let deadline = Instant::now() + Duration::from_secs(args.seconds);
    let mut stats;
    let mut base;
    let mut vd;
    // Settle phase: identical to the screenshot path (wait for position, meshing done, quiet).
    loop {
        let view = shared.view.lock().clone();
        let (mut camera, sky, lightmap, partial, v, animation_tick, weather) = frame_view(
            &shared,
            &view,
            game::Perspective::FirstPerson,
            0.0,
            0.0,
            &session.options,
            Instant::now(),
            &mut weather_frame,
            &mut weather_cache,
        );
        camera.yaw = args.yaw.unwrap_or(camera.yaw);
        camera.pitch = args.pitch.unwrap_or(camera.pitch);
        base = camera;
        vd = v;
        feed.feed(&shared, &mut renderer, camera_section(&camera), vd);
        feed.set_first_person(
            &shared,
            &mut renderer,
            game::Perspective::FirstPerson,
            false,
            view.first_person,
        );
        let (entities, block_entities) =
            feed.entities(&shared, &mut renderer, session.local, partial, &view);
        let block_target = view.block_target;
        let breaking = shared.breaking.lock().clone();
        let frame = FrameInput {
            camera,
            entities,
            block_entities,
            gui: &gui,
            sky,
            partial_tick: partial,
            block_target,
            breaking: &breaking,
            camera_entity: Some(session.local.0),
            spin: None,
            weather,
        };
        renderer.set_camera_spectator(view.spectator);
        renderer.sample_world(&*shared.world.read(), &shared.data.shapes, &frame);
        renderer.set_animation_tick(animation_tick);
        renderer.set_game_time(view.world_age);
        renderer.set_lightmap_frame(lightmap);
        renderer.set_cloud_status(crate::app::frame::cloud_status(session.options.clouds));
        {
            let snapshot = shared
                .particle_snapshots
                .take()
                .unwrap_or_else(|| Arc::new(Default::default()));
            renderer.prepare_particles(&snapshot, camera, partial, None);
        }
        stats = renderer.render(&frame)?;
        if shared.is_closed() {
            anyhow::bail!(
                "disconnected: {}",
                shared.closed.lock().clone().unwrap_or_default()
            );
        }
        let placed = view.has_position;
        // `--pos` holds the settle until the server has teleported the player there (the
        // screenshot path's rule), so the bench measures the intended spot, not the spawn.
        let reached = args
            .pos
            .as_ref()
            .filter(|v| v.len() == 3)
            .map(|v| {
                let want = DVec3::new(v[0], v[1], v[2]);
                placed && (view.feet - want).length() <= 0.5
            })
            .unwrap_or(placed);
        if reached
            && feed.settled(
                &stats,
                Duration::from_millis(1500),
                &renderer,
                camera_section(&camera),
                vd,
            )
        {
            break;
        }
        if Instant::now() > deadline {
            tracing::warn!("--seconds elapsed before the world settled; benching what is there");
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // Measure phase. The buffer is sized up front: at most 1000 fps for the whole run, and it
    // grows once if the machine is faster than that (no allocation during steady-state frames).
    {
        // Memory diagnostics: peak RSS plus every large allocation we can attribute —
        // terrain arenas (allocated vs used), the world's chunk storage, the renderer's
        // CPU-side copies and the embedded JVM's pools — so the diet work has a table.
        let (arenas, arena_bytes, arena_used, sections) = renderer.arena_memory();
        let cpu = renderer.cpu_memory();
        let world = shared.world.read().memory_stats();
        let jvm = shared
            .ui
            .lock()
            .as_ref()
            .map(|ui| ui.jvm_memory())
            .unwrap_or_default();
        let mib = memory::mib;
        let rss_kb = memory::peak_rss_kib();
        // smaps_rollup for the live (settled) picture: anonymous vs file-backed pages.
        let rollup = memory::smaps_rollup_kib();
        println!(
            "bench memory: peak RSS {rss_kb} kB ({:.1} MiB), {arenas} arenas ({:.1} MiB vertex+index, {:.1} used), {sections} resident sections",
            mib(rss_kb * 1024),
            mib(arena_bytes),
            mib(arena_used),
        );
        println!(
            "bench memory: settled Rss {:.1} MiB (Pss_Anon {:.1}, Pss_File {:.1}), gpu resident {:.1} MiB",
            mib(rollup.0 * 1024),
            mib(rollup.1 * 1024),
            mib(rollup.2 * 1024),
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
    }
    let mut samples: Vec<BenchSample> = Vec::with_capacity(seconds as usize * 1000);
    let total = seconds as f32;
    let adapter = {
        let info = renderer.adapter_info();
        format!("{} ({})", info.name, info.backend)
    };
    let meta = BenchMeta {
        seconds: total,
        render_distance: vd,
        requested_render_distance: session.options.render_distance,
        width: args.width,
        height: args.height,
        adapter,
    };
    let started = Instant::now();
    loop {
        let elapsed = started.elapsed().as_secs_f32();
        if elapsed >= total {
            break;
        }
        // The bench camera follows its fixed path; the sky and entities come from the live view
        // (time of day moves, the player stays where the server put it).
        let view = shared.view.lock().clone();
        let (_, mut sky, lightmap, partial_tick, _, animation_tick, weather) = frame_view(
            &shared,
            &view,
            game::Perspective::FirstPerson,
            0.0,
            0.0,
            &session.options,
            Instant::now(),
            &mut weather_frame,
            &mut weather_cache,
        );
        let camera = if args.bench_fixed {
            // Still camera at the settle spot: one fixed view, comparable with `--screenshot`.
            base
        } else {
            bench_camera(elapsed / total, &base, total)
        };
        let fog_end = (vd.max(2) * 16) as f32;
        sky.fog_start = fog_end * 0.75;
        sky.fog_end = fog_end;
        let feed_start = Instant::now();
        feed.feed(&shared, &mut renderer, camera_section(&camera), vd);
        let feed_ms = feed_start.elapsed().as_secs_f32() * 1000.0;
        let (entities, block_entities) =
            feed.entities(&shared, &mut renderer, session.local, partial_tick, &view);
        let block_target = view.block_target;
        let breaking = shared.breaking.lock().clone();
        let frame = FrameInput {
            camera,
            entities,
            block_entities,
            gui: &gui,
            sky,
            partial_tick,
            block_target,
            breaking: &breaking,
            camera_entity: Some(session.local.0),
            spin: None,
            weather,
        };
        renderer.set_camera_spectator(view.spectator);
        renderer.sample_world(&*shared.world.read(), &shared.data.shapes, &frame);
        renderer.set_animation_tick(animation_tick);
        renderer.set_game_time(view.world_age);
        renderer.set_lightmap_frame(lightmap);
        renderer.set_cloud_status(crate::app::frame::cloud_status(session.options.clouds));
        {
            let snapshot = shared
                .particle_snapshots
                .take()
                .unwrap_or_else(|| Arc::new(Default::default()));
            renderer.prepare_particles(&snapshot, camera, partial_tick, None);
        }
        let stats = renderer.render(&frame)?;
        let prev_drawn = samples.last().map_or(0, |s| s.sections_drawn);
        samples.push(BenchSample {
            frame_ms: stats.frame_ms,
            sections_drawn: stats.chunk_sections_drawn,
            sections_total: stats.chunk_sections_total,
            draw_calls: stats.draw_calls,
            timings: renderer.timings(),
            feed_ms,
            sections_new: stats.chunk_sections_drawn.saturating_sub(prev_drawn),
            uploads: stats.uploads,
            upload_bytes: stats.upload_bytes,
            indices_drawn: stats.indices_drawn,
            index: samples.len() as u32,
        });
        if shared.is_closed() {
            anyhow::bail!(
                "disconnected during the bench: {}",
                shared.closed.lock().clone().unwrap_or_default()
            );
        }
    }
    let summary = bench_summary(&samples);
    println!("{}", summary.text(&meta));
    print_slowest_frames(&samples);
    if let Some(out) = &args.bench_out {
        let json = serde_json::json!({
            "frames": summary.frames,
            "seconds": total,
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
            "gpu_main_sub_ms": {
                "sky": summary.gpu_sub_ms[0],
                "opaque": summary.gpu_sub_ms[1],
                "cutout": summary.gpu_sub_ms[2],
                "translucent": summary.gpu_sub_ms[3],
                "entities": summary.gpu_sub_ms[4],
                "particles": summary.gpu_sub_ms[5],
                "overlays": summary.gpu_sub_ms[6],
            },
            "render_distance": meta.render_distance,
            "requested_render_distance": meta.requested_render_distance,
            "width": meta.width,
            "height": meta.height,
            "adapter": meta.adapter,
        });
        std::fs::write(out, serde_json::to_string_pretty(&json)?)?;
        println!("bench summary written to {}", out.display());
    }
    drop(session);
    Ok(())
}
