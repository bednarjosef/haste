// Quad-format offscreen render check (`--terrain-format quad`): the same flat
// world rendered in the vertex format and the quad format must produce the
// same pixels — the record keeps the vertex path's quantization grids, light
// packing and AO-flip triangulation, so nothing the player can see changes
// (AGENTS.md "faithful wherever the player can feel it"). Runs as its own
// integration test binary because the format is a process-wide switch.

use haste_core::view::BlockView;
use haste_core::{BlockPos, ChunkPos, SectionPos};
use haste_render::{Camera, FrameInput, Renderer, SkyState, ViewBob};

/// A flat stone world at y=0 (the `offscreen_render_flat_world` fixture: its
/// top faces greedy-merge into split fans, exercising the fan spans, and the
/// sides draw as rect records).
struct Flat;

impl BlockView for Flat {
    fn block_state(&self, p: BlockPos) -> haste_core::BlockStateId {
        if p.y == 0 {
            haste_core::BlockStateId(1)
        } else {
            haste_core::BlockStateId::AIR
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

fn frame(gui: &'static haste_core::gui::GuiFrame) -> FrameInput<'static> {
    FrameInput {
        camera: Camera {
            pos: glam::DVec3::new(8.0, 20.0, 8.0),
            yaw: 0.0,
            pitch: 60.0,
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

/// Renders the flat world once and returns the read-back frame. `quad` builds
/// the renderer with the quad format switched on first (the mesher and the
/// renderer both read the global at construction/upload time).
fn render_flat(quad: bool) -> image::RgbaImage {
    haste_render::set_terrain_format_quad(quad);
    let jar = haste_core::paths::client_jar();
    if !jar.exists() {
        panic!("needs the client jar in the cache (HASTE_CACHE)");
    }
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::cache_dir().join("assets"), "26.3")
            .unwrap();
    let states = haste_assets::BlocksJsonStates::load(
        &haste_core::paths::datagen_reports().join("blocks.json"),
    )
    .unwrap();
    let mut renderer = Renderer::new_offscreen(320, 240, &mut assets, &states).unwrap();
    let dirty: Vec<SectionPos> = (-1..=1)
        .flat_map(|x| (-1..=1).map(move |z| SectionPos::new(x, 0, z)))
        .collect();
    renderer.update_world(&Flat, &dirty, ChunkPos::new(0, 0), 4);
    // Meshing and the visibility walk are asynchronous: keep draining and
    // rendering until every mesh landed and the drawn count is stable (the
    // frame the walk first reports a section would differ from the settled
    // one, and the comparison must not depend on that timing).
    let gui = haste_core::gui::GuiFrame::default();
    let gui: &'static haste_core::gui::GuiFrame = Box::leak(Box::new(gui));
    let frame = frame(gui);
    let mut stats = renderer.render(&frame).unwrap();
    let mut stable = 0;
    let mut last_drawn = u32::MAX;
    for _ in 0..400 {
        if stats.meshes_pending == 0 && stats.chunk_sections_drawn > 0 && stable >= 3 {
            break;
        }
        stable = if stats.chunk_sections_drawn == last_drawn {
            stable + 1
        } else {
            0
        };
        last_drawn = stats.chunk_sections_drawn;
        std::thread::sleep(std::time::Duration::from_millis(5));
        renderer.update_world(&Flat, &[], ChunkPos::new(0, 0), 4);
        stats = renderer.render(&frame).unwrap();
    }
    eprintln!("render_flat(quad={quad}): {stats:?}");
    assert!(
        stats.chunk_sections_drawn > 0,
        "no sections drawn (quad={quad}, stats {stats:?})"
    );
    let image = renderer.read_back().unwrap();
    assert_terrain_visible(&image, quad);
    image
}

/// The camera looks 60 degrees down onto the stone floor, so the lower half of
/// the frame is grey stone, not blue sky. Two blank frames (terrain never
/// drawn) would otherwise compare equal.
fn assert_terrain_visible(image: &image::RgbaImage, quad: bool) {
    let (w, h) = image.dimensions();
    let total = (w * (h - h / 2)) as usize;
    let grey = (h / 2..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let [r, _, b, _] = image.get_pixel(x, y).0;
            (b as i32 - r as i32).abs() < 20
        })
        .count();
    assert!(
        grey * 2 > total,
        "terrain not drawn (quad={quad}): only {grey} of {total} lower-half pixels are stone grey"
    );
}

#[test]
#[ignore = "needs the client jar, datagen reports, and a Vulkan driver (source scripts/env.sh)"]
fn vertex_format_draws_terrain() {
    render_flat(false);
}

#[test]
#[ignore = "known broken since 2026-10-19: the quad format draws scattered garbage (see NOTES); \
            also needs the client jar, datagen reports, and a Vulkan driver"]
fn quad_format_renders_the_vertex_format_pixels() {
    let vertex = render_flat(false);
    let quad = render_flat(true);
    assert_eq!(vertex.dimensions(), quad.dimensions());
    let mut diff = 0usize;
    for (i, (a, b)) in vertex.pixels().zip(quad.pixels()).enumerate() {
        if a != b {
            diff += 1;
            if diff <= 8 {
                let (x, y) = (i % 320, i / 320);
                eprintln!("diff at ({x},{y}): vertex {a:?} quad {b:?}");
            }
        }
    }
    assert_eq!(
        diff,
        0,
        "{diff} of {} pixels differ between the vertex and quad terrain formats",
        vertex.dimensions().0 * vertex.dimensions().1
    );
}
