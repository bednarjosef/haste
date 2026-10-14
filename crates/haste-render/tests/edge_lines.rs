//! Thin-line detector for block edges (branch block-edge-lines).
//!
//! Two synthetic worlds, both rendered with fixed cameras into an image buffer:
//!
//! * the scene from the bug report: a row of snowy grass blocks next to a row of
//!   grass blocks carrying a snow layer, one short grass on a snowy grass block
//!   and one on a snow layer. A level camera along the row sees the boundaries
//!   between adjacent columns as vertical lines on their side faces; a grazing
//!   camera from above sees the footprint rectangle of the block carrying the
//!   plant. Grass, snow and plant textures are noisy at one texel per pixel, so
//!   this scene runs at a high contrast threshold.
//! * a flat stone plain whose sky light varies per column two blocks above the
//!   blocks. Stone has a flat texture, so every line pixel here is a seam: the
//!   per-column variation at y+2 is what makes the smooth light of two adjacent
//!   coplanar top faces come from different cells (and draw a thin line along
//!   the shared edge) when the neighbour cells are read one cell further out
//!   along the face normal.
//!
//! A pixel is a line pixel when both its horizontal (or both its vertical)
//! neighbours agree with each other but the pixel between them differs from both
//! by more than the threshold, and it belongs to a run of at least `MIN_RUN` such
//! pixels: a texture cannot make a long, one-pixel-wide feature, a seam between
//! two faces can.
//!
//! Measured with the neighbour-cell fix in `mesher::vertex_light_for` reverted:
//! the plain's grazing views find 3 and 2 line runs (strongest weighted contrast
//! 127 and 114); with the fix, every view finds none. Saves
//! `target/render-checks/edge_lines_*.png`. Needs the client jar, the datagen
//! reports and a Vulkan driver (source scripts/env.sh).

use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId, ChunkPos, Identifier, SectionPos};
use haste_render::{Camera, FrameInput, Renderer, SkyState};

/// A seam runs along a whole face edge; shorter runs are texture detail.
const MIN_RUN: usize = 8;
/// Offscreen target size (x width is also used by the row dump).
const W: u32 = 400;
const H: u32 = 300;

/// The line detector's settings for one scene.
struct Detector {
    /// Minimum contrast between the line pixel and both its neighbours
    /// (weighted: 2*red + 3*green + blue + alpha deltas).
    threshold: i32,
}

/// The most common colour in the image: the sky fills whole areas of it, and a
/// line only counts when it sits between two terrain pixels (a plant's blades
/// against the sky are thin and dark, but they are not a seam).
fn sky_colour(img: &image::RgbaImage) -> image::Rgba<u8> {
    use std::collections::HashMap;
    let (w, h) = img.dimensions();
    let mut counts: HashMap<[u8; 4], usize> = HashMap::new();
    let mut y = 0;
    while y < h {
        let mut x = (y * 7) % 17;
        while x < w {
            *counts.entry(img.get_pixel(x, y).0).or_insert(0) += 1;
            x += 17;
        }
        y += 5;
    }
    counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(c, _)| image::Rgba(c))
        .unwrap_or(image::Rgba([182, 209, 255, 255]))
}

impl Detector {
    fn is_line(&self, img: &image::RgbaImage, x: u32, y: u32, sky: image::Rgba<u8>) -> bool {
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
        let thin = |n: i32, a: i32| n <= self.threshold && a > self.threshold;
        thin(diff(l, r), diff(p, l).min(diff(p, r))) || thin(diff(u, d), diff(p, u).min(diff(p, d)))
    }

    /// Starts, coordinates, lengths and the strongest contrast of `MIN_RUN`-long
    /// runs of line pixels, horizontally (`"row y"`) and vertically (`"col x"`).
    fn line_runs(
        &self,
        img: &image::RgbaImage,
        first_row: u32,
    ) -> Vec<(String, u32, u32, usize, i32)> {
        let (w, h) = img.dimensions();
        let sky = sky_colour(img);
        let contrast = |x: u32, y: u32| -> i32 {
            let p = img.get_pixel(x, y);
            let left = img.get_pixel(x.saturating_sub(1), y);
            let right = img.get_pixel((x + 1).min(w - 1), y);
            let above = img.get_pixel(x, y.saturating_sub(1));
            let below = img.get_pixel(x, (y + 1).min(h - 1));
            diff(p, left)
                .min(diff(p, right))
                .max(diff(p, above).min(diff(p, below)))
        };
        let line = |x: u32, y: u32| self.is_line(img, x, y, sky);
        let mut out = Vec::new();
        let flush = |out: &mut Vec<(String, u32, u32, usize, i32)>,
                     run: usize,
                     start: u32,
                     y: u32,
                     worst: i32| {
            if run >= MIN_RUN {
                out.push((format!("row {y}"), start, y, run, worst));
            }
        };
        for y in first_row..h - 1 {
            let mut run = 0usize;
            let mut start = 0u32;
            let mut worst = 0i32;
            for x in 1..w - 1 {
                if line(x, y) {
                    if run == 0 {
                        start = x;
                        worst = 0;
                    }
                    worst = worst.max(contrast(x, y));
                    run += 1;
                } else {
                    flush(&mut out, run, start, y, worst);
                    run = 0;
                }
            }
            flush(&mut out, run, start, y, worst);
        }
        let flush = |out: &mut Vec<(String, u32, u32, usize, i32)>,
                     run: usize,
                     start: u32,
                     x: u32,
                     worst: i32| {
            if run >= MIN_RUN {
                out.push((format!("col {x}"), x, start, run, worst));
            }
        };
        for x in 1..w - 1 {
            let mut run = 0usize;
            let mut start = 0u32;
            let mut worst = 0i32;
            for y in first_row..h - 1 {
                if line(x, y) {
                    if run == 0 {
                        start = y;
                        worst = 0;
                    }
                    worst = worst.max(contrast(x, y));
                    run += 1;
                } else {
                    flush(&mut out, run, start, x, worst);
                    run = 0;
                }
            }
            flush(&mut out, run, start, x, worst);
        }
        out
    }
}

fn diff(a: &image::Rgba<u8>, b: &image::Rgba<u8>) -> i32 {
    let c = |i: usize| a.0[i] as i32 - b.0[i] as i32;
    (c(0) * 2 + c(1) * 3 + c(2)).abs() + c(3).abs()
}

/// The horizon row of a symmetric vertical-fov projection with no roll:
/// `h/2 - pitch_half_fov_fraction * h/2`. Everything below it is terrain.
fn horizon_row(img_height: u32, pitch_deg: f32, fov_deg: f32) -> u32 {
    let half = (pitch_deg / (fov_deg / 2.0)).clamp(-1.0, 1.0);
    ((img_height as f32) * (0.5 - 0.5 * half)) as u32
}

/// Renders every camera of one world and returns the views that have line runs.
fn detect(
    renderer: &mut Renderer,
    world: &dyn BlockView,
    tag: &str,
    cameras: &[(&str, Camera)],
    detector: &Detector,
) -> Vec<(String, usize)> {
    let dirty: Vec<SectionPos> = (-1..=1)
        .flat_map(|dx| (-1..=1).map(move |dz| SectionPos::new(dx, 0, dz)))
        .collect();
    let gui = haste_core::gui::GuiFrame::default();
    renderer.update_world(world, &dirty, ChunkPos::new(0, 0), 4);
    for _ in 0..300 {
        renderer.update_world(world, &[], ChunkPos::new(0, 0), 4);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    let _ = std::fs::create_dir_all(&out);
    let mut failures = Vec::new();
    for (view, camera) in cameras {
        let frame = FrameInput {
            camera: *camera,
            entities: &[],
            gui: &gui,
            sky: SkyState {
                time_of_day: 6000,
                rain: 0.0,
                dimension_has_sky: true,
                fog_start: 100.0,
                fog_end: 200.0,
            },
            partial_tick: 0.0,
            block_target: None,
            breaking: &[],
            camera_entity: None,
            block_entities: &[],
            spin: None,
        };
        // Two frames: the section visibility walk's mask lands on a later frame.
        renderer.render(&frame).unwrap();
        renderer.render(&frame).unwrap();
        let img = renderer.read_back().unwrap();
        img.save(out.join(format!("edge_lines_{tag}_{view}.png")))
            .unwrap();
        let horizon = horizon_row(H, camera.pitch, camera.fov_deg) + 2;
        let runs = detector.line_runs(&img, horizon);
        let sky = sky_colour(&img);
        eprintln!(
            "{tag} {view}: {} runs, worst contrast {:?}",
            runs.len(),
            runs.iter()
                .map(|(k, x, y, l, c)| (c, k, x, y, l))
                .take(4)
                .collect::<Vec<_>>()
        );
        // The strongest run, pixel by pixel, so the line's colour is visible
        // without an image viewer.
        if let Some((_, x, y, len, _)) = runs.iter().max_by_key(|r| r.4) {
            let slice: String = (x.saturating_sub(20)..(x + *len as u32 + 20).min(W - 1))
                .map(|col| {
                    let p = img.get_pixel(col, *y).0;
                    format!(
                        "{}{:02x}{:02x}{:02x}",
                        if detector.is_line(&img, col, *y, sky) {
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
            eprintln!("  {tag} {view} strongest at ({x},{y}): {slice}");
        }
        if !runs.is_empty() {
            failures.push((format!("{tag}/{view}"), runs.len()));
        }
    }
    failures
}

/// x = 0..7: snowy grass blocks (a full cube with the snow side texture, top at
/// y=1). x = 8..15: grass at y=0 with a snow layer on top (top at y=1.125).
/// z = 0..8 only, so the rest of the area is empty and the row stands out.
struct SnowRow {
    grass: BlockStateId,
    snowy_grass: BlockStateId,
    snow_layer: BlockStateId,
    plant: BlockStateId,
}

impl BlockView for SnowRow {
    fn block_state(&self, p: BlockPos) -> BlockStateId {
        let (x, z) = (p.x, p.z);
        let in_area = (0..16).contains(&x) && (0..8).contains(&z);
        match p.y {
            0 if in_area => {
                if x < 8 {
                    self.snowy_grass
                } else {
                    self.grass
                }
            }
            1 if in_area && x >= 8 => self.snow_layer,
            // A short grass on a snowy grass block (its top is y=1).
            1 if (x, z) == (4, 4) => self.plant,
            // A short grass on a snow layer (its top is y=1.125).
            2 if (x, z) == (12, 6) => self.plant,
            _ => BlockStateId::AIR,
        }
    }
    fn is_chunk_loaded(&self, _: ChunkPos) -> bool {
        true
    }
    fn sky_light(&self, p: BlockPos) -> u8 {
        let _ = p.y;
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

/// A flat stone plain whose sky light varies per column two blocks above the
/// blocks: 9 at y=1 (just above them), 15 - (x & 7) at y=2, 15 in the open. The
/// y=2 plane is the one the *neighbour* light would read if the face normal were
/// applied twice, so a per-column variation there is exactly what turns into a
/// line along every block edge.
struct LitPlain {
    stone: BlockStateId,
}

impl BlockView for LitPlain {
    fn block_state(&self, p: BlockPos) -> BlockStateId {
        if p.y == 0 && (-20..20).contains(&p.x) && (-20..20).contains(&p.z) {
            self.stone
        } else {
            BlockStateId::AIR
        }
    }
    fn is_chunk_loaded(&self, _: ChunkPos) -> bool {
        true
    }
    fn sky_light(&self, p: BlockPos) -> u8 {
        match p.y {
            1 => 9,
            2 => 15 - (p.x as u8 & 7),
            _ => 15,
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
fn no_thin_lines_along_block_edges() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let plain_grass = data
        .blocks
        .state(&Identifier::minecraft("grass_block"), &[("snowy", "false")])
        .unwrap();
    let snow_row = SnowRow {
        grass: plain_grass,
        snowy_grass: data
            .blocks
            .state(&Identifier::minecraft("grass_block"), &[("snowy", "true")])
            .unwrap(),
        snow_layer: data
            .blocks
            .state(&Identifier::minecraft("snow"), &[("layers", "1")])
            .unwrap(),
        plant: data
            .blocks
            .default_state(&Identifier::minecraft("short_grass"))
            .unwrap(),
    };
    let lit_plain = LitPlain {
        stone: data
            .blocks
            .default_state(&Identifier::minecraft("stone"))
            .unwrap(),
    };
    let mut renderer = Renderer::new_offscreen(W, H, &mut assets, &data.blocks).unwrap();

    // The report's scene. A level camera along the row: the boundary between two
    // adjacent columns is a vertical shared edge on their side faces. The camera
    // stands far enough away that the textures are minified to a flat tone, so
    // only a seam can pass the contrast threshold.
    let row_views: [(&str, Camera); 2] = [
        (
            "side",
            Camera {
                pos: glam::DVec3::new(-2.0, 1.5, 40.0),
                yaw: -90.0,
                pitch: 0.0,
                fov_deg: 70.0,
                roll_deg: 0.0,
                bob: haste_render::ViewBob::default(),
            },
        ),
        (
            "above",
            Camera {
                pos: glam::DVec3::new(4.0, 6.0, -20.0),
                yaw: 0.0,
                pitch: 8.0,
                fov_deg: 70.0,
                roll_deg: 0.0,
                bob: haste_render::ViewBob::default(),
            },
        ),
    ];
    // The row's grass, snow and plant textures are noisy at one texel per pixel
    // (a run of them reaches a weighted contrast of ~145), so this scene runs at
    // a high threshold: only a real seam can pass it.
    let textured = Detector { threshold: 160 };
    let failures = detect(&mut renderer, &snow_row, "row", &row_views, &textured);

    // The light-varying plain: its top faces fill a level camera's view, and the
    // cells the buggy neighbour sampling reads (one cell further out along the
    // face normal) are what turn into a line along each block edge. Both cameras
    // look at the top faces from the side, where the textures are minified to a
    // flat tone and only a seam can pass the threshold.
    let plain_views: [(&str, Camera); 2] = [
        (
            "grazing",
            Camera {
                pos: glam::DVec3::new(8.0, 1.5, 24.0),
                yaw: 180.0,
                pitch: 0.0,
                fov_deg: 70.0,
                roll_deg: 0.0,
                bob: haste_render::ViewBob::default(),
            },
        ),
        (
            "grazing-north",
            Camera {
                pos: glam::DVec3::new(8.0, 1.5, -24.0),
                yaw: 0.0,
                pitch: 0.0,
                fov_deg: 70.0,
                roll_deg: 0.0,
                bob: haste_render::ViewBob::default(),
            },
        ),
    ];
    // Stone has a flat texture (its speckle stays below a weighted contrast of
    // ~70), so every line pixel here is a seam.
    let flat = Detector { threshold: 80 };
    let failures = [
        failures,
        detect(&mut renderer, &lit_plain, "plain", &plain_views, &flat),
    ]
    .concat();
    assert!(failures.is_empty(), "thin lines: {failures:?}");
}
