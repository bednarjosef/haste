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
use haste_protocol::haste_core::gui::GuiFrame;
use haste_protocol::haste_core::input::InputState;
use haste_protocol::haste_core::model::EntityInstance;
use haste_protocol::haste_core::{ChunkPos, SectionPos};
use haste_protocol::{Account, ConnectOptions};
use haste_render::{Camera, FrameInput, FrameStats, Renderer, SkyState};
use haste_world::World;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Fullscreen, Window, WindowId};

use crate::game::{self, Shared};
use crate::net::{self, NetDriver};
use crate::Args;

/// Field of view in degrees (vanilla default "Normal" = 70).
const FOV_DEG: f32 = 70.0;
/// Degrees of rotation per pixel of mouse motion at vanilla's default sensitivity (50%):
/// 0.15 * (0.5 * 0.6 + 0.2)^3 * 8 (minecraft.wiki "Options", mouse sensitivity).
const LOOK_DEG_PER_PIXEL: f32 = 0.15;
/// Two jump presses closer than this toggle flight (vanilla: 7 ticks).
const DOUBLE_TAP: Duration = Duration::from_millis(350);
/// Wall time the render thread may spend capturing sections for the mesher per frame.
const CAPTURE_BUDGET: Duration = Duration::from_millis(4);

/// A joined game: runtime, game thread and shared state.
struct Session {
    /// Kept alive for the connection's tasks; dropped after the game thread.
    _rt: tokio::runtime::Runtime,
    shared: Arc<Shared>,
    game_thread: Option<std::thread::JoinHandle<()>>,
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
        let opts = ConnectOptions::new(
            address.clone(),
            Account::Offline {
                username: args.username.clone(),
            },
        );
        let driver: NetDriver = rt.block_on(NetDriver::connect(opts))?;
        let regs = driver.connection().registries();
        let biomes = net::synced_biome_count(regs).unwrap_or(64);
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
        let shared = Arc::new(Shared::new(
            data,
            world,
            net::new_entity_store(entity_types),
            login.view_distance,
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
    /// Every section ever submitted (the renderer keeps a mesh entry for each).
    submitted: HashSet<SectionPos>,
    sort_needed: bool,
    last_arrival: Option<Instant>,
    /// Entity instances for this frame (reused).
    instances: Vec<EntityInstance>,
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

    /// Submits queued sections (nearest first) until the capture budget is used, and lets the
    /// renderer upload finished meshes.
    fn feed(
        &mut self,
        shared: &Shared,
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
        let world = shared.world.read();
        let started = Instant::now();
        let mut submitted_any = false;
        while started.elapsed() < CAPTURE_BUDGET {
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
            self.submitted.insert(pos);
            renderer.update_world(&*world, &[pos], camera_chunk, view_distance);
            submitted_any = true;
        }
        if !submitted_any {
            // Still upload meshes that finished since the last frame.
            renderer.update_world(&*world, &[], camera_chunk, view_distance);
        }
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
    fn settled(&self, stats: &FrameStats, quiet: Duration) -> bool {
        self.queue.is_empty()
            && stats.meshes_pending == 0
            && stats.chunk_sections_total as usize >= self.submitted.len()
            && self.last_arrival.is_some_and(|t| t.elapsed() >= quiet)
    }
}

/// Builds this frame's camera and sky from the shared view at `now`. `yaw`/`pitch` are the
/// input layer's look; the player's server rotation offset is added here so mouse look is
/// smooth at frame rate while the player itself turns once per tick.
fn frame_view(shared: &Shared, yaw: f32, pitch: f32, now: Instant) -> (Camera, SkyState, f32, i32) {
    let view = *shared.view.lock();
    let partial = game::partial_tick(view.tick_at, now);
    let camera = Camera {
        // Linear between LocalPlayer::camera(0.0) and camera(1.0), i.e. camera(partial).
        pos: game::interpolate(view.prev_eye, view.eye, partial),
        yaw: yaw + view.look_offset.0,
        pitch: (pitch + view.look_offset.1).clamp(-90.0, 90.0),
        fov_deg: FOV_DEG,
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
    let gui = GuiFrame::default();
    let mut feed = WorldFeed::default();
    let deadline = Instant::now() + Duration::from_secs(args.seconds);
    let mut frames = 0u32;
    let mut stats;
    let mut last_camera;
    loop {
        let (mut camera, sky, partial, vd) = frame_view(&shared, 0.0, 0.0, Instant::now());
        camera.yaw = args.yaw.unwrap_or(camera.yaw);
        camera.pitch = args.pitch.unwrap_or(camera.pitch);
        last_camera = camera;
        feed.feed(&shared, &mut renderer, camera_section(&camera), vd);
        let entities = feed.entities(&shared, &mut renderer, partial);
        stats = renderer.render(&FrameInput {
            camera,
            entities,
            gui: &gui,
            sky,
            partial_tick: partial,
            block_target: None,
            breaking: &[],
        })?;
        frames += 1;
        let placed = shared.view.lock().has_position;
        if shared.is_closed() {
            anyhow::bail!(
                "disconnected: {}",
                shared.closed.lock().clone().unwrap_or_default()
            );
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
    println!(
        "screenshot {} ({}x{}) after {frames} frames: camera ({:.2}, {:.2}, {:.2}) yaw {:.1} pitch {:.1}, time of day {}, sections {} drawn of {}, {} draw calls",
        file.display(),
        image.width(),
        image.height(),
        c.pos.x,
        c.pos.y,
        c.pos.z,
        c.yaw,
        c.pitch,
        shared.view.lock().time_of_day % 24000,
        stats.chunk_sections_drawn,
        stats.chunk_sections_total,
        stats.draw_calls
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
        vsync: args.vsync,
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
    vsync: bool,
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

    fn draw(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_none() {
            return;
        }
        self.sample_input();
        let renderer = self.renderer.as_mut().expect("checked above");
        let (camera, sky, partial, vd) =
            frame_view(&self.shared, self.yaw, self.pitch, Instant::now());
        self.feed
            .feed(&self.shared, renderer, camera_section(&camera), vd);
        let entities = self.feed.entities(&self.shared, renderer, partial);
        match renderer.render(&FrameInput {
            camera,
            entities,
            gui: &self.gui,
            sky,
            partial_tick: partial,
            // Filled from LocalPlayer::target / break_progress when interaction lands.
            block_target: None,
            breaking: &[],
        }) {
            Ok(stats) => {
                if let Some(line) = self.fps.frame(&stats) {
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
                let (yaw, pitch) = apply_mouse(self.yaw, self.pitch, delta);
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

/// Mouse look: yaw wraps freely, pitch is clamped to straight up/down.
pub fn apply_mouse(yaw: f32, pitch: f32, delta: (f64, f64)) -> (f32, f32) {
    let yaw = yaw + delta.0 as f32 * LOOK_DEG_PER_PIXEL;
    let pitch = (pitch + delta.1 as f32 * LOOK_DEG_PER_PIXEL).clamp(-90.0, 90.0);
    (yaw, pitch)
}

/// Frame-time statistics for the title bar, reported twice a second.
struct FpsCounter {
    since: Instant,
    frames: u32,
    worst_ms: f32,
    last: Instant,
}

impl FpsCounter {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            since: now,
            frames: 0,
            worst_ms: 0.0,
            last: now,
        }
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

    #[test]
    fn mouse_look_clamps_pitch() {
        let (yaw, pitch) = apply_mouse(0.0, 0.0, (100.0, 0.0));
        assert!((yaw - 15.0).abs() < 1e-4 && pitch == 0.0, "{yaw} {pitch}");
        assert_eq!(apply_mouse(0.0, 80.0, (0.0, 200.0)), (0.0, 90.0));
        assert_eq!(apply_mouse(10.0, -80.0, (0.0, -200.0)), (10.0, -90.0));
    }
}
