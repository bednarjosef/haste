//! renderer's world viewer: renders a synthetic flat world in a window (Task 2's
//! own testing tool; the lead wires the real headless-join world later).
//! Run: cargo run -p haste-render --example view_world [-- --distance 12]
use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId, ChunkPos, SectionPos};
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

struct FlatWorld {
    ground: BlockStateId,
}

impl BlockView for FlatWorld {
    fn block_state(&self, p: BlockPos) -> BlockStateId {
        // A flat plane with a few holes so the occlusion culling has work.
        if p.y == 0 && !(p.x.rem_euclid(16) < 3 && p.z.rem_euclid(16) < 3) {
            self.ground
        } else {
            BlockStateId::AIR
        }
    }
    fn is_chunk_loaded(&self, _: ChunkPos) -> bool {
        true
    }
    fn sky_light(&self, p: BlockPos) -> u8 {
        if p.y <= 0 {
            0
        } else {
            15
        }
    }
    fn block_light(&self, _: BlockPos) -> u8 {
        0
    }
    fn min_y(&self) -> i32 {
        -64
    }
    fn height(&self) -> i32 {
        384
    }
}

struct App {
    renderer: Option<haste_render::Renderer>,
    window: Option<Arc<Window>>,
    // Camera state (free-fly).
    cam: haste_render::Camera,
    dirty: Vec<SectionPos>,
    distance: i32,
    last_frame: std::time::Instant,
    frame_accum: (f32, u32),
}

impl App {
    fn new(distance: i32) -> App {
        App {
            renderer: None,
            window: None,
            cam: haste_render::Camera {
                pos: glam::DVec3::new(8.0, 24.0, 8.0),
                yaw: 45.0,
                pitch: 50.0,
                fov_deg: 70.0,
                roll_deg: 0.0,
            },
            dirty: (-distance..=distance)
                .flat_map(|x| (-distance..=distance).map(move |z| SectionPos::new(x, 0, z)))
                .collect(),
            distance,
            last_frame: std::time::Instant::now(),
            frame_accum: (0.0, 0),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title("haste: view_world"))
                .expect("create window"),
        );
        let jar = haste_core::paths::client_jar();
        let mut assets = haste_assets::Assets::open(
            &jar,
            &haste_core::paths::cache_dir().join("assets"),
            "26.3",
        )
        .expect("open assets");
        let states = haste_assets::BlocksJsonStates::load(
            &haste_core::paths::datagen_reports().join("blocks.json"),
        )
        .expect("load blocks.json");
        let renderer =
            haste_render::Renderer::new(window.clone(), &mut assets, &states).expect("renderer");
        self.window = Some(window);
        self.renderer = Some(renderer);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(size.width.max(1), size.height.max(1));
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(code),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => match code {
                KeyCode::Escape => event_loop.exit(),
                KeyCode::KeyW => self.cam.pos += forward(&self.cam) * 4.0,
                KeyCode::KeyS => self.cam.pos -= forward(&self.cam) * 4.0,
                KeyCode::KeyA => {
                    let f = forward(&self.cam);
                    self.cam.pos -= glam::DVec3::new(f.z, 0.0, -f.x).normalize() * 4.0;
                }
                KeyCode::KeyD => {
                    let f = forward(&self.cam);
                    self.cam.pos += glam::DVec3::new(f.z, 0.0, -f.x).normalize() * 4.0;
                }
                KeyCode::Space => self.cam.pos.y += 4.0,
                KeyCode::ShiftLeft => self.cam.pos.y -= 4.0,
                KeyCode::ArrowLeft => self.cam.yaw -= 10.0,
                KeyCode::ArrowRight => self.cam.yaw += 10.0,
                KeyCode::ArrowUp => self.cam.pitch += 5.0,
                KeyCode::ArrowDown => self.cam.pitch -= 5.0,
                _ => {}
            },
            WindowEvent::RedrawRequested => {
                self.draw();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Continuous rendering on Wayland: draw directly here rather than
        // waiting for RedrawRequested (frame callbacks can stall the loop).
        self.draw();
    }
}

impl App {
    fn draw(&mut self) {
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        let world = FlatWorld {
            ground: BlockStateId(1),
        };
        let dirty = std::mem::take(&mut self.dirty);
        renderer.update_world(&world, &dirty, ChunkPos::new(0, 0), self.distance);
        self.dirty = dirty;

        let frame = haste_render::FrameInput {
            camera: self.cam,
            entities: &[],
            gui: &haste_core::gui::GuiFrame::default(),
            sky: haste_render::SkyState {
                time_of_day: 6000,
                rain: 0.0,
                dimension_has_sky: true,
                fog_start: 100.0,
                fog_end: (self.distance as f32) * 16.0,
            },
            partial_tick: 0.0,
            block_target: None,
            breaking: &[],
            camera_entity: None,
        };
        let stats = renderer.render(&frame).expect("render");
        let dt = self.last_frame.elapsed().as_secs_f32() * 1000.0;
        self.last_frame = std::time::Instant::now();
        self.frame_accum = (self.frame_accum.0 + dt, self.frame_accum.1 + 1);
        if self.frame_accum.1 >= 60 {
            let avg = self.frame_accum.0 / self.frame_accum.1 as f32;
            eprintln!(
                "frame {:.2} ms (avg over 60), {:.0} FPS | drawn {}/{} sections, {} draw calls, pending {}",
                stats.frame_ms,
                1000.0 / avg,
                stats.chunk_sections_drawn,
                stats.chunk_sections_total,
                stats.draw_calls,
                stats.meshes_pending
            );
            self.frame_accum = (0.0, 0);
        }
    }
}

fn forward(cam: &haste_render::Camera) -> glam::DVec3 {
    let yaw = cam.yaw.to_radians();
    let pitch = cam.pitch.to_radians();
    glam::DVec3::new(
        (-pitch.cos() * yaw.sin()) as f64,
        (-pitch.sin()) as f64,
        (pitch.cos() * yaw.cos()) as f64,
    )
    .normalize()
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?),
        )
        .init();
    let distance: i32 = std::env::args()
        .nth_position_flag()
        .unwrap_or_else(|| "12".into())
        .parse()
        .unwrap_or(12);
    let mut app = App::new(distance);
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut app)?;
    Ok(())
}

trait ArgExt {
    fn nth_position_flag(&mut self) -> Option<String>;
}

impl ArgExt for std::env::Args {
    fn nth_position_flag(&mut self) -> Option<String> {
        // Skip the binary name; take the first arg not starting with '--'.
        self.skip(1).find(|a| !a.starts_with('-'))
    }
}
