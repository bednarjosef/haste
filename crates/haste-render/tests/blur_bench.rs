//! Dev-only benchmark (ignored test): the menu background blur's cost on the frame path.
//! Renders the flat offscreen world with a screen-split GUI frame (like the pause screen:
//! blur after the first command) and compares the frame time with the blur off (radius 0)
//! and on (radius 5, vanilla's default).
//!
//! usage: cargo test -q -p haste-render --test blur_bench -- --ignored --nocapture

use haste_core::gui::{GuiCommand, GuiFrame};
use haste_render::{Camera, FrameInput, Renderer, SkyState, ViewBob};

#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn menu_blur_frame_cost() {
    let jar = haste_core::paths::client_jar();
    if !jar.exists() {
        eprintln!("ignored: needs the client jar in the cache");
        return;
    }
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::cache_dir().join("assets"), "26.3")
            .unwrap();
    let states = haste_assets::BlocksJsonStates::load(
        &haste_core::paths::datagen_reports().join("blocks.json"),
    )
    .unwrap();
    let mut renderer = Renderer::new_offscreen(1280, 720, &mut assets, &states).unwrap();

    struct Flat;
    impl haste_core::view::BlockView for Flat {
        fn block_state(&self, p: haste_core::BlockPos) -> haste_core::BlockStateId {
            if p.y == 0 {
                haste_core::BlockStateId(1)
            } else {
                haste_core::BlockStateId::AIR
            }
        }
        fn is_chunk_loaded(&self, _: haste_core::ChunkPos) -> bool {
            true
        }
        fn sky_light(&self, p: haste_core::BlockPos) -> u8 {
            if p.y <= 0 {
                0
            } else {
                15
            }
        }
        fn block_light(&self, _: haste_core::BlockPos) -> u8 {
            0
        }
        fn min_y(&self) -> i32 {
            -64
        }
        fn height(&self) -> i32 {
            384
        }
    }
    let dirty: Vec<haste_core::SectionPos> = (-1..=1)
        .flat_map(|x| (-1..=1).map(move |z| haste_core::SectionPos::new(x, 0, z)))
        .collect();
    renderer.update_world(&Flat, &dirty, haste_core::ChunkPos::new(0, 0), 4);
    for _ in 0..200 {
        renderer.update_world(&Flat, &[], haste_core::ChunkPos::new(0, 0), 4);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    // A pause-screen-like frame: one gradient fill, the blur after it, then the screen's
    // own commands on top.
    let gui = GuiFrame {
        commands: vec![
            GuiCommand::FillGradient {
                x1: 0,
                y1: 0,
                x2: 1920,
                y2: 1080,
                top: 0x40000000,
                bottom: 0x40333333,
            },
            GuiCommand::Fill {
                x1: 500,
                y1: 300,
                x2: 780,
                y2: 420,
                color: 0xFFC0_C0C0,
            },
        ],
        scale: 2,
        width: 640,
        height: 360,
        cursor: None,
        blur_after: 1,
    };
    let camera = Camera {
        pos: glam::DVec3::new(8.0, 20.0, 8.0),
        yaw: 0.0,
        pitch: 60.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: ViewBob::default(),
    };
    fn frame(gui: &GuiFrame, camera: Camera) -> FrameInput<'_> {
        FrameInput {
            camera,
            entities: &[],
            block_entities: &[],
            gui,
            sky: SkyState {
                time_of_day: 6000,
                rain: 0.0,
                dimension_has_sky: true,
                fog_start: 100.0,
                fog_end: 200.0,
                ..Default::default()
            },
            partial_tick: 0.0,
            block_target: None,
            breaking: &[],
            camera_entity: None,
            spin: None,
        }
    }
    fn measure(renderer: &mut Renderer, gui: &GuiFrame, radius: u8, camera: Camera) -> (f32, f32) {
        renderer.set_menu_blur_radius(radius);
        let f = frame(gui, camera);
        let mut times = Vec::new();
        for _ in 0..300 {
            let stats = renderer.render(&f).unwrap();
            times.push(stats.frame_ms);
        }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        (
            times.iter().sum::<f32>() / times.len() as f32,
            times[times.len() * 99 / 100],
        )
    }
    let (off_avg, off_worst) = measure(&mut renderer, &gui, 0, camera);
    let (on_avg, on_worst) = measure(&mut renderer, &gui, 5, camera);
    eprintln!(
        "menu blur frame cost at 1280x720: off {off_avg:.2} ms avg / {off_worst:.2} ms worst, radius 5 {on_avg:.2} ms avg / {on_worst:.2} ms worst"
    );
}
