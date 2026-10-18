//! T-junction cracks: the greedy merge (`mesher/merge.rs`) joins coplanar
//! unit faces into rectangles, so a merged quad's edge can run past the
//! corners of the smaller unmerged quads next to it (a T-junction). The two
//! sides rasterize the shared line from different vertex pairs, and the
//! floating-point difference opens single-pixel cracks that show the
//! background inside the terrain silhouette — thin lines vanilla (one quad
//! per block face) never draws.
//!
//! The scene: a wide grass field with stone blocks breaking the merge (and
//! section boundaries crossing it, where independently meshed sections meet),
//! plus a terraced slope, seen from far away at a grazing pitch. The oracle
//! is a pixel diff against the same scene meshed with the greedy merge
//! switched off (`set_greedy_merge_enabled(false)`, vanilla's one quad per
//! face): the merge is designed to be pixel-identical, so every differing
//! pixel is a rendering fault of the merge — a crack shows the background
//! colour where the unmerged mesh draws terrain. Saves
//! `target/render-checks/merged_quad_cracks_*.png`.
//!
//! Needs the client jar, the datagen reports and a Vulkan driver (source
//! scripts/env.sh).

use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId, ChunkPos, Identifier, SectionPos};
use haste_render::mesher::set_greedy_merge_enabled;
use haste_render::{Camera, FrameInput, Renderer, SkyState};

const W: u32 = 1920;
const H: u32 = 1080;
/// Weighted channel delta above which two pixels count as different.
const DIFF_THRESHOLD: i32 = 12;

fn diff(a: &image::Rgba<u8>, b: &image::Rgba<u8>) -> i32 {
    let c = |i: usize| a.0[i] as i32 - b.0[i] as i32;
    (c(0) * 2 + c(1) * 3 + c(2)).abs() + c(3).abs()
}

/// The field: grass tops at the terrain height (flat, then terraces rising
/// one block every 8 rows from z=48), stone blocks scattered on the flat
/// part to break the merge, and a snow-layer patch (the scene the lead saw
/// the line on: merged snow tops against per-unit snow side quads). The
/// field spans several sections so their independently meshed boundaries
/// (x = -16, 0, 16; z = 0, 16, ...) meet the merged quads too.
struct Field {
    grass: BlockStateId,
    snowy_grass: BlockStateId,
    dirt: BlockStateId,
    stone: BlockStateId,
    snow: BlockStateId,
}

/// Terrain height at (x, z): flat 0, then a terrace slope of 1-block steps
/// every 8 rows from z=48 up to z=79, flat 4 after.
fn ground_height(x: i32, z: i32) -> i32 {
    let _ = x;
    if z < 48 {
        0
    } else {
        4.min(1 + (z - 48) / 8)
    }
}

/// The stone blocks breaking the merge on the flat part (x, z), some next to
/// section boundaries.
const BREAKERS: [(i32, i32); 16] = [
    (-9, -2),
    (5, 7),
    (1, 14),
    (-20, 20),
    (17, 29),
    (-27, 33),
    (12, 41),
    (-4, 45),
    (-12, 3),
    (8, 18),
    (-24, 26),
    (20, 36),
    (-2, 9),
    (14, 24),
    (-17, 38),
    (3, 30),
];

/// The snow patch: layers=2 on the ground for x < -12, z in 8..40 (over grass
/// made snowy, as vanilla renders it).
fn snow_at(x: i32, z: i32) -> bool {
    x < -12 && (8..40).contains(&z)
}

impl BlockView for Field {
    fn block_state(&self, p: BlockPos) -> BlockStateId {
        let in_field = (-32..32).contains(&p.x) && (-16..112).contains(&p.z);
        if !in_field || p.y < 0 || p.y > 6 {
            return BlockStateId::AIR;
        }
        let h = ground_height(p.x, p.z);
        if p.y == h {
            if snow_at(p.x, p.z) {
                self.snowy_grass
            } else {
                self.grass
            }
        } else if p.y < h && p.y >= h - 3 {
            self.dirt
        } else if p.y == h + 1 && snow_at(p.x, p.z) {
            self.snow
        } else if p.y == h + 1 && p.z < 48 && BREAKERS.contains(&(p.x, p.z)) {
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

/// Meshes the field from scratch (greedy merge on or off), renders it from
/// `camera` and returns the image (saved under `tag`).
fn render(
    renderer: &mut Renderer,
    world: &dyn BlockView,
    camera: Camera,
    merge: bool,
    tag: &str,
) -> image::RgbaImage {
    set_greedy_merge_enabled(merge);
    let dirty: Vec<SectionPos> = (-2..2)
        .flat_map(|sx| (-1..7).map(move |sz| SectionPos::new(sx, 0, sz)))
        .collect();
    let gui = haste_core::gui::GuiFrame::default();
    renderer.update_world(world, &dirty, ChunkPos::new(0, 0), 8);
    for _ in 0..200 {
        renderer.update_world(world, &[], ChunkPos::new(0, 0), 8);
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
            fog_start: 400.0,
            fog_end: 800.0,
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
    // Render until the image is stable: the section visibility walk's mask
    // lands on a later frame, and a section not yet visible would read as a
    // false diff (sky where the plain render, taken later, draws terrain).
    let mut img = renderer.read_back().unwrap();
    for _ in 0..20 {
        renderer.render(&frame).unwrap();
        let next = renderer.read_back().unwrap();
        if next.as_raw() == img.as_raw() {
            img = next;
            break;
        }
        img = next;
    }
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    let _ = std::fs::create_dir_all(&out);
    img.save(out.join(format!("merged_quad_cracks_{tag}.png")))
        .unwrap();
    img
}

/// Pixels that differ between the merged and the unmerged render AND show
/// the background through the merged one: the crack signature (the sky or
/// clear colour where the unmerged mesh, like vanilla, draws terrain). The
/// sky at noon is blue-dominant and no terrain colour is (grass is green,
/// dirt and stone are red-dominant), so a blue-dominant merged pixel is the
/// background showing through. Small colour deltas without the background
/// showing (mip-wrap residuals at the sprite tiling boundary) are a
/// different issue and do not count.
fn fault_pixels(merged: &image::RgbaImage, plain: &image::RgbaImage) -> Vec<(u32, u32)> {
    let sky_like = |p: &image::Rgba<u8>| p.0[2] as i32 >= p.0[0] as i32 + 30 && p.0[2] > 100;
    let mut out = Vec::new();
    for y in 0..H {
        for x in 0..W {
            let m = merged.get_pixel(x, y);
            if diff(m, plain.get_pixel(x, y)) > DIFF_THRESHOLD && sky_like(m) {
                out.push((x, y));
            }
        }
    }
    out
}

fn camera(pitch: f32) -> Camera {
    Camera {
        pos: glam::DVec3::new(0.5, 2.6, -30.5),
        yaw: 0.0,
        pitch,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    }
}

#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn merged_quad_edges_do_not_crack() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let field = Field {
        grass: data
            .blocks
            .state(&Identifier::minecraft("grass_block"), &[("snowy", "false")])
            .unwrap(),
        snowy_grass: data
            .blocks
            .state(&Identifier::minecraft("grass_block"), &[("snowy", "true")])
            .unwrap(),
        dirt: data
            .blocks
            .default_state(&Identifier::minecraft("dirt"))
            .unwrap(),
        stone: data
            .blocks
            .default_state(&Identifier::minecraft("stone"))
            .unwrap(),
        snow: data
            .blocks
            .state(&Identifier::minecraft("snow"), &[("layers", "2")])
            .unwrap(),
    };
    let mut renderer = Renderer::new_offscreen(W, H, &mut assets, &data.blocks).unwrap();

    for (pitch, name) in [(-3.0f32, "grazing"), (4.0f32, "low"), (12.0f32, "steep")] {
        let merged = render(&mut renderer, &field, camera(pitch), true, name);
        let plain = render(&mut renderer, &field, camera(pitch), false, name);
        let faults = fault_pixels(&merged, &plain);
        eprintln!(
            "{name} view (pitch {pitch}): {} differing pixels",
            faults.len()
        );
        // Per-row fault count and longest horizontal run: cracks are thin
        // horizontal lines (many rows, short runs), holes are blobs.
        let mut row_stats: Vec<(u32, u32, u32)> = Vec::new(); // (row, count, max run)
        let mut run = 0u32;
        for &(x, y) in &faults {
            let fresh = row_stats.last().map(|(r, _, _)| *r != y).unwrap_or(true);
            if fresh {
                row_stats.push((y, 0, 0));
                run = 0;
            }
            let entry = row_stats.last_mut().unwrap();
            entry.1 += 1;
            run = if run == 0 || faults.contains(&(x - 1, y)) {
                run + 1
            } else {
                1
            };
            entry.2 = entry.2.max(run);
        }
        for (y, n, run) in row_stats.iter().take(25) {
            eprintln!("  row {y}: {n} faults, longest run {run}");
        }
        for (x, y) in faults.iter().take(24) {
            let (m, p) = (merged.get_pixel(*x, *y), plain.get_pixel(*x, *y));
            eprintln!(
                "  ({x},{y}): merged {:?} plain {:?}",
                [m.0[0], m.0[1], m.0[2]],
                [p.0[0], p.0[1], p.0[2]]
            );
        }
        assert!(
            faults.is_empty(),
            "{name} view: {} pixels differ between the merged and the unmerged render: {:?}",
            faults.len(),
            &faults[..faults.len().min(20)]
        );
    }
    set_greedy_merge_enabled(true);
}
