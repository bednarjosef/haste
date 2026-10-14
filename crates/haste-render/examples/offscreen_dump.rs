//! Renders a flat world offscreen and saves it to a PNG (dev tool for renderer).
//! Run: cargo run -p haste-render --example offscreen_dump [-- out.png]
use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId, ChunkPos, SectionPos};

struct FlatWorld;

impl BlockView for FlatWorld {
    fn block_state(&self, p: BlockPos) -> BlockStateId {
        if p.y == 0 {
            BlockStateId(1) // stone
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

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?),
        )
        .init();
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::cache_dir().join("assets"), "26.3")?;
    let states = haste_assets::BlocksJsonStates::load(
        &haste_core::paths::datagen_reports().join("blocks.json"),
    )?;
    let mut renderer = haste_render::Renderer::new_offscreen(640, 480, &mut assets, &states)?;
    // args: [--distance N] [--out FILE]
    let mut distance = 2i32;
    let mut out = "offscreen.png".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--distance" => {
                distance = args.next().and_then(|v| v.parse().ok()).unwrap_or(2);
            }
            "--out" => {
                out = args.next().unwrap_or_else(|| "offscreen.png".into());
            }
            _ => {}
        }
    }
    let dirty: Vec<SectionPos> = (-distance..=distance)
        .flat_map(|x| (-distance..=distance).map(move |z| SectionPos::new(x, 0, z)))
        .collect();
    renderer.update_world(&FlatWorld, &dirty, ChunkPos::new(0, 0), 4);
    let frame = haste_render::FrameInput {
        camera: haste_render::Camera {
            pos: glam::DVec3::new(8.0, 24.0, 8.0),
            yaw: 45.0,
            pitch: 55.0,
            fov_deg: 70.0,
            roll_deg: 0.0,
            bob: haste_render::ViewBob::default(),
        },
        entities: &[],
        block_entities: &[],
        gui: &haste_core::gui::GuiFrame::default(),
        sky: haste_render::SkyState {
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
    };
    // Warm up the meshing pipeline: keep calling update_world (drain budget
    // 32/frame) until all sections are uploaded, then measure steady state.
    let mut stats = renderer.render(&frame)?;
    for _ in 0..(distance * 2 + 1).pow(2) / 32 + 5 {
        renderer.update_world(&FlatWorld, &dirty, ChunkPos::new(0, 0), distance);
        stats = renderer.render(&frame)?;
    }
    let mut total_ms = 0.0f32;
    let n = 100;
    for _ in 0..n {
        renderer.update_world(&FlatWorld, &[], ChunkPos::new(0, 0), distance);
        stats = renderer.render(&frame)?;
        total_ms += stats.frame_ms;
    }
    println!(
        "steady state over {n} frames: {:.2} ms avg ({:.0} FPS) | drawn {}/{} sections, {} draw calls",
        total_ms / n as f32,
        1000.0 * n as f32 / total_ms,
        stats.chunk_sections_drawn,
        stats.chunk_sections_total,
        stats.draw_calls
    );
    let image = renderer.read_back()?;
    image.save(&out)?;
    println!("saved {out} ({}x{})", image.width(), image.height());
    Ok(())
}
