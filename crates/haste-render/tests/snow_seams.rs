//! The user bug "thin dashed lines on block edges from a distance" on a
//! snow-layered grass platform: reproduce the server scene offscreen from the
//! same camera and locate the dashes' geometry by bisecting the world.
//! Needs the client jar, datagen reports and a Vulkan driver (source
//! scripts/env.sh).
use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId, ChunkPos, Identifier, SectionPos};
use haste_render::{Camera, FrameInput, Renderer, SkyState};

/// The server platform at section-local coords: dirt at y=0, grass at y=1
/// (snowy on the west half), snow layers on the west half at y=2, short grass
/// sprinkled on the east half.
struct Platform {
    dirt: BlockStateId,
    snowy_grass: BlockStateId,
    grass: BlockStateId,
    snow: BlockStateId,
    /// Which parts are built (bisecting the cause).
    with_snow: bool,
    with_grass: bool,
    with_dirt: bool,
}

impl BlockView for Platform {
    fn block_state(&self, p: BlockPos) -> BlockStateId {
        let in_area = (0..10).contains(&p.x) && (0..10).contains(&p.z);
        match p.y {
            0 if in_area && self.with_dirt => self.dirt,
            1 if in_area && self.with_grass => {
                if p.x < 7 {
                    self.snowy_grass
                } else {
                    self.grass
                }
            }
            2 if in_area && self.with_snow && p.x < 5 => self.snow,
            _ => BlockStateId::AIR,
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

fn state(data: &haste_data::GameData, path: &str, prop: &str, value: &str) -> BlockStateId {
    (0..data.blocks.len() as u32)
        .map(BlockStateId)
        .find(|s| {
            data.blocks.block_of(*s).path() == path
                && data.blocks.property_value(*s, prop) == Some(value)
        })
        .unwrap_or_else(|| panic!("{path}[{prop}={value}]"))
}

/// Renders `world` from the server shot's camera (36 blocks south): grazing
/// two degrees up, or steep (pitch 15 down) when `steep`.
fn render_at(renderer: &mut Renderer, world: &dyn BlockView, steep: bool) -> image::RgbaImage {
    let dirty: Vec<SectionPos> = (-1..=1)
        .flat_map(|dx| (-1..=1).map(move |dz| SectionPos::new(dx, 0, dz)))
        .collect();
    let gui = haste_core::gui::GuiFrame::default();
    renderer.update_world(world, &dirty, ChunkPos::new(0, 0), 8);
    for _ in 0..300 {
        renderer.update_world(world, &[], ChunkPos::new(0, 0), 8);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let camera = Camera {
        pos: glam::DVec3::new(4.5, 3.62, 45.5),
        yaw: 180.0,
        pitch: if steep { 15.0 } else { -2.0 },
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    };
    let frame = FrameInput {
        camera,
        entities: &[],
        gui: &gui,
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
        block_entities: &[],
        spin: None,
        weather: None,
    };
    renderer.render(&frame).unwrap();
    renderer.render(&frame).unwrap();
    renderer.read_back().unwrap()
}

/// The darkest vertical run in the snow band: the user bug's dashes are
/// one-to-three-pixel wide dark vertical lines on the snow top band. Returns
/// the count of pixels at least `threshold` darker than both neighbours.
fn dark_line_pixels(img: &image::RgbaImage, y0: u32, y1: u32) -> u32 {
    let mut n = 0;
    for y in y0..y1 {
        for x in 2..img.width() - 2 {
            let lum =
                |p: image::Rgba<u8>| (p.0[0] as i32 * 2 + p.0[1] as i32 * 3 + p.0[2] as i32) / 6;
            let p = lum(*img.get_pixel(x, y));
            let l = lum(*img.get_pixel(x - 2, y));
            let r = lum(*img.get_pixel(x + 2, y));
            if l - p > 40 && r - p > 40 {
                n += 1;
            }
        }
    }
    n
}

#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn snow_platform_seams() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let mut renderer = Renderer::new_offscreen(1920, 1080, &mut assets, &data.blocks).unwrap();

    let full = Platform {
        dirt: data
            .blocks
            .default_state(&Identifier::minecraft("dirt"))
            .unwrap(),
        snowy_grass: state(&data, "grass_block", "snowy", "true"),
        grass: state(&data, "grass_block", "snowy", "false"),
        snow: state(&data, "snow", "layers", "2"),
        with_snow: true,
        with_grass: true,
        with_dirt: true,
    };
    let img = render_at(&mut renderer, &full, false);
    img.save("target/render-checks/snow-seams-full.png")
        .unwrap();
    let full_lines = dark_line_pixels(&img, 590, 610);
    eprintln!("full platform: {full_lines} dark line pixels");

    let no_snow = Platform {
        with_snow: false,
        ..full
    };
    let img = render_at(&mut renderer, &no_snow, false);
    img.save("target/render-checks/snow-seams-no-snow.png")
        .unwrap();
    let bare_lines = dark_line_pixels(&img, 590, 610);
    eprintln!("without snow layers: {bare_lines}");

    // The snowy/plain grass colour boundary is one legitimate vertical edge in
    // the window; everything beyond it is a seam. With the fract-wrap
    // derivative bug the counts were 46 (one dashed line per block edge on
    // the merged side quads).
    assert!(
        full_lines < 20,
        "thin dashed lines on the platform: {full_lines}"
    );
    assert!(
        bare_lines < 20,
        "thin dashed lines without snow (merged grass sides): {bare_lines}"
    );

    // The same platform seen steeply from above (the snow band is at a
    // different place there): merged tops must not crack either.
    let img = render_at(&mut renderer, &full, true);
    img.save("target/render-checks/snow-seams-steep.png")
        .unwrap();
    let steep_lines = dark_line_pixels(&img, 700, 795);
    eprintln!("steep view: {steep_lines} dark line pixels");
    assert!(
        steep_lines < 20,
        "thin dashed lines from above (merged snow tops): {steep_lines}"
    );
}
