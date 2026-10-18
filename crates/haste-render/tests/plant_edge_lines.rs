//! Light outlines around distant plants (branch plant-edge-lines).
//!
//! A meadow of grass blocks carrying short and tall grass, seen from a raised
//! camera looking down the plain, so the plants are minified (mipped) and
//! project onto the grass tops behind them. When the cross quads are lit as
//! side faces instead of vanilla's `shade_direction_override: "up"` (branch
//! plant-edge-lines), the plants draw 20-40% darker than the meadow, and the
//! bright background shining through their transparent fringe reads as a thin
//! light outline around every plant quad (user bug 3). A line pixel is one
//! that differs from both its horizontal (or both its vertical) neighbours by
//! more than a threshold while those neighbours agree with each other, in a
//! run of at least `MIN_RUN` pixels; with the bug the strongest such pixels
//! reach a weighted contrast of ~130, while correct plants stay below ~105
//! (their blades are as bright as the grass tops).
//!
//! Only rows below the horizon (terrain, not the sky gradient) and above the
//! close-plant band (closer than ~25 m, where individual blade texels are
//! normal detail) are scanned. Saves `target/render-checks/plant_edge_lines_*.png`.
//! Needs the client jar, the datagen reports and a Vulkan driver (source
//! scripts/env.sh).

use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId, ChunkPos, Identifier, SectionPos};
use haste_render::{Camera, FrameInput, Renderer, SkyState};

const MIN_RUN: usize = 3;
const W: u32 = 400;
const H: u32 = 300;
/// Line-pixel contrast (weighted: 2*red + 3*green + blue + alpha deltas).
const THRESHOLD: i32 = 120;
/// The scanned row band: below the horizon (pitch 20°, fov 70° → row 64),
/// above the close-plant rows (closer than ~25 m).
const FIRST_ROW: u32 = 66;
const LAST_ROW: u32 = 150;

fn diff(a: &image::Rgba<u8>, b: &image::Rgba<u8>) -> i32 {
    let c = |i: usize| a.0[i] as i32 - b.0[i] as i32;
    (c(0) * 2 + c(1) * 3 + c(2)).abs() + c(3).abs()
}

fn is_line(img: &image::RgbaImage, x: u32, y: u32, sky: image::Rgba<u8>) -> bool {
    let (w, h) = img.dimensions();
    if x == 0 || y == 0 || x + 1 >= w || y + 1 >= h {
        return false;
    }
    let p = img.get_pixel(x, y);
    let (l, r) = (img.get_pixel(x - 1, y), img.get_pixel(x + 1, y));
    let (u, d) = (img.get_pixel(x, y - 1), img.get_pixel(x, y + 1));
    let near_sky = |c: &image::Rgba<u8>| diff(c, &sky) <= 6;
    if [p, l, r, u, d].iter().any(|c| near_sky(c)) {
        return false;
    }
    let thin = |n: i32, a: i32| n <= THRESHOLD / 2 && a > THRESHOLD;
    thin(diff(l, r), diff(p, l).min(diff(p, r))) || thin(diff(u, d), diff(p, u).min(diff(p, d)))
}

fn most_common(img: &image::RgbaImage) -> image::Rgba<u8> {
    use std::collections::HashMap;
    let (w, h) = img.dimensions();
    let mut counts: HashMap<[u8; 4], usize> = HashMap::new();
    for y in (0..h).step_by(3) {
        for x in (0..w).step_by(7) {
            *counts.entry(img.get_pixel(x, y).0).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(c, _)| image::Rgba(c))
        .unwrap_or(image::Rgba([182, 209, 255, 255]))
}

/// Meadow: grass blocks at y=0 for x in -24..24, z in 0..56; short grass every
/// 4 blocks in x and z (offset), tall grass on a coarser grid.
struct Meadow {
    grass: BlockStateId,
    short: BlockStateId,
    tall: BlockStateId,
}

impl BlockView for Meadow {
    fn block_state(&self, p: BlockPos) -> BlockStateId {
        let (x, z) = (p.x, p.z);
        let in_area = (-24..24).contains(&x) && (0..56).contains(&z);
        match p.y {
            0 if in_area => self.grass,
            1 if in_area && x.rem_euclid(4) == 0 && z.rem_euclid(4) == 2 => self.short,
            1 if in_area && x.rem_euclid(12) == 6 && z.rem_euclid(12) == 8 => self.tall,
            _ => BlockStateId::AIR,
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

/// Renders `world` with `camera` after letting the mesh settle, and returns
/// the image (saving it under `tag` for manual inspection).
fn render(
    renderer: &mut Renderer,
    world: &dyn BlockView,
    camera: Camera,
    tag: &str,
) -> image::RgbaImage {
    let dirty: Vec<SectionPos> = (-2..=2)
        .flat_map(|dx| (-1..=2).map(move |dz| SectionPos::new(dx, 0, dz)))
        .collect();
    let gui = haste_core::gui::GuiFrame::default();
    renderer.update_world(world, &dirty, ChunkPos::new(0, 0), 4);
    for _ in 0..300 {
        renderer.update_world(world, &[], ChunkPos::new(0, 0), 4);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
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
    // Two frames: the section visibility walk's mask lands on a later frame.
    renderer.render(&frame).unwrap();
    renderer.render(&frame).unwrap();
    let img = renderer.read_back().unwrap();
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    let _ = std::fs::create_dir_all(&out);
    img.save(out.join(format!("plant_edge_lines_{tag}.png")))
        .unwrap();
    img
}

#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn no_light_outlines_around_distant_plants() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let meadow = Meadow {
        grass: data
            .blocks
            .state(&Identifier::minecraft("grass_block"), &[("snowy", "false")])
            .unwrap(),
        short: data
            .blocks
            .default_state(&Identifier::minecraft("short_grass"))
            .unwrap(),
        tall: data
            .blocks
            .default_state(&Identifier::minecraft("tall_grass"))
            .unwrap(),
    };
    let mut renderer = Renderer::new_offscreen(W, H, &mut assets, &data.blocks).unwrap();
    let img = render(
        &mut renderer,
        &meadow,
        Camera {
            pos: glam::DVec3::new(0.0, 6.0, -10.0),
            yaw: 0.0,
            pitch: 20.0,
            fov_deg: 70.0,
            hand_fov_deg: 70.0,
            roll_deg: 0.0,
            bob: haste_render::ViewBob::default(),
        },
        "meadow",
    );

    let sky = most_common(&img);
    let mut runs: Vec<(u32, u32, usize)> = Vec::new();
    for y in FIRST_ROW..LAST_ROW {
        let mut run = 0usize;
        let mut start = 0u32;
        for x in 1..W - 1 {
            if is_line(&img, x, y, sky) {
                if run == 0 {
                    start = x;
                }
                run += 1;
            } else {
                if run >= MIN_RUN {
                    runs.push((start, y, run));
                }
                run = 0;
            }
        }
        if run >= MIN_RUN {
            runs.push((start, y, run));
        }
    }
    eprintln!(
        "meadow: {} line runs (rows {FIRST_ROW}..{LAST_ROW})",
        runs.len()
    );
    for (x, y, len) in runs.iter().take(12) {
        let lo = x.saturating_sub(10);
        let hi = (x + *len as u32 + 10).min(W - 1);
        let slice: String = (lo..hi)
            .map(|col| {
                let p = img.get_pixel(col, *y).0;
                format!(
                    "{}{:02x}{:02x}{:02x}",
                    if is_line(&img, col, *y, sky) {
                        "*"
                    } else {
                        " "
                    },
                    p[0],
                    p[1],
                    p[2]
                )
            })
            .collect();
        eprintln!("  run at ({x},{y}) len {len}: {slice}");
    }
    assert!(runs.is_empty(), "light outlines: {runs:?}");
}
