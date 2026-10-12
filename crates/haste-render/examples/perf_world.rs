//! Frame time on a real world: joins a server (offline mode), loads every chunk it sends into a
//! `haste_world::World`, meshes all of it, then renders offscreen from the spawn point looking
//! in four directions and prints the average frame time and its parts.
//!
//! cargo run --release -p haste-render --example perf_world -- 127.0.0.1:25613 12 [width height]
//! `synthetic` instead of an address renders generated hilly terrain (stone, dirt, grass, sand,
//! water, caves, ores) without a server.
//! (needs the cache and a Vulkan driver: source scripts/env.sh)

use std::time::{Duration, Instant};

use haste_core::view::DirtyTracker;
use haste_core::{ChunkPos, SectionPos};
use haste_protocol::{Account, ClientboundPlay, ConnectOptions};
use haste_render::{Camera, FrameInput, Renderer, SkyState};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();
    let args: Vec<String> = std::env::args().collect();
    let address = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "127.0.0.1:25613".into());
    let view_distance: i32 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(12);
    let width: u32 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(1280);
    let height: u32 = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(720);

    let data = std::sync::Arc::new(haste_data::GameData::load()?);
    if address == "synthetic" {
        let world = Synthetic::new(&data);
        let r = view_distance;
        let dirty: Vec<SectionPos> = (-r..=r)
            .flat_map(|x| (-r..=r).map(move |z| (x, z)))
            .filter(|(x, z)| x * x + z * z <= r * r)
            .flat_map(|(x, z)| (-4..8).map(move |y| SectionPos::new(x, y, z)))
            .collect();
        let pos = glam::DVec3::new(0.5, world.height_at(0, 0) as f64 + 1.0, 0.5);
        return measure(
            &world,
            dirty,
            pos,
            view_distance,
            width,
            height,
            &data,
            "synthetic",
        );
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .build()?;
    let mut opts = ConnectOptions::new(
        address.clone(),
        Account::Offline {
            username: "perf".into(),
        },
    );
    opts.client_information.view_distance = view_distance as i8;
    let (mut conn, login) = rt.block_on(haste_protocol::Connection::connect(opts))?;
    let mut world = haste_world::World::new_with_biomes(
        login.dimension.clone(),
        data.clone(),
        conn.registries()
            .entries(&haste_core::Identifier::minecraft("worldgen/biome"))
            .map_or(64, |e| e.len()),
    );
    println!(
        "joined {address}, view distance {view_distance}, server view distance {}",
        login.view_distance
    );

    // Receive until no new chunk arrived for 3 s (or 90 s in total).
    let mut pos = glam::DVec3::new(0.0, 100.0, 0.0);
    let started = Instant::now();
    let mut last_chunk = Instant::now();
    let mut chunks = 0;
    while last_chunk.elapsed() < Duration::from_secs(3)
        && started.elapsed() < Duration::from_secs(90)
    {
        let _guard = rt.enter();
        match conn.try_recv() {
            Some(ClientboundPlay::LevelChunkWithLight(c)) => {
                if world.load_chunk(&c.chunk).is_ok() {
                    world.apply_light(c.chunk.pos, &c.chunk.light);
                    chunks += 1;
                }
                last_chunk = Instant::now();
            }
            Some(ClientboundPlay::LightUpdate(l)) => world.apply_light(l.pos, &l.light),
            Some(ClientboundPlay::PlayerPosition(p)) => pos = p.pos,
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(2)),
        }
    }
    println!(
        "{chunks} chunks in {:.1} s, player at {pos:.1}",
        started.elapsed().as_secs_f32()
    );

    let dirty: Vec<SectionPos> = world.take_dirty_sections();
    measure(
        &world,
        dirty,
        pos,
        view_distance,
        width,
        height,
        &data,
        "server",
    )?;
    conn.disconnect();
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn measure(
    world: &dyn haste_core::view::BlockView,
    dirty: Vec<SectionPos>,
    pos: glam::DVec3,
    view_distance: i32,
    width: u32,
    height: u32,
    data: &haste_data::GameData,
    label: &str,
) -> anyhow::Result<()> {
    let jar = haste_core::paths::client_jar();
    let mut assets = haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3")?;
    let mut renderer = Renderer::new_offscreen(width, height, &mut assets, &data.blocks)?;
    renderer.set_item_registry(&data.registries);
    let camera_chunk = ChunkPos::new((pos.x.floor() as i32) >> 4, (pos.z.floor() as i32) >> 4);
    let t_mesh = Instant::now();
    renderer.update_world(world, &dirty, camera_chunk, view_distance);
    let gui = haste_core::gui::GuiFrame::default();
    let sky = SkyState {
        time_of_day: 6000,
        rain: 0.0,
        dimension_has_sky: true,
        fog_start: (view_distance * 16) as f32 * 0.9,
        fog_end: (view_distance * 16) as f32,
    };
    let frame = |yaw: f32| FrameInput {
        camera: Camera {
            pos: pos + glam::DVec3::new(0.0, 1.62, 0.0),
            yaw,
            pitch: 10.0,
            fov_deg: 70.0,
        },
        entities: &[],
        gui: &gui,
        sky,
        partial_tick: 0.0,
        block_target: None,
        breaking: &[],
    };
    // Mesh everything (drain uploads frame by frame) until the resident section count has not
    // changed for a second.
    let mut last_total = 0;
    let mut stable_since = Instant::now();
    loop {
        renderer.update_world(world, &[], camera_chunk, view_distance);
        let s = renderer.render(&frame(0.0))?;
        if s.chunk_sections_total != last_total || s.meshes_pending > 0 {
            last_total = s.chunk_sections_total;
            stable_since = Instant::now();
        }
        if stable_since.elapsed() > Duration::from_secs(1) {
            println!(
                "meshed {} dirty sections -> {} section meshes in {:.2} s",
                dirty.len(),
                s.chunk_sections_total,
                t_mesh.elapsed().as_secs_f32()
            );
            break;
        }
    }
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    std::fs::create_dir_all(&out)?;
    for yaw in [0.0f32, 90.0, 180.0, 270.0] {
        // Warm up, then measure the submit-side frame time and the full GPU round trip.
        for _ in 0..5 {
            renderer.render(&frame(yaw))?;
        }
        let n = 60;
        let mut cpu = 0.0;
        let mut drawn = 0;
        let mut calls = 0;
        let t = Instant::now();
        for _ in 0..n {
            let s = renderer.render(&frame(yaw))?;
            cpu += s.frame_ms;
            drawn = s.chunk_sections_drawn;
            calls = s.draw_calls;
        }
        let img = renderer.read_back()?; // waits for the GPU
        let wall = t.elapsed().as_secs_f32() * 1000.0 / n as f32;
        img.save(out.join(format!(
            "perf_{label}_rd{view_distance}_yaw{}.png",
            yaw as i32
        )))?;
        println!(
            "rd {view_distance} yaw {yaw:>3}: {wall:.2} ms/frame wall ({:.0} FPS), {:.2} ms CPU submit, {drawn} sections drawn, {calls} draw calls",
            1000.0 / wall,
            cpu / n as f32
        );
    }
    Ok(())
}

/// Generated terrain: rolling hills from summed sines, stone/dirt/grass, sand and water at sea
/// level 62, sinusoidal caves, scattered ores. Sky light 15 above the surface, 0 below.
struct Synthetic {
    stone: haste_core::BlockStateId,
    dirt: haste_core::BlockStateId,
    grass: haste_core::BlockStateId,
    sand: haste_core::BlockStateId,
    water: haste_core::BlockStateId,
    ore: haste_core::BlockStateId,
}

impl Synthetic {
    fn new(data: &haste_data::GameData) -> Synthetic {
        let s = |n: &str| {
            data.blocks
                .default_state(&haste_core::Identifier::minecraft(n))
                .unwrap()
        };
        Synthetic {
            stone: s("stone"),
            dirt: s("dirt"),
            grass: data
                .blocks
                .state(
                    &haste_core::Identifier::minecraft("grass_block"),
                    &[("snowy", "false")],
                )
                .unwrap(),
            sand: s("sand"),
            water: s("water"),
            ore: s("iron_ore"),
        }
    }

    fn height_at(&self, x: i32, z: i32) -> i32 {
        let (x, z) = (x as f32, z as f32);
        let h = 66.0
            + 10.0 * (x * 0.021).sin() * (z * 0.017).cos()
            + 5.0 * (x * 0.067 + z * 0.041).sin()
            + 2.0 * (z * 0.13).sin();
        h as i32
    }
}

impl haste_core::view::BlockView for Synthetic {
    fn block_state(&self, p: haste_core::BlockPos) -> haste_core::BlockStateId {
        let h = self.height_at(p.x, p.z);
        if p.y > h {
            return if p.y <= 62 {
                self.water
            } else {
                haste_core::BlockStateId::AIR
            };
        }
        let (fx, fy, fz) = (p.x as f32, p.y as f32, p.z as f32);
        if p.y < h - 3
            && p.y > -60
            && ((fx * 0.09).sin() + (fy * 0.13).sin() + (fz * 0.08).cos()) > 1.6
        {
            return haste_core::BlockStateId::AIR;
        }
        if p.y == h {
            if h <= 63 {
                self.sand
            } else {
                self.grass
            }
        } else if p.y > h - 4 {
            if h <= 63 {
                self.sand
            } else {
                self.dirt
            }
        } else if (p.x.wrapping_mul(73856093)
            ^ p.y.wrapping_mul(19349663)
            ^ p.z.wrapping_mul(83492791))
        .rem_euclid(97)
            == 0
        {
            self.ore
        } else {
            self.stone
        }
    }
    fn is_chunk_loaded(&self, _: ChunkPos) -> bool {
        true
    }
    fn sky_light(&self, p: haste_core::BlockPos) -> u8 {
        if p.y > self.height_at(p.x, p.z) {
            15
        } else {
            0
        }
    }
    fn block_light(&self, _: haste_core::BlockPos) -> u8 {
        0
    }
    fn min_y(&self) -> i32 {
        -64
    }
    fn height(&self) -> i32 {
        384
    }
}
