//! Terrain far from the origin: a grass plain around (1000, 64, -2000), seen at pitch 0 and
//! looking down, in a 300x200 target (rows not 256-byte aligned). Checks the camera-relative
//! view, frustum culling, texture namespaces (no missing sprite) and read-back padding.
//! Saves `target/render-checks/world_far_*.png`. Needs the client jar, the datagen reports
//! and a Vulkan driver (source scripts/env.sh).

use haste_core::{BlockPos, BlockStateId, ChunkPos, SectionPos};
use haste_render::{Camera, FrameInput, Renderer, SkyState};

const X0: i32 = 1000;
const Z0: i32 = -2000;

struct Plain {
    grass: BlockStateId,
    dirt: BlockStateId,
}

impl haste_core::view::BlockView for Plain {
    fn block_state(&self, p: BlockPos) -> BlockStateId {
        match p.y {
            63 => self.grass,
            60..=62 => self.dirt,
            _ => BlockStateId::AIR,
        }
    }
    fn is_chunk_loaded(&self, _: ChunkPos) -> bool {
        true
    }
    fn sky_light(&self, p: BlockPos) -> u8 {
        if p.y >= 64 {
            15
        } else {
            0
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
fn terrain_far_from_origin() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let state = |name: &str| {
        data.blocks
            .default_state(&haste_core::Identifier::minecraft(name))
            .unwrap()
    };
    let world = Plain {
        grass: data
            .blocks
            .state(
                &haste_core::Identifier::minecraft("grass_block"),
                &[("snowy", "false")],
            )
            .unwrap(),
        dirt: state("dirt"),
    };
    let (w, h) = (300, 200);
    let mut renderer = Renderer::new_offscreen(w, h, &mut assets, &data.blocks).unwrap();
    let centre = SectionPos::new(X0 >> 4, 63 >> 4, Z0 >> 4);
    let dirty: Vec<SectionPos> = (-4..=4)
        .flat_map(|dx| {
            (-4..=4).map(move |dz| SectionPos::new(centre.x + dx, centre.y, centre.z + dz))
        })
        .collect();
    renderer.update_world(&world, &dirty, ChunkPos::new(centre.x, centre.z), 4);
    for _ in 0..300 {
        renderer.update_world(&world, &[], ChunkPos::new(centre.x, centre.z), 4);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let gui = haste_core::gui::GuiFrame::default();
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    std::fs::create_dir_all(&out).unwrap();
    for (name, pitch) in [("level", 0.0f32), ("down", 45.0)] {
        let frame = FrameInput {
            camera: Camera {
                pos: glam::DVec3::new(X0 as f64 + 0.5, 65.62, Z0 as f64 + 0.5),
                yaw: 30.0,
                pitch,
                fov_deg: 70.0,
            },
            entities: &[],
            gui: &gui,
            sky: SkyState {
                time_of_day: 6000,
                rain: 0.0,
                dimension_has_sky: true,
                fog_start: 48.0,
                fog_end: 64.0,
            },
            partial_tick: 0.0,
        };
        let stats = renderer.render(&frame).unwrap();
        let img = renderer.read_back().unwrap();
        assert_eq!(img.dimensions(), (w, h));
        img.save(out.join(format!("world_far_{name}.png"))).unwrap();
        eprintln!(
            "{name}: {} of {} sections drawn, {} draw calls",
            stats.chunk_sections_drawn, stats.chunk_sections_total, stats.draw_calls
        );
        assert!(stats.chunk_sections_drawn > 0, "{name}: nothing drawn");
        assert!(
            stats.chunk_sections_drawn < stats.chunk_sections_total || pitch > 0.0,
            "{name}: frustum culls something behind the camera"
        );
        // No magenta (missing sprite) anywhere.
        let magenta = img
            .pixels()
            .filter(|p| p.0[0] > 200 && p.0[1] < 40 && p.0[2] > 200)
            .count();
        assert_eq!(magenta, 0, "{name}: missing-texture pixels");
        // The bottom row is the grass top (grey until biome tint lands: the texture is
        // greyscale), not sky blue; the top row at level pitch is sky.
        let bottom = img.get_pixel(w / 2, h - 2).0;
        assert!(
            bottom[2] <= bottom[0] + 10 && bottom[0] > 60,
            "{name}: terrain at the bottom {bottom:?}"
        );
        if pitch == 0.0 {
            let top = img.get_pixel(w / 2, 2).0;
            assert!(top[2] > top[1] && top[2] > top[0], "sky at the top {top:?}");
            // The horizon is in the middle: the grass ends below it, the camera is 1.62 above
            // the grass, so the boundary sits just under the centre row.
            let mid_above = img.get_pixel(w / 2, h / 2 - 10).0;
            assert!(
                mid_above[2] > mid_above[1],
                "sky just above the centre {mid_above:?}"
            );
        }
    }
}

#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn sky_times_of_day() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let world = Plain {
        grass: data
            .blocks
            .state(
                &haste_core::Identifier::minecraft("grass_block"),
                &[("snowy", "false")],
            )
            .unwrap(),
        dirt: data
            .blocks
            .default_state(&haste_core::Identifier::minecraft("dirt"))
            .unwrap(),
    };
    let (w, h) = (320, 240);
    let mut renderer = Renderer::new_offscreen(w, h, &mut assets, &data.blocks).unwrap();
    let centre = SectionPos::new(X0 >> 4, 63 >> 4, Z0 >> 4);
    let dirty: Vec<SectionPos> = (-4..=4)
        .flat_map(|dx| {
            (-4..=4).map(move |dz| SectionPos::new(centre.x + dx, centre.y, centre.z + dz))
        })
        .collect();
    renderer.update_world(&world, &dirty, ChunkPos::new(centre.x, centre.z), 4);
    for _ in 0..300 {
        renderer.update_world(&world, &[], ChunkPos::new(centre.x, centre.z), 4);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let gui = haste_core::gui::GuiFrame::default();
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    std::fs::create_dir_all(&out).unwrap();
    let mut tops = Vec::new();
    // (name, time, rain, yaw, pitch): yaw -90 looks east (+x), 90 west.
    for (name, time, rain, yaw, pitch) in [
        ("noon_up", 6000i64, 0.0f32, -90.0f32, -60.0f32),
        ("sunrise_east", 23300, 0.0, -90.0, -5.0),
        ("sunset_west", 12700, 0.0, 90.0, -5.0),
        ("night", 18000, 0.0, 0.0, -30.0),
        ("rain", 6000, 1.0, 0.0, 10.0),
        ("clouds", 6000, 0.0, 0.0, -80.0),
    ] {
        let frame = FrameInput {
            camera: Camera {
                pos: glam::DVec3::new(X0 as f64 + 0.5, 65.62, Z0 as f64 + 0.5),
                yaw,
                pitch,
                fov_deg: 70.0,
            },
            entities: &[],
            gui: &gui,
            sky: SkyState {
                time_of_day: time,
                rain,
                dimension_has_sky: true,
                fog_start: 48.0,
                fog_end: 64.0,
            },
            partial_tick: 0.0,
        };
        renderer.render(&frame).unwrap();
        let img = renderer.read_back().unwrap();
        img.save(out.join(format!("sky_{name}.png"))).unwrap();
        // Sampled a little above the horizon (the sunrise glow sits low).
        tops.push((
            name,
            img.get_pixel(w / 2, if pitch < -20.0 { 10 } else { h / 2 - 30 })
                .0,
        ));
    }
    eprintln!("{tops:?}");
    let get = |n: &str| tops.iter().find(|t| t.0 == n).unwrap().1;
    let lum = |p: [u8; 4]| p[0] as u32 + p[1] as u32 + p[2] as u32;
    assert!(
        lum(get("night")) < lum(get("noon_up")) / 4,
        "night sky is dark"
    );
    let sunrise = get("sunrise_east");
    assert!(sunrise[0] > sunrise[2], "sunrise glow is warm: {sunrise:?}");
}
