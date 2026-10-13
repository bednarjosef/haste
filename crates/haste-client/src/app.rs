//! Windowed client and screenshot mode (client-app).
//!
//! `haste --server 127.0.0.1:25611` joins, starts the game thread (`game::run`, 20 Hz) and runs
//! the winit event loop on the main thread: each frame it feeds dirty sections to the renderer
//! (`WorldFeed`), copies the interpolated entity instances, builds the camera from the
//! interpolated player position and renders. `--screenshot <file>` renders the same frame
//! offscreen once the world has finished meshing, saves it and exits.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use glam::DVec3;
use haste_protocol::haste_core::gui::GuiFrame;
use haste_protocol::haste_core::input::InputState;
use haste_protocol::haste_core::model::EntityInstance;
use haste_protocol::haste_core::{ChunkPos, SectionPos};
use haste_protocol::{Account, ConnectOptions};
use haste_render::{Camera, FrameInput, FrameStats, Renderer, SkyState};
use haste_setup::options::Options;
use haste_world::World;
use winit::application::ApplicationHandler;
use winit::event::{
    DeviceEvent, DeviceId, ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Fullscreen, Window, WindowId};

// Declared here rather than in main.rs (lead-owned); the lead may move it.
#[path = "biomes.rs"]
mod biomes;

use crate::game::{self, Shared};
use crate::net::{self, NetDriver};
use crate::Args;

/// Saved options this run uses: FOV, render distance, mouse sensitivity and (unless the
/// `--vsync` flag says otherwise) vsync. `Config::load` creates the file with defaults on
/// first run, so the fallback only fires on a broken file.
fn load_options() -> haste_setup::options::Options {
    match haste_setup::config::Config::load() {
        Ok(c) => c.options,
        Err(e) => {
            tracing::warn!("config load failed, using default options: {e:#}");
            Options::default()
        }
    }
}

/// Degrees of rotation per pixel of mouse motion at sensitivity 0..=1. Vanilla's curve, as
/// the client community documents it (minecraft.wiki "Options" > Sensitivity documents the
/// 0..=1 range): `f = s * 0.6 + 0.2`, `f * f * f * 8 * 0.15` degrees per pixel.
pub fn look_deg_per_pixel(sensitivity: f32) -> f32 {
    let f = sensitivity * 0.6 + 0.2;
    f * f * f * 8.0 * 0.15
}
/// Two jump presses closer than this toggle flight (vanilla: 7 ticks).
const DOUBLE_TAP: Duration = Duration::from_millis(350);
/// Wall time the render thread may spend capturing sections for the mesher per frame.
/// Sections handed to the mesh workers per frame (they capture the snapshots themselves).
const SUBMIT_PER_FRAME: usize = 512;

/// A joined game: runtime, game thread and shared state.
struct Session {
    /// Kept alive for the connection's tasks; dropped after the game thread.
    _rt: tokio::runtime::Runtime,
    shared: Arc<Shared>,
    game_thread: Option<std::thread::JoinHandle<()>>,
    /// Biome colours of the synced biome registry (index = biome id), for the renderer.
    biomes: Vec<haste_render::BiomeClimate>,
    /// The saved options this run uses (loaded once).
    options: Options,
}

impl Session {
    fn join(args: &Args) -> anyhow::Result<Self> {
        let address = args.server.clone().ok_or_else(|| {
            anyhow::anyhow!("--server <host[:port]> is required (no launcher yet)")
        })?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .thread_name("haste-net")
            .build()?;
        let data = net::load_game_data()?;
        let options = load_options();
        let mut opts = ConnectOptions::new(
            address.clone(),
            Account::Offline {
                username: args.username.clone(),
            },
        );
        // The saved render distance is what we advertise in client_information; the server's
        // own view distance (login packet) caps what we mesh below.
        opts.client_information.view_distance = options.render_distance as i8;
        let driver: NetDriver = rt.block_on(NetDriver::connect(opts))?;
        let regs = driver.connection().registries();
        let biomes = net::synced_biome_count(regs).unwrap_or(64);
        let biome_climates = biomes::biome_climates(regs);
        let login = driver.login();
        tracing::info!(
            "joined {address} as {}: {:?}, dimension {}, view distance {}",
            driver.connection().profile().name,
            login.game_mode,
            login.dimension.name,
            login.view_distance
        );
        let world = World::new_with_biomes(login.dimension.clone(), data.clone(), biomes);
        let entity_types = data.registries.len("minecraft:entity_type");
        let view_distance = login.view_distance.min(options.render_distance as i32);
        let shared = Arc::new(Shared::new(
            data,
            world,
            net::new_entity_store(entity_types),
            view_distance,
        ));
        let handle = rt.handle().clone();
        let sh = shared.clone();
        let game_thread = std::thread::Builder::new()
            .name("haste-game".into())
            .spawn(move || {
                let _rt = handle.enter();
                game::run(driver, sh)
            })?;
        Ok(Self {
            _rt: rt,
            shared,
            game_thread: Some(game_thread),
            biomes: biome_climates,
            options,
        })
    }

    /// Stops the game thread (it closes the connection) and lets the writer flush.
    fn leave(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.game_thread.take() {
            let _ = t.join();
        }
        // The runtime's worker threads flush the writer meanwhile.
        std::thread::sleep(Duration::from_millis(100));
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.leave();
    }
}

/// Feeds dirty sections to the renderer within a per-frame time budget, nearest first, and
/// skips sections that are empty and were never meshed (nothing to draw, nothing to clear).
#[derive(Default)]
pub struct WorldFeed {
    /// Sections waiting, sorted so the nearest is last (`pop`).
    queue: Vec<SectionPos>,
    queued: HashSet<SectionPos>,
    /// Sections whose last submitted mesh was non-empty.
    meshed: HashSet<SectionPos>,
    sort_needed: bool,
    last_arrival: Option<Instant>,
    /// Entity instances for this frame (reused).
    instances: Vec<EntityInstance>,
    /// The shared world as the mesh workers see it (created on the first feed).
    source: Option<Arc<dyn haste_render::SnapshotSource>>,
    /// Sections submitted this frame (reused).
    batch: Vec<SectionPos>,
}

/// Lets the mesh worker threads capture section snapshots from the shared world under its
/// read lock, so the render thread never copies blocks.
struct SharedWorldSource(Arc<Shared>);

impl haste_render::SnapshotSource for SharedWorldSource {
    fn capture(&self, pos: SectionPos, blend_radius: i32) -> Option<haste_render::MeshSnapshot> {
        let world = self.0.world.read();
        world.chunk(pos.chunk())?;
        Some(haste_render::MeshSnapshot::capture_with_biomes(
            &*world,
            Some(&biomes::WorldBiomes(&world)),
            blend_radius,
            pos,
        ))
    }
}

impl WorldFeed {
    /// Pulls new dirty sections from the game thread.
    fn collect(&mut self, shared: &Shared) {
        let mut dirty = shared.dirty.lock();
        if dirty.is_empty() {
            return;
        }
        for p in dirty.drain(..) {
            if self.queued.insert(p) {
                self.queue.push(p);
            }
        }
        self.sort_needed = true;
        self.last_arrival = Some(Instant::now());
    }

    /// Hands queued sections (nearest first) to the mesh workers, which capture and mesh them
    /// off the render thread, and lets the renderer upload finished meshes.
    fn feed(
        &mut self,
        shared: &Arc<Shared>,
        renderer: &mut Renderer,
        camera: SectionPos,
        view_distance: i32,
    ) {
        self.collect(shared);
        if self.sort_needed {
            let d = |p: &SectionPos| {
                let (dx, dy, dz) = (p.x - camera.x, p.y - camera.y, p.z - camera.z);
                dx * dx + dy * dy + dz * dz
            };
            self.queue.sort_unstable_by_key(|p| std::cmp::Reverse(d(p)));
            self.sort_needed = false;
        }
        let camera_chunk = ChunkPos::new(camera.x, camera.z);
        let source = self
            .source
            .get_or_insert_with(|| Arc::new(SharedWorldSource(shared.clone())))
            .clone();
        let mut batch = std::mem::take(&mut self.batch);
        batch.clear();
        {
            let world = shared.world.read();
            while batch.len() < SUBMIT_PER_FRAME {
                let Some(pos) = self.queue.pop() else { break };
                self.queued.remove(&pos);
                let non_empty = world
                    .chunk(pos.chunk())
                    .and_then(|c| c.section(pos.y))
                    .is_some_and(|s| !s.is_empty());
                if !non_empty && !self.meshed.contains(&pos) {
                    continue;
                }
                if non_empty {
                    self.meshed.insert(pos);
                } else {
                    self.meshed.remove(&pos);
                }
                batch.push(pos);
            }
        }
        // Also uploads meshes that finished since the last frame.
        renderer.update_world_from(&source, &batch, camera_chunk, view_distance);
        self.batch = batch;
    }

    /// Copies the interpolated entity instances.
    /// Copies the interpolated entity instances, gives players their default skin and samples
    /// each entity's light for the renderer.
    fn entities(
        &mut self,
        shared: &Shared,
        renderer: &mut Renderer,
        partial: f32,
    ) -> &[EntityInstance] {
        self.instances.clear();
        {
            let store = shared.entities.read();
            store.render_instances(partial, &mut self.instances);
            for inst in &mut self.instances {
                if inst.skin.is_none() {
                    if let Some(e) = store.get(inst.id).filter(|e| e.is_player_type) {
                        // The slim-arm flag is not used: haste-entity's humanoid model is wide.
                        inst.skin = Some(haste_render::default_skin(e.uuid.as_u128()).0);
                    }
                }
            }
        }
        renderer.sample_entity_lights(&*shared.world.read(), &self.instances);
        &self.instances
    }

    /// Whether everything the game thread produced has been meshed and uploaded, and nothing
    /// new arrived for `quiet`.
    ///
    /// `meshes_pending` (pending + queued results + in-flight) already guarantees every
    /// submitted section was meshed and uploaded; comparing the renderer's resident-section
    /// count against a client-side count cannot work, because a section with blocks can still
    /// mesh to zero vertices (fully enclosed) and the renderer then keeps no entry for it.
    fn settled(&self, stats: &FrameStats, quiet: Duration) -> bool {
        self.queue.is_empty()
            && stats.meshes_pending == 0
            && self.last_arrival.is_some_and(|t| t.elapsed() >= quiet)
    }
}

/// Builds this frame's camera and sky from the shared view at `now`. `yaw`/`pitch` are the
/// input layer's look; the player's server rotation offset is added here so mouse look is
/// smooth at frame rate while the player itself turns once per tick.
fn frame_view(
    shared: &Shared,
    yaw: f32,
    pitch: f32,
    fov_deg: f32,
    now: Instant,
) -> (Camera, SkyState, f32, i32) {
    let view = *shared.view.lock();
    let partial = game::partial_tick(view.tick_at, now);
    let camera = Camera {
        // Linear between LocalPlayer::camera(0.0) and camera(1.0), i.e. camera(partial).
        pos: game::interpolate(view.prev_eye, view.eye, partial),
        yaw: yaw + view.look_offset.0,
        pitch: (pitch + view.look_offset.1).clamp(-90.0, 90.0),
        fov_deg,
    };
    let fog_end = (view.view_distance.max(2) * 16) as f32;
    let sky = SkyState {
        time_of_day: view.time_of_day,
        rain: 0.0,
        dimension_has_sky: view.has_sky,
        fog_start: fog_end * 0.75,
        fog_end,
    };
    (camera, sky, partial, view.view_distance)
}

/// Number keys 1-9 select hotbar slots 0-8.
fn hotbar_key(code: KeyCode) -> Option<u8> {
    Some(match code {
        KeyCode::Digit1 => 0,
        KeyCode::Digit2 => 1,
        KeyCode::Digit3 => 2,
        KeyCode::Digit4 => 3,
        KeyCode::Digit5 => 4,
        KeyCode::Digit6 => 5,
        KeyCode::Digit7 => 6,
        KeyCode::Digit8 => 7,
        KeyCode::Digit9 => 8,
        _ => return None,
    })
}

fn camera_section(c: &Camera) -> SectionPos {
    let p = c.pos.floor().as_ivec3();
    SectionPos::new(p.x >> 4, p.y >> 4, p.z >> 4)
}

fn open_assets() -> anyhow::Result<haste_assets::Assets> {
    let jar = haste_protocol::haste_core::paths::client_jar();
    haste_assets::Assets::open(
        &jar,
        &haste_protocol::haste_core::paths::assets_dir(),
        "26.3",
    )
    .with_context(|| format!("opening assets from {}", jar.display()))
}

/// `--screenshot <file>`: join, wait until the world is meshed, render one frame offscreen,
/// save it as PNG and leave. `--seconds` bounds the wait.
pub fn screenshot(args: &Args, file: &Path) -> anyhow::Result<()> {
    let session = Session::join(args)?;
    let shared = session.shared.clone();
    let mut assets = open_assets()?;
    let mut renderer =
        Renderer::new_offscreen(args.width, args.height, &mut assets, &shared.data.blocks)?;
    renderer.set_item_registry(&shared.data.registries);
    renderer.set_biomes(&session.biomes);
    let gui = GuiFrame::default();
    let mut feed = WorldFeed::default();
    let deadline = Instant::now() + Duration::from_secs(args.seconds);
    // With `--pos`, the shot is only taken once the server has teleported the player to these
    // feet coordinates (the compare script teleports over RCON while we join).
    let want_pos = args
        .pos
        .as_ref()
        .filter(|v| v.len() == 3)
        .map(|v| DVec3::new(v[0], v[1], v[2]));
    let mut frames = 0u32;
    let mut stats;
    let mut last_camera;
    loop {
        let (mut camera, sky, partial, vd) = frame_view(
            &shared,
            0.0,
            0.0,
            session.options.fov as f32,
            Instant::now(),
        );
        camera.yaw = args.yaw.unwrap_or(camera.yaw);
        camera.pitch = args.pitch.unwrap_or(camera.pitch);
        if let Some(pos) = want_pos {
            let view = *shared.view.lock();
            camera.pos = pos + DVec3::new(0.0, view.eye_height, 0.0);
        }
        last_camera = camera;
        {
            // The player looks where the screenshot camera looks, so its crosshair target
            // (and the outline drawn for it) matches the picture.
            let offset = shared.view.lock().look_offset;
            let mut input = shared.input.lock();
            input.state.yaw = camera.yaw - offset.0;
            input.state.pitch = camera.pitch - offset.1;
        }
        feed.feed(&shared, &mut renderer, camera_section(&camera), vd);
        let entities = feed.entities(&shared, &mut renderer, partial);
        let block_target = shared.view.lock().block_target;
        let breaking = shared.breaking.lock().clone();
        let frame = FrameInput {
            camera,
            entities,
            gui: &gui,
            sky,
            partial_tick: partial,
            block_target,
            breaking: &breaking,
        };
        renderer.sample_world(&*shared.world.read(), &shared.data.shapes, &frame);
        stats = renderer.render(&frame)?;
        frames += 1;
        let placed = shared.view.lock().has_position;
        if shared.is_closed() {
            anyhow::bail!(
                "disconnected: {}",
                shared.closed.lock().clone().unwrap_or_default()
            );
        }
        if let Some(pos) = want_pos {
            let view = *shared.view.lock();
            let reached = placed && (view.feet - pos).length() <= 0.5;
            if !reached {
                if Instant::now() > deadline {
                    let c = last_camera;
                    anyhow::bail!(
                        "--seconds elapsed before reaching --pos; player at ({:.2}, {:.2}, {:.2}), camera would have been at ({:.2}, {:.2}, {:.2})",
                        view.feet.x,
                        view.feet.y,
                        view.feet.z,
                        c.pos.x,
                        c.pos.y,
                        c.pos.z
                    );
                }
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
        }
        if placed && feed.settled(&stats, Duration::from_millis(1500)) {
            break;
        }
        if Instant::now() > deadline {
            tracing::warn!("--seconds elapsed before the world settled; saving what is there");
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let image = renderer.read_back()?;
    image
        .save(file)
        .with_context(|| format!("saving {}", file.display()))?;
    let c = last_camera;
    let view = *shared.view.lock();
    println!(
        "screenshot {} ({}x{}) after {frames} frames: camera ({:.2}, {:.2}, {:.2}) yaw {:.1} pitch {:.1}, time of day {}, sections {} drawn of {}, {} draw calls, target {:?}",
        file.display(),
        image.width(),
        image.height(),
        c.pos.x,
        c.pos.y,
        c.pos.z,
        c.yaw,
        c.pitch,
        view.time_of_day % 24000,
        stats.chunk_sections_drawn,
        stats.chunk_sections_total,
        stats.draw_calls,
        view.block_target
    );
    drop(session);
    Ok(())
}

/// Windowed mode.
pub fn run(args: &Args) -> anyhow::Result<()> {
    let session = Session::join(args)?;
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        shared: session.shared.clone(),
        biomes: session.biomes.clone(),
        // The flag wins over the saved option; without it, the saved option decides.
        vsync: args.vsync.unwrap_or(session.options.vsync),
        options: session.options.clone(),
        window: None,
        renderer: None,
        feed: WorldFeed::default(),
        gui: GuiFrame::default(),
        grabbed: false,
        focused: true,
        yaw: 0.0,
        pitch: 0.0,
        keys: HashSet::new(),
        input: InputState::default(),
        last_jump_press: None,
        toggle_flight: false,
        attack: false,
        use_item: false,
        pick_block: false,
        drop_item: false,
        drop_stack: false,
        swap_offhand: false,
        hotbar: None,
        scroll: 0.0,
        breaking: Vec::new(),
        fps: FpsCounter::new(),
        error: None,
        seconds: args.window_seconds,
        started: Instant::now(),
    };
    event_loop.run_app(&mut app)?;
    let reason = session.shared.closed.lock().clone();
    drop(session);
    if let Some(e) = app.error {
        return Err(e);
    }
    if let Some(r) = reason {
        println!("connection ended: {r}");
    }
    Ok(())
}

struct App {
    shared: Arc<Shared>,
    biomes: Vec<haste_render::BiomeClimate>,
    vsync: bool,
    /// Saved options (FOV for rendering, sensitivity for mouse look).
    options: Options,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    feed: WorldFeed,
    gui: GuiFrame,
    grabbed: bool,
    focused: bool,
    yaw: f32,
    pitch: f32,
    keys: HashSet<KeyCode>,
    /// Movement keys and look as of this frame (the game thread reads it each tick).
    input: InputState,
    last_jump_press: Option<Instant>,
    toggle_flight: bool,
    /// Left / right mouse held (attack / use), middle click and hotbar choice since the last
    /// sample.
    attack: bool,
    use_item: bool,
    pick_block: bool,
    drop_item: bool,
    drop_stack: bool,
    swap_offhand: bool,
    hotbar: Option<u8>,
    /// Scroll wheel steps not yet turned into a slot change.
    scroll: f64,
    breaking: Vec<(haste_protocol::haste_core::BlockPos, u8)>,
    fps: FpsCounter,
    error: Option<anyhow::Error>,
    /// Close the window after this long (`--window-seconds`, for unattended runs).
    seconds: Option<u64>,
    started: Instant,
}

impl App {
    fn set_grab(&mut self, grab: bool) {
        let Some(w) = &self.window else { return };
        if grab {
            let ok = w
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| w.set_cursor_grab(CursorGrabMode::Confined))
                .is_ok();
            w.set_cursor_visible(!ok);
            self.grabbed = ok;
        } else {
            let _ = w.set_cursor_grab(CursorGrabMode::None);
            w.set_cursor_visible(true);
            self.grabbed = false;
        }
    }

    fn release_keys(&mut self) {
        self.keys.clear();
        self.attack = false;
        self.use_item = false;
        self.last_jump_press = None;
        self.sample_input();
    }

    fn sample_input(&mut self) {
        let k = |c| self.keys.contains(&c);
        self.input = InputState {
            forward: k(KeyCode::KeyW),
            back: k(KeyCode::KeyS),
            left: k(KeyCode::KeyA),
            right: k(KeyCode::KeyD),
            jump: k(KeyCode::Space),
            sneak: k(KeyCode::ShiftLeft),
            sprint: k(KeyCode::ControlLeft),
            yaw: self.yaw,
            pitch: self.pitch,
        };
        let mut shared = self.shared.input.lock();
        shared.state = self.input;
        shared.toggle_flight |= std::mem::take(&mut self.toggle_flight);
        let i = &mut shared.interact;
        i.attack = self.attack;
        i.use_item = self.use_item;
        i.pick_block |= std::mem::take(&mut self.pick_block);
        i.drop_item |= std::mem::take(&mut self.drop_item);
        i.drop_stack |= std::mem::take(&mut self.drop_stack);
        i.swap_offhand |= std::mem::take(&mut self.swap_offhand);
        if let Some(slot) = self.hotbar.take() {
            i.hotbar = Some(slot);
        }
    }

    /// Jump pressed: a second press within vanilla's 7-tick window toggles flight.
    fn jump_pressed(&mut self, now: Instant) {
        match self.last_jump_press {
            Some(t) if now.duration_since(t) < DOUBLE_TAP => {
                self.toggle_flight = true;
                self.last_jump_press = None;
            }
            _ => self.last_jump_press = Some(now),
        }
    }

    /// Mouse look speed from the saved sensitivity.
    fn look_deg_per_pixel(&self) -> f32 {
        look_deg_per_pixel(self.options.mouse_sensitivity)
    }

    fn draw(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_none() {
            return;
        }
        self.sample_input();
        let renderer = self.renderer.as_mut().expect("checked above");
        let t0 = Instant::now();
        let (camera, sky, partial, vd) = frame_view(
            &self.shared,
            self.yaw,
            self.pitch,
            self.options.fov as f32,
            Instant::now(),
        );
        self.feed
            .feed(&self.shared, renderer, camera_section(&camera), vd);
        let t1 = Instant::now();
        let entities = self.feed.entities(&self.shared, renderer, partial);
        let block_target = self.shared.view.lock().block_target;
        self.breaking.clear();
        self.breaking
            .extend_from_slice(&self.shared.breaking.lock());
        let t2 = Instant::now();
        let frame = FrameInput {
            camera,
            entities,
            gui: &self.gui,
            sky,
            partial_tick: partial,
            block_target,
            breaking: &self.breaking,
        };
        // Outline boxes of the target and the states of breaking blocks (one short read lock).
        renderer.sample_world(&*self.shared.world.read(), &self.shared.data.shapes, &frame);
        let result = renderer.render(&frame);
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
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Haste")
            .with_inner_size(winit::dpi::LogicalSize::new(854.0, 480.0));
        let result = (|| -> anyhow::Result<(Arc<Window>, Renderer)> {
            let window = Arc::new(event_loop.create_window(attrs)?);
            let mut assets = open_assets()?;
            let mut renderer =
                Renderer::new(window.clone(), &mut assets, &self.shared.data.blocks)?;
            renderer.set_item_registry(&self.shared.data.registries);
            renderer.set_biomes(&self.biomes);
            Ok((window, renderer))
        })();
        match result {
            Ok((w, mut r)) => {
                r.set_vsync(self.vsync);
                self.window = Some(w);
                self.renderer = Some(r);
                self.set_grab(true);
            }
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(size.width.max(1), size.height.max(1));
                }
            }
            WindowEvent::Focused(f) => {
                self.focused = f;
                if !f {
                    self.release_keys();
                    self.set_grab(false);
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } if !self.grabbed => self.set_grab(true),
            WindowEvent::MouseInput { state, button, .. } if self.grabbed && self.focused => {
                let pressed = state == ElementState::Pressed;
                match button {
                    MouseButton::Left => self.attack = pressed,
                    MouseButton::Right => self.use_item = pressed,
                    MouseButton::Middle if pressed => self.pick_block = true,
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } if self.grabbed && self.focused => {
                self.scroll += match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / 40.0,
                };
                let steps = self.scroll.trunc() as i32;
                if steps != 0 {
                    self.scroll -= steps as f64;
                    // Scrolling down (negative) moves to the next slot, wrapping 0..=8.
                    let current = self
                        .hotbar
                        .map_or(self.shared.view.lock().held_slot, |s| s as i32);
                    self.hotbar = Some((current - steps).rem_euclid(9) as u8);
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(code),
                        state,
                        repeat,
                        ..
                    },
                ..
            } => {
                let pressed = state == ElementState::Pressed;
                if pressed && !repeat {
                    match code {
                        KeyCode::Escape => {
                            self.set_grab(false);
                            self.release_keys();
                        }
                        KeyCode::Space if self.grabbed && self.focused => {
                            self.jump_pressed(Instant::now())
                        }
                        _ if self.grabbed && self.focused && hotbar_key(code).is_some() => {
                            self.hotbar = hotbar_key(code);
                        }
                        KeyCode::KeyQ if self.grabbed && self.focused => {
                            // Ctrl+Q drops the whole stack, plain Q one item.
                            if self.keys.contains(&KeyCode::ControlLeft) {
                                self.drop_stack = true;
                            } else {
                                self.drop_item = true;
                            }
                        }
                        KeyCode::KeyF if self.grabbed && self.focused => {
                            self.swap_offhand = true;
                        }
                        KeyCode::F11 => {
                            if let Some(w) = &self.window {
                                let full = w.fullscreen().is_some();
                                w.set_fullscreen((!full).then_some(Fullscreen::Borderless(None)));
                            }
                        }
                        _ => {}
                    }
                }
                if self.focused && self.grabbed && pressed {
                    self.keys.insert(code);
                } else if !pressed {
                    self.keys.remove(&code);
                }
            }
            WindowEvent::RedrawRequested => self.draw(event_loop),
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if self.grabbed && self.focused {
                let (yaw, pitch) =
                    apply_mouse(self.yaw, self.pitch, delta, self.look_deg_per_pixel());
                self.yaw = yaw;
                self.pitch = pitch;
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.shared.is_closed() {
            event_loop.exit();
            return;
        }
        if self
            .seconds
            .is_some_and(|s| self.started.elapsed() >= Duration::from_secs(s))
        {
            event_loop.exit();
            return;
        }
        // Continuous rendering: draw here rather than waiting for RedrawRequested (Wayland
        // frame callbacks stall an unviewed window, NOTES.md).
        self.draw(event_loop);
    }
}

/// Mouse look: yaw wraps freely, pitch is clamped to straight up/down. `deg_per_pixel` is
/// [`look_deg_per_pixel`] of the saved sensitivity.
pub fn apply_mouse(yaw: f32, pitch: f32, delta: (f64, f64), deg_per_pixel: f32) -> (f32, f32) {
    let yaw = yaw + delta.0 as f32 * deg_per_pixel;
    let pitch = (pitch + delta.1 as f32 * deg_per_pixel).clamp(-90.0, 90.0);
    (yaw, pitch)
}

/// Frame-time statistics for the title bar, reported twice a second.
struct FpsCounter {
    since: Instant,
    frames: u32,
    worst_ms: f32,
    last: Instant,
    /// Per-phase sums since the last breakdown: feed, entities, render, and the renderer's
    /// own phases; logged every 2 s.
    phase_sum: [f32; 3],
    render_sum: haste_render::FrameTimings,
    phase_frames: u32,
    phase_since: Instant,
}

impl FpsCounter {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            since: now,
            frames: 0,
            worst_ms: 0.0,
            last: now,
            phase_sum: [0.0; 3],
            render_sum: Default::default(),
            phase_frames: 0,
            phase_since: now,
        }
    }

    fn phases(&mut self, feed: f32, entities: f32, render: f32, t: haste_render::FrameTimings) {
        self.phase_sum[0] += feed;
        self.phase_sum[1] += entities;
        self.phase_sum[2] += render;
        let s = &mut self.render_sum;
        s.prepare_ms += t.prepare_ms;
        s.acquire_ms += t.acquire_ms;
        s.cull_ms += t.cull_ms;
        s.encode_ms += t.encode_ms;
        s.submit_ms += t.submit_ms;
        s.present_ms += t.present_ms;
        for i in 0..3 {
            s.gpu_ms[i] += t.gpu_ms[i];
        }
        self.phase_frames += 1;
    }

    /// Average per-phase milliseconds over the last 2 s.
    fn breakdown(&mut self, renderer: &Renderer) -> Option<String> {
        if self.phase_since.elapsed() < Duration::from_secs(2) || self.phase_frames == 0 {
            return None;
        }
        let n = self.phase_frames as f32;
        let s = self.render_sum;
        let size = renderer.size();
        let line = format!(
            "frame ms: feed {:.2} entities {:.2} render {:.2} (prepare {:.2} acquire {:.2} cull {:.2} encode {:.2} submit {:.2} present {:.2}) gpu main {:.2} hand {:.2} gui {:.2}; {}x{} {:?}",
            self.phase_sum[0] / n,
            self.phase_sum[1] / n,
            self.phase_sum[2] / n,
            s.prepare_ms / n,
            s.acquire_ms / n,
            s.cull_ms / n,
            s.encode_ms / n,
            s.submit_ms / n,
            s.present_ms / n,
            s.gpu_ms[0] / n,
            s.gpu_ms[1] / n,
            s.gpu_ms[2] / n,
            size.0,
            size.1,
            renderer.present_mode(),
        );
        self.phase_sum = [0.0; 3];
        self.render_sum = Default::default();
        self.phase_frames = 0;
        self.phase_since = Instant::now();
        Some(line)
    }

    fn frame(&mut self, stats: &FrameStats) -> Option<String> {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32() * 1000.0;
        self.last = now;
        self.frames += 1;
        self.worst_ms = self.worst_ms.max(dt);
        let span = now.duration_since(self.since).as_secs_f32();
        if span < 0.5 {
            return None;
        }
        let line = format!(
            "{:.0} FPS, {:.2} ms avg, {:.2} ms worst, sections {}/{}, pending {}",
            self.frames as f32 / span,
            span * 1000.0 / self.frames as f32,
            self.worst_ms,
            stats.chunk_sections_drawn,
            stats.chunk_sections_total,
            stats.meshes_pending
        );
        self.since = now;
        self.frames = 0;
        self.worst_ms = 0.0;
        Some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DPP: f32 = 0.15; // vanilla's default sensitivity (50%) value

    #[test]
    fn mouse_look_clamps_pitch() {
        let (yaw, pitch) = apply_mouse(0.0, 0.0, (100.0, 0.0), DPP);
        assert!((yaw - 15.0).abs() < 1e-4 && pitch == 0.0, "{yaw} {pitch}");
        assert_eq!(apply_mouse(0.0, 80.0, (0.0, 200.0), DPP), (0.0, 90.0));
        assert_eq!(apply_mouse(10.0, -80.0, (0.0, -200.0), DPP), (10.0, -90.0));
    }

    #[test]
    fn sensitivity_formula_matches_vanilla_curve() {
        // Default sensitivity 50%: (0.5 * 0.6 + 0.2)^3 * 8 * 0.15 = 0.4^3 * 8 * 0.15.
        assert_eq!(look_deg_per_pixel(0.5), 0.15);
        // Off: the curve's minimum.
        assert_eq!(look_deg_per_pixel(0.0), 0.0096);
        // Max sensitivity: 0.8^3 * 8 * 0.15.
        assert!((look_deg_per_pixel(1.0) - 0.6144).abs() < 1e-6);
    }
}
