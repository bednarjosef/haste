//! Offscreen render of the fancy clouds from below (the ground player's view): one frame
//! with the cloud layer in view and one with it beyond the far plane, differing exactly
//! where the clouds draw. Catches a wrong face winding (the mesh would vanish) as well as
//! a misplaced mesh. Needs the client jar and a Vulkan driver (source scripts/env.sh);
//! saves the frame to target/render-checks/clouds_from_below.png.

use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId, ChunkPos, SectionPos};
use haste_render::{Camera, FrameInput, Renderer, SkyState, ViewBob};

struct Flat;

impl BlockView for Flat {
    fn block_state(&self, p: BlockPos) -> BlockStateId {
        if p.y == 0 {
            BlockStateId(1)
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

#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn clouds_visible_from_below() {
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
    let (w, h) = (320, 240);
    let mut renderer = Renderer::new_offscreen(w, h, &mut assets, &states).unwrap();
    let dirty: Vec<SectionPos> = (-2..=2)
        .flat_map(|x| (-2..=2).map(move |z| SectionPos::new(x, 0, z)))
        .collect();
    renderer.update_world(&Flat, &dirty, ChunkPos::new(0, 0), 4);
    for _ in 0..200 {
        renderer.update_world(&Flat, &[], ChunkPos::new(0, 0), 4);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let gui = haste_core::gui::GuiFrame::default();
    // Looking up from the ground (camera at 21.62, layer bottom 192.33), east toward the
    // texture's cloud blob around cell (23..31, 0..2) — the camera's cell (0, 0) is an
    // empty corner of clouds.png. The noon sun sits at the zenith, out of the frame at
    // this pitch.
    let frame = |cloud_height: f32| FrameInput {
        camera: Camera {
            pos: glam::DVec3::new(8.0, 21.62, 8.0),
            yaw: -90.0,
            pitch: -35.0,
            fov_deg: 70.0,
            hand_fov_deg: 70.0,
            roll_deg: 0.0,
            bob: ViewBob::default(),
        },
        entities: &[],
        block_entities: &[],
        gui: &gui,
        sky: SkyState {
            time_of_day: 6000,
            game_time: 0,
            rain: 0.0,
            thunder: 0.0,
            dimension_has_sky: true,
            fog_start: 100.0,
            fog_end: 200.0,
            cloud_height,
            ..Default::default()
        },
        partial_tick: 0.0,
        block_target: None,
        breaking: &[],
        camera_entity: None,
        spin: None,
        weather: None,
    };
    let render = |renderer: &mut Renderer, cloud_height: f32| {
        let frame = frame(cloud_height);
        let mut stats = renderer.render(&frame).unwrap();
        for _ in 0..200 {
            if stats.chunk_sections_drawn > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            stats = renderer.render(&frame).unwrap();
        }
        renderer.read_back().unwrap()
    };
    // The layer at its vanilla height, then lifted beyond the far plane (4096): the sky is
    // identical, so every differing pixel is a cloud.
    let with_clouds = render(&mut renderer, 192.33);
    let without = render(&mut renderer, 5000.0);
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    std::fs::create_dir_all(&out).unwrap();
    let file = out.join("clouds_from_below.png");
    with_clouds.save(&file).unwrap();
    let differing = with_clouds
        .pixels()
        .zip(without.pixels())
        .filter(|(a, b)| a.0 != b.0)
        .count();
    eprintln!(
        "saved {}: {differing} of {} pixels differ with the layer in view",
        file.display(),
        w * h
    );
    assert!(
        differing > 1000,
        "the cloud layer barely changes the frame: the mesh is missing or misplaced"
    );
    // And it is not a solid sheet: most of the sky stays clear between the clouds.
    assert!(
        differing < (w * h / 2) as usize,
        "the clouds cover more than half the frame"
    );
}
