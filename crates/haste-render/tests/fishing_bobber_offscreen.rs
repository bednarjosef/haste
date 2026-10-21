//! The fishing bobbers offscreen (`FishingHookRenderer`): a bobber's sprite and its black line
//! from a player's rod hand, drawn through the entity pass, and a release-timed frame-cost
//! check of 50 bobbers with their lines against the same frame without them.
use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId, ChunkPos, DVec3, EntityId, Identifier};
use haste_render::entity::{BobberOwner, FishingBobber, LINE_POINTS};
use haste_render::{Camera, FrameInput, Renderer, SkyState, ViewBob};

/// A stone floor at y = 0, air above.
struct Floor {
    stone: BlockStateId,
}

impl BlockView for Floor {
    fn block_state(&self, p: BlockPos) -> BlockStateId {
        if p.y < 0 {
            self.stone
        } else {
            BlockStateId(0)
        }
    }
    fn is_chunk_loaded(&self, _: ChunkPos) -> bool {
        true
    }
    fn sky_light(&self, _: BlockPos) -> u8 {
        15
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

/// The renderer and the stone floor, meshed and settled (the jar, the reports and a Vulkan
/// driver must be present).
fn scene() -> (Renderer, Floor) {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let mut renderer = Renderer::new_offscreen(640, 480, &mut assets, &data.blocks).unwrap();
    renderer.set_item_registry(&data.registries);
    let world = Floor {
        stone: data
            .blocks
            .default_state(&Identifier::minecraft("stone"))
            .unwrap(),
    };
    let dirty: Vec<haste_core::SectionPos> = (-1..=1)
        .flat_map(|x| (-1..=1).map(move |z| haste_core::SectionPos::new(x, 0, z)))
        .collect();
    renderer.update_world(&world, &dirty, ChunkPos::new(0, 0), 4);
    for _ in 0..200 {
        renderer.update_world(&world, &[], ChunkPos::new(0, 0), 4);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    (renderer, world)
}

/// The frame the bobbers render with, from a camera 4 blocks south of them.
/// `gui` outlives the returned frame (the caller owns it).
fn frame<'a>(gui: &'a haste_core::gui::GuiFrame) -> FrameInput<'a> {
    FrameInput {
        camera: Camera {
            pos: DVec3::new(0.0, 2.6, -4.0),
            yaw: 0.0,
            pitch: 12.0,
            fov_deg: 70.0,
            hand_fov_deg: 70.0,
            roll_deg: 0.0,
            bob: ViewBob::default(),
        },
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
        weather: None,
    }
}

/// The empty HUD frame the scenes render with.
fn gui() -> haste_core::gui::GuiFrame {
    haste_core::gui::GuiFrame {
        scale: 2,
        ..Default::default()
    }
}

/// One bobber at `(x, 1.3, 0.5)` whose owner is player `id`, holding the rod in the right
/// hand, with a line to a hand 1.6 blocks to the side and 0.6 above (the 17 `stringVertex`
/// points, relative to the bobber).
fn bobber(id: i32, x: f64) -> FishingBobber {
    let hand = glam::Vec3::new(1.6, 0.6, -2.0);
    let mut line = [glam::Vec3::ZERO; LINE_POINTS];
    for (i, p) in line.iter_mut().enumerate() {
        *p = hand * (i as f32 / (LINE_POINTS - 1) as f32);
    }
    FishingBobber {
        pos: DVec3::new(x, 1.3, 0.5),
        light: (15, 0),
        line: Some(line),
        owner: Some(BobberOwner {
            id: EntityId(id),
            holding_right: true,
        }),
    }
}

#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn fishing_bobbers_offscreen() {
    let (mut renderer, _world) = scene();
    let gui = gui();
    let bobbers = [bobber(1, -1.2), bobber(2, 0.0), bobber(3, 1.2)];
    renderer.set_fishing_bobbers(&bobbers);
    renderer.render(&frame(&gui)).unwrap();
    let img = renderer.read_back().unwrap();
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    std::fs::create_dir_all(&out).unwrap();
    img.save(out.join("fishing_bobbers.png")).unwrap();

    // The same frame with no bobbers leaves the floor and the sky alone.
    renderer.set_fishing_bobbers(&[]);
    renderer.render(&frame(&gui)).unwrap();
    let empty = renderer.read_back().unwrap();
    let changed = img
        .enumerate_pixels()
        .filter(|(x, y, p)| p.0 != empty.get_pixel(*x, *y).0)
        .count();
    eprintln!("bobber pixels: {changed}");
    // Three sprites and three black lines cover a band of the floor.
    assert!(
        changed > 300,
        "the bobbers and lines are drawn ({changed} pixels)"
    );
}

/// The per-frame cost of 50 fishing bobbers, each with its line from its player's rod hand:
/// the frame with them against the same frame with none. Release only; prints average and
/// worst frame.
#[test]
#[ignore = "timed test, run in release: cargo test --release -p haste-render --test fishing_bobber_offscreen -- --ignored --nocapture"]
fn fifty_fishing_bobbers_frame_time() {
    let (mut renderer, _world) = scene();
    let gui = gui();
    let bobbers: Vec<FishingBobber> = (0..50)
        .map(|i| bobber(100 + i, -7.0 + f64::from(i % 25) * 0.6))
        .collect();
    let measure = |renderer: &mut Renderer| {
        let mut worst = std::time::Duration::ZERO;
        let mut total = std::time::Duration::ZERO;
        const FRAMES: u32 = 200;
        for _ in 0..FRAMES {
            let start = std::time::Instant::now();
            renderer.render(&frame(&gui)).unwrap();
            let d = start.elapsed();
            worst = worst.max(d);
            total += d;
        }
        (total / FRAMES, worst)
    };
    // Warm both paths (texture rects and the first vertex upload).
    renderer.set_fishing_bobbers(&bobbers);
    for _ in 0..10 {
        renderer.render(&frame(&gui)).unwrap();
    }
    let (avg, worst) = measure(&mut renderer);
    renderer.set_fishing_bobbers(&[]);
    for _ in 0..10 {
        renderer.render(&frame(&gui)).unwrap();
    }
    let (base_avg, base_worst) = measure(&mut renderer);
    eprintln!(
        "50 fishing bobbers with lines: avg {avg:?} worst {worst:?} (no bobbers: avg {base_avg:?} worst {base_worst:?})"
    );
}
