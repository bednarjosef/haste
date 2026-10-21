//! The frame path (vanilla `Minecraft.runTick`'s render half): the per-frame `draw` from the
//! game view through the feed and the GUI to `Renderer::render` (the title bar's frame-time
//! statistics are in `fps_counter`). Split out of `app.rs` (CI keeps that file from growing);
//! a pure move.

use super::*;

mod hitch;
mod render_options;
use hitch::FramePhases;
pub(crate) use hitch::HitchTrace;
mod fps_counter;
pub(crate) use fps_counter::FpsCounter;
pub(crate) use render_options::{
    cloud_status, forced_premesh, parse_premesh, premesh_mode, work_budget_observation, work_feed,
};

/// Applies the config's cloud options to the renderer (vanilla `Options.cloudStatus`
/// and `Options.cloudRange`), and the world border the game view carries.
pub(super) fn apply_view_options(
    renderer: &mut haste_render::Renderer,
    options: &haste_setup::options::Options,
    world_border: Option<haste_render::world_border::WorldBorderFrame>,
) {
    renderer.set_cloud_status(cloud_status(options.clouds));
    renderer.set_cloud_range(options.cloud_range);
    renderer.set_world_border(world_border);
}

impl App {
    /// Menu mode's frame: the open screen over the title panorama (see `menu_background`).
    fn draw_menu(&mut self) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let (gui, camera, sky, partial) = match &mut self.ui {
            Some(ui) => menu_frame(ui, renderer, &self.input, &self.options),
            None => (GuiFrame::default(), menu_camera(), menu_sky(), 0.0),
        };
        self.gui = gui;
        let frame = FrameInput {
            camera,
            entities: &[],
            block_entities: &[],
            gui: &self.gui,
            sky,
            partial_tick: partial,
            block_target: None,
            breaking: &[],
            camera_entity: None,
            spin: None,
            weather: None,
        };
        if let Err(e) = renderer.render(&frame) {
            self.error = Some(e.context("render"));
        }
    }

    /// One frame: the game view, the world feed, the GUI and the render (vanilla
    /// `Minecraft.runTick`'s render half). Called from `about_to_wait` every poll and from
    /// `RedrawRequested`.
    pub(super) fn draw(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_none() {
            return;
        }
        // The resource pack selection changed (the pack screen's Done or a server
        // pack download, vanilla `Minecraft.reloadResourcePacks`): rebuild the
        // renderer from a fresh asset stack — atlas, models, fonts and GUI textures
        // re-read with the new packs layered on the jar.
        let reload = self
            .session
            .as_ref()
            .map(|s| s.shared.resource_pack_reload.lock().take())
            .unwrap_or_default();
        if let Some(reload) = reload {
            let window = self
                .window
                .clone()
                .expect("renderer exists, so does the window");
            match self.reload_renderer(&window, &reload) {
                Ok(()) => {
                    let names = reload
                        .ids
                        .iter()
                        .cloned()
                        .chain(reload.server.iter().map(|p| p.display().to_string()))
                        .collect::<Vec<_>>()
                        .join(", ");
                    tracing::info!("renderer rebuilt with resource packs [{names}]")
                }
                Err(e) => {
                    self.error = Some(e.context("resource pack reload"));
                    event_loop.exit();
                    return;
                }
            }
        }
        // The screen that opened or closed since the last frame takes or frees the cursor,
        // and "Disconnect" leaves the server (back to the multiplayer screen, like vanilla).
        self.sync_grab();
        self.take_disconnect(event_loop);
        if self.session.is_none() {
            // Menu mode: the screens tick here, the join the multiplayer screen requested
            // connects on its own thread, and the frame is the open screen over the plain
            // background (no world). The session arriving switches to the game path below.
            self.menu_tick();
            self.poll_join();
            self.draw_menu();
            return;
        }
        // The frame's view state: locked and copied once here, then passed to everything
        // below (frame_view, the feed, debug_info) instead of each taking the lock again.
        // The spyglass scope's easing reads its flag from the same copy.
        let t0 = Instant::now();
        let gap_ms = self.hitches.frame_started();
        let view = self
            .session
            .as_ref()
            .expect("checked above")
            .shared
            .view
            .lock()
            .clone();
        // The view lock's wait time, of the view phase below (t0 to the feed): contention
        // with the game thread's own view writes shows here first.
        let view_lock_ms = t0.elapsed().as_secs_f32() * 1e3;
        let scope = self.scope_scale(view.screen.scoping, t0);
        let session = self.session.as_ref().expect("checked above");
        // The options the player just changed in a vanilla screen: the game thread applied
        // them to `live_options` and bumped its version; copy them here so FOV, GUI scale and
        // sensitivity follow this frame (vanilla reads its Options object live too).
        let version = session.shared.options_version.load(Ordering::Relaxed);
        if version != self.options_version {
            self.options_version = version;
            self.options = session.shared.live_options.lock().clone();
            // The two options the window itself owns (vanilla's Fullscreen/Vsync callbacks touch
            // the window and the swapchain): apply them to this client's window and renderer.
            if let Some(w) = &self.window {
                let want = self.options.fullscreen;
                if w.fullscreen().is_some() != want {
                    w.set_fullscreen(want.then_some(Fullscreen::Borderless(None)));
                }
            }
            let want_vsync = self.vsync_forced.unwrap_or(self.options.vsync);
            if want_vsync != self.vsync {
                self.vsync = want_vsync;
                if let Some(r) = &mut self.renderer {
                    r.set_vsync(want_vsync);
                }
            }
        }
        self.input
            .sample(&session.shared, self.ui.as_ref(), self.perspective);
        // The windowed bench holds the camera still (the settle frame's view with the
        // requested look), like the offscreen `--bench --bench-fixed`; the `turns` path
        // instead drives the look from the run clock (the pre-mesh benchmark).
        if let Some(bench) = &mut self.bench {
            if let Some(yaw) = bench.look.0 {
                self.input.yaw = yaw - view.look_offset.0;
            }
            if let Some(pitch) = bench.look.1 {
                self.input.pitch = (pitch - view.look_offset.1).clamp(-90.0, 90.0);
            }
            if bench.path == BenchPath::Turns {
                if let (Some(started), Some(base)) = (bench.started, bench.camera.as_ref()) {
                    let turned =
                        bench_camera_turns(started.elapsed().as_secs_f32(), base, bench.seconds);
                    self.input.yaw = turned.yaw - view.look_offset.0;
                    self.input.pitch = (turned.pitch - view.look_offset.1).clamp(-90.0, 90.0);
                }
            }
        }
        // The work budget's per-frame observation (`--work-budget`): the interval since
        // the previous frame plus the previous frame's render and feed times.
        let (budget_interval_ms, budget_render_ms, budget_feed_ms) = work_budget_observation(
            &mut self.frame_at,
            self.frame_stats.frame_ms,
            self.feed.last_feed_ms(),
            t0,
        );
        let renderer = self.renderer.as_mut().expect("checked above");
        // The bench tracks pop-in (visible-but-unmeshed sections and pre-mesh verdicts);
        // enabling it before the settle phase seeds the meshed set with everything the
        // settle meshed, so the measured phase starts from zero.
        if self.bench.is_some() {
            renderer.set_popin_tracking(true);
        }
        let (mut camera, sky, lightmap, partial, vd, animation_tick, weather) = frame_view(
            &session.shared,
            &view,
            self.perspective,
            self.input.yaw,
            self.input.pitch,
            &self.options,
            t0,
            &mut self.weather,
            &mut self.weather_cache,
        );
        // The windowed bench's settle and still camera: until the world has settled (and
        // the HUD's JVM arrived, like the screenshot path), the frames only warm up; the
        // first settled frame's camera is then held for the whole run.
        if let Some(bench) = &mut self.bench {
            let settled_now = {
                let at_pos = bench
                    .want_pos
                    .is_none_or(|p| (view.feet - p).length() <= 0.5);
                view.has_position
                    && at_pos
                    && self.feed.settled(
                        &self.frame_stats,
                        Duration::from_millis(1500),
                        renderer,
                        camera_section(&camera),
                        vd,
                    )
                    && (self.ui.is_some()
                        || session.shared.hud_failed.load(Ordering::Relaxed)
                        || !session.shared.hud_enabled.load(Ordering::Relaxed))
            };
            if settled_now && bench.camera.is_none() {
                bench.camera = Some(camera);
                bench.view_distance = vd;
            }
            if let Some(base) = bench.camera {
                // The still camera is held; the `turns` path replays the fixed turn and
                // flight schedule from the run clock (same path every run).
                camera = match bench.path {
                    BenchPath::Still => base,
                    BenchPath::Turns => bench_camera_turns(
                        bench.started.map_or(0.0, |s| s.elapsed().as_secs_f32()),
                        &base,
                        bench.seconds,
                    ),
                };
            }
        }
        let feed_start = Instant::now();
        self.feed
            .feed(&session.shared, renderer, camera_section(&camera), vd);
        let feed_ms = feed_start.elapsed().as_secs_f32() * 1000.0;
        if let Some(bench) = &mut self.bench {
            bench.feed_ms = feed_ms;
        }
        let t1 = Instant::now();
        let hud_hidden = self.ui.as_ref().is_some_and(|ui| ui.hud_hidden());
        self.feed.set_first_person(
            &session.shared,
            renderer,
            self.perspective,
            hud_hidden,
            &view,
        );
        let (entities, block_entities) = self.feed.entities(
            &session.shared,
            renderer,
            session.local,
            partial,
            &view,
            matches!(self.perspective, game::Perspective::FirstPerson).then_some(camera),
        );
        let block_target = view.block_target;
        self.breaking.clear();
        self.breaking
            .extend_from_slice(&session.shared.breaking.lock());
        let t2 = Instant::now();
        // The HUD: once the game thread's `Ui` is ready, clone it here (cheap, one per thread)
        // and render the frame through it; until then (JVM still starting, or failed: no JDK
        // or cache) the empty default draws no HUD.
        if self.ui.is_none() {
            self.ui = session.shared.ui.lock().clone();
            if let Some(ui) = &self.ui {
                ui.set_text_metrics(renderer.text_metrics());
            }
        }
        let (w, h) = renderer.size();
        if let Some(ui) = &self.ui {
            // F3's numbers and the water overlay's flag, every frame (cheap: short mutexes,
            // no allocation while the overlay is off). The section counts are the previous
            // frame's.
            let stats = self.frame_stats;
            let gpu = gpu_info(renderer);
            ui.set_debug(debug_info(
                &session.debug_env,
                &session.shared,
                renderer,
                &view,
                (self.input.yaw, self.input.pitch),
                self.fps.fps as i32,
                &stats,
                entities.len() as i32,
                resolve_gui_scale(
                    u32::from(self.options.gui_scale),
                    w,
                    h,
                    self.options.force_unicode_font,
                ),
                gpu,
            ));
            ui.set_eye_in_water(view.eye_in_water);
        }
        if let Some(ui) = &mut self.ui {
            let mouse = self.input.mouse;
            let scale = resolve_gui_scale(
                u32::from(self.options.gui_scale),
                w,
                h,
                self.options.force_unicode_font,
            );
            let (gw, gh) = gui_scaled_size(w, h, scale);
            self.gui = ui.take_frame(gw, gh, scale, mouse, partial);
            renderer.set_panorama(ui.gui().panorama());
        }
        let t_gui = Instant::now();
        // The screen effects around the HUD frame (vanilla's `ScreenEffectRenderer.submit`'s
        // level-pass overlays and `Hud.extractCameraOverlays`'s own).
        {
            let frame = crate::screen_effects::Frame {
                width: self.gui.width,
                height: self.gui.height,
                first_person: !self.perspective.draws_player(),
                effects: &view.screen,
                partial_tick: partial,
                // The look the frame's camera uses, which the water overlay scrolls by.
                look: (
                    self.input.yaw + view.look_offset.0,
                    (self.input.pitch + view.look_offset.1).clamp(-90.0, 90.0),
                ),
                scope_scale: scope,
            };
            frame.append_to(&mut self.gui.commands);
        }
        let t_pre = Instant::now();
        let frame = FrameInput {
            camera,
            entities,
            block_entities,
            gui: &self.gui,
            sky,
            partial_tick: partial,
            block_target,
            breaking: &self.breaking,
            camera_entity: game::camera_entity::hidden_entity(
                &view,
                !self.perspective.draws_player(),
                session.local.0,
            ),
            spin: view.screen.spin_warp(partial),
            weather,
        };
        // Outline boxes of the target and the states of breaking blocks (one short read lock).
        let session = self.session.as_ref().expect("the game path above");
        renderer.set_camera_spectator(view.spectator);
        renderer.sample_world(
            &*session.shared.world.read(),
            &session.shared.data.shapes,
            &frame,
        );
        {
            let snapshot = session
                .shared
                .particle_snapshots
                .take()
                .unwrap_or_else(|| Arc::new(Default::default()));
            renderer.prepare_particles(&snapshot, camera, partial, frame.spin);
        }
        renderer.set_animation_tick(animation_tick);
        renderer.set_game_time(view.world_age);
        renderer.set_lightmap_frame(lightmap);
        renderer.set_menu_blur_radius(self.options.menu_background_blurriness);
        renderer.set_view_scale(self.options.entity_distance_scaling);
        apply_view_options(renderer, &self.options, view.world_border);
        renderer.set_premesh(
            self.premesh_forced
                .unwrap_or_else(|| premesh_mode(self.options.premesh)),
        );
        renderer.note_frame(budget_interval_ms, budget_render_ms, budget_feed_ms);
        if self.screenshot {
            self.screenshot = false;
            renderer.request_capture();
        }
        let t_render = Instant::now();
        let result = renderer.render(&frame);
        if let Ok(stats) = &result {
            self.frame_stats = *stats;
        }
        // The windowed bench's sample: recorded once the still camera is held (the settle
        // frames before it are warm-up, not measurements). The run ends with the window.
        if let (Some(bench), Ok(stats)) = (&mut self.bench, &result) {
            if bench.started.is_some() {
                let prev_drawn = bench.samples.last().map_or(0, |s| s.sections_drawn);
                let sample = BenchSample {
                    frame_ms: stats.frame_ms,
                    sections_drawn: stats.chunk_sections_drawn,
                    sections_total: stats.chunk_sections_total,
                    draw_calls: stats.draw_calls,
                    timings: renderer.timings(),
                    feed_ms: bench.feed_ms,
                    sections_new: stats.chunk_sections_drawn.saturating_sub(prev_drawn),
                    uploads: stats.uploads,
                    upload_bytes: stats.upload_bytes,
                    upload_budget_ms: stats.upload_budget_ms,
                    indices_drawn: stats.indices_drawn,
                    index: bench.samples.len() as u32,
                    visible_unmeshed: stats.visible_unmeshed,
                };
                self.quitting |= bench.record(sample, session, renderer);
            } else if bench.camera.is_some() {
                bench.started = Some(Instant::now());
            }
        }
        if let Some(result) = renderer.take_capture() {
            // Encode and write off the render thread: PNG compression of a full frame
            // would stall the frame loop (vanilla's chat line becomes a log line here).
            match result {
                Ok(image) => {
                    std::thread::spawn(move || match crate::screenshots::save(image) {
                        Ok(name) => tracing::info!("Saved screenshot as {name}"),
                        Err(e) => tracing::warn!("screenshot failed: {e:#}"),
                    });
                }
                Err(e) => tracing::warn!("screenshot readback failed: {e:#}"),
            }
        }
        let t3 = Instant::now();
        self.fps.phases(
            (t1 - t0).as_secs_f32() * 1e3,
            (t2 - t1).as_secs_f32() * 1e3,
            (t3 - t2).as_secs_f32() * 1e3,
            renderer.timings(),
        );
        match result {
            Ok(stats) => {
                if let Some(line) = self.fps.frame(&stats) {
                    if let Some(b) = self.fps.breakdown(renderer) {
                        tracing::info!("{b}");
                    }
                    if let Some(w) = &self.window {
                        w.set_title(&format!("Haste - {line}"));
                    }
                    tracing::info!("{line}");
                }
            }
            Err(e) => {
                self.error = Some(e.context("render"));
                event_loop.exit();
            }
        }
        self.hitches.frame_ended(
            gap_ms,
            FramePhases {
                view_lock: view_lock_ms,
                view: (feed_start - t0).as_secs_f32() * 1e3,
                feed: feed_ms,
                entities: (t2 - t1).as_secs_f32() * 1e3,
                gui: (t_gui - t2).as_secs_f32() * 1e3,
                pre: (t_pre - t_gui).as_secs_f32() * 1e3,
                render: (t3 - t_render).as_secs_f32() * 1e3,
                post: t3.elapsed().as_secs_f32() * 1e3,
            },
        );
    }
    /// Rebuilds the renderer after the resource pack selection changed (vanilla's reload
    /// rebuilds its resource manager, model manager and atlases). The new selection is
    /// written to options.txt by the JVM's own `Options.save` before we get here, so
    /// `open_assets` reads it back; the server's downloaded packs layer above it.
    fn reload_renderer(
        &mut self,
        window: &Arc<Window>,
        reload: &game::PackReload,
    ) -> anyhow::Result<()> {
        let mut assets = open_assets_with(&reload.server)?;
        let data = net::load_game_data()?;
        let mut renderer = Renderer::new(window.clone(), &mut assets, &data.blocks)?;
        if let Some(session) = &self.session {
            renderer.set_item_registry(&session.shared.data.registries);
            renderer.set_biomes(&session.biomes);
            renderer.set_particle_types(&session.particle_types);
            renderer.set_dimension(session.shared.world.read().dimension());
        }
        renderer.set_vsync(self.vsync);
        // The old renderer's GPU objects drop with it; every loaded section is queued for
        // re-meshing into the new arenas (vanilla's reload re-bakes the world's models too).
        self.renderer = Some(renderer);
        self.feed = WorldFeed::default();
        if let Some(session) = &self.session {
            self.feed.queue_all_sections(&session.shared);
        }
        Ok(())
    }
}
