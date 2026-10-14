//! Block entities checked offscreen: a chest and a shulker box on a stone floor, drawn closed
//! and open, and the lid animation between them. Saves `target/render-checks/chest.png` and
//! `target/render-checks/chest-open.png`. Needs the client jar, the datagen reports and a
//! Vulkan driver (source scripts/env.sh).

use haste_core::{BlockPos, BlockStateId, ChunkPos, Direction, SectionPos};
use haste_render::blockentity::{BlockEntityInstance, BlockEntityKind, ChestHalf, ChestMaterial};
use haste_render::{Camera, FrameInput, Renderer, SkyState, TickPair};

struct Flat {
    stone: BlockStateId,
}

impl haste_core::view::BlockView for Flat {
    fn block_state(&self, p: BlockPos) -> BlockStateId {
        if p.y == 0 {
            self.stone
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

fn render(
    renderer: &mut Renderer,
    _world: &Flat,
    block_entities: &[BlockEntityInstance],
    camera: Camera,
) -> image::RgbaImage {
    let frame = FrameInput {
        camera,
        entities: &[],
        block_entities,
        gui: &haste_core::gui::GuiFrame::default(),
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
    };
    renderer.render(&frame).unwrap();
    renderer.read_back().unwrap()
}

#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn block_entities_offscreen() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let mut renderer = Renderer::new_offscreen(640, 480, &mut assets, &data.blocks).unwrap();
    let world = Flat {
        stone: data
            .blocks
            .default_state(&haste_core::Identifier::minecraft("stone"))
            .unwrap(),
    };
    let dirty: Vec<SectionPos> = (-1..=1)
        .flat_map(|x| (-1..=1).map(move |z| SectionPos::new(x, 0, z)))
        .collect();
    renderer.update_world(&world, &dirty, ChunkPos::new(0, 0), 4);
    for _ in 0..200 {
        renderer.update_world(&world, &[], ChunkPos::new(0, 0), 4);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // A chest and a shulker box on the floor at y = 1, seen from z = -4 at eye height. The
    // chest faces the camera (its front, the lock side, points south = +z = toward us).
    let chest = BlockEntityInstance {
        pos: BlockPos::new(0, 1, 0),
        facing: Direction::South,
        half: ChestHalf::Single,
        kind: BlockEntityKind::Chest(ChestMaterial::Normal),
        open: TickPair::fixed(0.0),
        light: (15, 0),
    };
    let shulker = BlockEntityInstance {
        pos: BlockPos::new(2, 1, 0),
        facing: Direction::Up,
        half: ChestHalf::Single,
        kind: BlockEntityKind::Shulker(None),
        open: TickPair::fixed(0.0),
        light: (15, 0),
    };
    // The camera looks at the chest's centre from four blocks south and 1.5 up: the chest
    // lands mid-frame.
    let camera = Camera {
        pos: glam::DVec3::new(0.5, 2.6, -4.0),
        yaw: 0.0,
        pitch: 18.0,
        fov_deg: 70.0,
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    };

    // Warm-up renders: the section-visibility walk lags a frame (the previous mask is drawn
    // until the new one arrives), so the first frames cull everything.
    for _ in 0..4 {
        render(&mut renderer, &world, &[], camera);
    }

    let closed = render(&mut renderer, &world, &[chest, shulker], camera);
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    std::fs::create_dir_all(&out).unwrap();
    closed.save(out.join("chest.png")).unwrap();
    let empty = render(&mut renderer, &world, &[], camera);
    empty.save(out.join("chest-empty.png")).unwrap();

    // The chest sits at the frame centre (the camera looks at it): its pixels there differ
    // from the empty scene, and they are the chest's brown, not the sky above it.
    let centre = closed.get_pixel(320, 240).0;
    assert_ne!(centre, empty.get_pixel(320, 240).0, "chest at the centre");
    let sky = closed.get_pixel(320, 30).0;
    assert_ne!(centre, sky, "chest is not sky");
    let (r, g, b) = (centre[0] as i32, centre[1] as i32, centre[2] as i32);
    assert!(
        r > g && g > b && r > 60,
        "chest centre is wooden brown, got {centre:?}"
    );

    // The shulker box, two blocks east of the chest: seen looking south, east is screen
    // left. One block is about 77 px at this distance (the camera is 4.5 blocks away); the
    // shulker centre is 2.5 blocks east of the camera.
    let shulker_x = (320.0 - 2.5 / 4.17 * 320.0) as u32;
    let shulker_pixel = closed.get_pixel(shulker_x.clamp(0, 639), 240).0;
    assert_ne!(
        shulker_pixel,
        empty.get_pixel(shulker_x.clamp(0, 639), 240).0,
        "shulker box east of the chest"
    );

    // The open chest: the lid swings up, so the frame differs from the closed one around the
    // chest (and the lid rises above the block, into what was sky).
    let mut open = chest;
    open.open = TickPair::fixed(0.5);
    let half = render(&mut renderer, &world, &[open, shulker], camera);
    half.save(out.join("chest-open.png")).unwrap();
    let mut differing = 0;
    for (x, y, p) in closed.enumerate_pixels() {
        if p != half.get_pixel(x, y) {
            differing += 1;
        }
    }
    assert!(differing > 2000, "lid moved {differing} pixels");
    // The shulker box itself is unchanged by the chest's lid.
    assert_eq!(
        closed.get_pixel(shulker_x.clamp(0, 639), 240),
        half.get_pixel(shulker_x.clamp(0, 639), 240)
    );

    // Fully open: more of the lid has moved up than at half open.
    open.open = TickPair::fixed(1.0);
    let full = render(&mut renderer, &world, &[open, shulker], camera);
    let mut differing_full = 0;
    for (x, y, p) in closed.enumerate_pixels() {
        if p != full.get_pixel(x, y) {
            differing_full += 1;
        }
    }
    assert!(
        differing_full > differing,
        "full open ({differing_full}) moves more than half ({differing})"
    );
    full.save(out.join("chest-full.png")).unwrap();
}
