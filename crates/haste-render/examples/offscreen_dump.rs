//! Renders a flat world offscreen and saves it to a PNG (dev tool for WS3).
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
        if p.y <= 0 { 0 } else { 15 }
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
    let out = std::env::args().nth(1).unwrap_or_else(|| "offscreen.png".into());
    let jar = haste_core::paths::client_jar();
    let mut assets = haste_assets::Assets::open(
        &jar,
        &haste_core::paths::cache_dir().join("assets"),
        "26.3",
    )?;
    let states = haste_assets::BlocksJsonStates::load(
        &haste_core::paths::datagen_reports().join("blocks.json"),
    )?;
    let mut renderer = haste_render::Renderer::new_offscreen(640, 480, &mut assets, &states)?;
    let dirty: Vec<SectionPos> = (-2..=2)
        .flat_map(|x| (-2..=2).map(move |z| SectionPos::new(x, 0, z)))
        .collect();
    renderer.update_world(&FlatWorld, &dirty, ChunkPos::new(0, 0), 4);
    let frame = haste_render::FrameInput {
        camera: haste_render::Camera {
            pos: glam::DVec3::new(8.0, 24.0, 8.0),
            yaw: 45.0,
            pitch: 55.0,
            fov_deg: 70.0,
        },
        entities: &[],
        gui: &haste_core::gui::GuiFrame::default(),
        sky: haste_render::SkyState {
            time_of_day: 6000,
            rain: 0.0,
            dimension_has_sky: true,
            fog_start: 100.0,
            fog_end: 200.0,
        },
        partial_tick: 0.0,
    };
    let stats = renderer.render(&frame)?;
    println!(
        "frame: {:.2} ms, {} drawn / {} sections, {} draw calls",
        stats.frame_ms, stats.chunk_sections_drawn, stats.chunk_sections_total, stats.draw_calls
    );
    let image = renderer.read_back()?;
    image.save(&out)?;
    println!("saved {out} ({}x{})", image.width(), image.height());
    Ok(())
}
