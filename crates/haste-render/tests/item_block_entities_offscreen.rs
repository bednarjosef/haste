//! Block entities that draw an item or a mob, checked offscreen: a campfire with four items
//! on its grill, a shelf with three items, a vault's spinning gold-block stack, a spawner
//! showing its pig and a brushed suspicious sand with an emerald stuck out of it. Saves
//! `target/render-checks/item-block-entities.png`. Needs the client jar, the datagen reports
//! and a Vulkan driver (source scripts/env.sh).

use haste_assets::Assets;
use haste_core::model::Cube;
use haste_core::{
    BlockPos, BlockStateId, ChunkPos, Direction, Identifier, RegistryIndex, SectionPos,
};
use haste_render::blockentity::{BlockEntityInstance, BlockEntityKind};
use haste_render::blockentity::{
    BrushableItem, CampfireItems, ShelfItems, SpawnerDisplay, VaultDisplay,
};
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

fn item_index(data: &haste_data::GameData, name: &str) -> RegistryIndex {
    data.registries
        .index("minecraft:item", &Identifier::minecraft(name))
        .unwrap()
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
        partial_tick: 0.5,
        block_target: None,
        breaking: &[],
        camera_entity: None,
        spin: None,
        weather: None,
    };
    renderer.render(&frame).unwrap();
    renderer.read_back().unwrap()
}

/// A small pig model (a body box and a head box), enough for the spawner's display entity.
fn pig_model() -> &'static haste_core::model::CubeModel {
    use std::sync::OnceLock;
    static MODEL: OnceLock<haste_core::model::CubeModel> = OnceLock::new();
    MODEL.get_or_init(|| haste_core::model::CubeModel {
        texture: Identifier::minecraft("haste:test"),
        tex_width: 64,
        tex_height: 64,
        root: haste_core::model::ModelPart {
            name: "root".into(),
            pivot: glam::Vec3::ZERO,
            rotation: glam::Vec3::ZERO,
            visible: true,
            cubes: vec![Cube {
                origin: glam::Vec3::new(-4.0, -12.0, -8.0),
                size: glam::Vec3::new(8.0, 8.0, 16.0),
                inflate: 0.0,
                uv: [28, 8],
                mirror: false,
            }],
            children: vec![],
        },
    })
}

#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn item_and_mob_block_entities_offscreen() {
    let jar = haste_core::paths::client_jar();
    let mut assets = Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let mut renderer = Renderer::new_offscreen(640, 480, &mut assets, &data.blocks).unwrap();
    renderer.set_item_registry(&data.registries);
    let world = Flat {
        stone: data
            .blocks
            .default_state(&Identifier::minecraft("stone"))
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

    let instance = |pos: BlockPos, facing: Direction, kind: BlockEntityKind| BlockEntityInstance {
        pos,
        facing,
        half: haste_render::ChestHalf::Single,
        kind,
        open: TickPair::fixed(0.0),
        light: (15, 15),
        sign_angle: 0.0,
        text: None,
        banner_phase: 0.0,
        banner: None,
        skull_anim: TickPair::fixed(0.0),
        anim: None,
        sherds: None,
    };

    let campfire = instance(
        BlockPos::new(0, 1, 0),
        Direction::South,
        BlockEntityKind::Campfire(CampfireItems {
            items: [
                Some(item_index(&data, "diamond")),
                Some(item_index(&data, "gold_ingot")),
                Some(item_index(&data, "emerald")),
                Some(item_index(&data, "iron_ingot")),
            ],
        }),
    );
    let shelf = instance(
        BlockPos::new(2, 1, 0),
        Direction::North,
        BlockEntityKind::Shelf(ShelfItems {
            items: [
                Some(item_index(&data, "book")),
                Some(item_index(&data, "apple")),
                Some(item_index(&data, "bread")),
            ],
            align_to_bottom: false,
        }),
    );
    let vault = instance(
        BlockPos::new(4, 1, 0),
        Direction::North,
        BlockEntityKind::Vault(VaultDisplay {
            item: item_index(&data, "gold_block"),
            count: 17,
            spin: TickPair::fixed(0.0),
        }),
    );
    let spawner = instance(
        BlockPos::new(6, 1, 0),
        Direction::North,
        BlockEntityKind::Spawner(SpawnerDisplay {
            model: Some(std::sync::Arc::new(pig_model().clone())),
            spin: TickPair::fixed(0.0),
            scale: haste_render::blockentity::SPAWNER_SCALE,
        }),
    );
    let brushable = instance(
        BlockPos::new(8, 1, 0),
        Direction::North,
        BlockEntityKind::Brushable(BrushableItem {
            item: item_index(&data, "emerald"),
            hit_direction: Some(Direction::North),
            dusted: 2,
        }),
    );

    let camera = Camera {
        pos: glam::DVec3::new(-4.0, 2.6, -6.0),
        yaw: 0.0,
        pitch: 12.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    };
    for _ in 0..4 {
        render(&mut renderer, &world, &[], camera);
    }
    let img = render(
        &mut renderer,
        &world,
        &[campfire, shelf, vault, spawner, brushable],
        camera,
    );
    let dir = std::path::Path::new("target/render-checks");
    std::fs::create_dir_all(dir).unwrap();
    img.save(dir.join("item-block-entities.png")).unwrap();

    // Every draw must leave pixels behind: the diamond and gold are the eye-catchers, and the
    // pig model is grey.
    let (mut cyan, mut yellow, mut green) = (0u32, 0u32, 0u32);
    for p in img.pixels() {
        let [r, g, b, _] = p.0;
        let (r, g, b) = (r as i32, g as i32, b as i32);
        if b > 100 && b - r > 30 && g > r {
            cyan += 1;
        }
        if r > 130 && g > 110 && b < r - 40 && g > b {
            yellow += 1;
        }
        if g > 70 && g - r > 30 && g - b > 30 {
            green += 1;
        }
    }
    println!("cyan {cyan}, yellow {yellow}, green {green}");
    assert!(cyan > 60, "diamond pixels: {cyan}");
    assert!(yellow > 60, "gold pixels: {yellow}");
    assert!(green > 30, "emerald pixels: {green}");
}

/// Frame timing with 50 campfires holding four items each, against the same 50 campfires
/// empty: the item models must cost the frame path nothing measurable beyond the batching
/// the block entity renderer already did for its own texture.
#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn fifty_campfire_items_frame_timing() {
    let jar = haste_core::paths::client_jar();
    let mut assets = Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let mut renderer = Renderer::new_offscreen(960, 540, &mut assets, &data.blocks).unwrap();
    renderer.set_item_registry(&data.registries);
    let world = Flat {
        stone: data
            .blocks
            .default_state(&Identifier::minecraft("stone"))
            .unwrap(),
    };
    let dirty: Vec<SectionPos> = (-2..=2)
        .flat_map(|x| (-2..=2).map(move |z| SectionPos::new(x, 0, z)))
        .collect();
    renderer.update_world(&world, &dirty, ChunkPos::new(0, 0), 12);
    for _ in 0..300 {
        renderer.update_world(&world, &[], ChunkPos::new(0, 0), 12);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // A 25 x 2 grid of campfires in front of the camera, every one holding the same four
    // items: 200 item models on the grill in view.
    let campfires = |items: bool| -> Vec<BlockEntityInstance> {
        (0..50)
            .map(|i| BlockEntityInstance {
                pos: BlockPos::new(-12 + i % 25, 1, 4 + i / 25),
                facing: Direction::South,
                half: haste_render::ChestHalf::Single,
                kind: BlockEntityKind::Campfire(CampfireItems {
                    items: if items {
                        [
                            Some(item_index(&data, "diamond")),
                            Some(item_index(&data, "gold_ingot")),
                            Some(item_index(&data, "emerald")),
                            Some(item_index(&data, "iron_ingot")),
                        ]
                    } else {
                        [None; 4]
                    },
                }),
                open: TickPair::fixed(0.0),
                light: (15, 15),
                sign_angle: 0.0,
                text: None,
                banner_phase: 0.0,
                banner: None,
                skull_anim: TickPair::fixed(0.0),
                anim: None,
                sherds: None,
            })
            .collect()
    };
    let camera = Camera {
        pos: glam::DVec3::new(0.5, 2.62, 0.5),
        yaw: 0.0,
        pitch: 10.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    };
    for (label, items) in [("empty", false), ("four items", true)] {
        let instances = campfires(items);
        let gui = haste_core::gui::GuiFrame::default();
        let sky = SkyState {
            time_of_day: 6000,
            rain: 0.0,
            dimension_has_sky: true,
            fog_start: 100.0,
            fog_end: 200.0,
            ..Default::default()
        };
        // Warm up, then time 100 frames.
        for _ in 0..10 {
            renderer
                .render(&FrameInput {
                    camera,
                    entities: &[],
                    block_entities: &instances,
                    gui: &gui,
                    sky,
                    partial_tick: 0.5,
                    block_target: None,
                    breaking: &[],
                    camera_entity: None,
                    spin: None,
                    weather: None,
                })
                .unwrap();
        }
        let mut worst = 0.0f32;
        let mut sum = 0.0f32;
        let mut drawn = 0;
        for _ in 0..100 {
            let stats = renderer
                .render(&FrameInput {
                    camera,
                    entities: &[],
                    block_entities: &instances,
                    gui: &gui,
                    sky,
                    partial_tick: 0.5,
                    block_target: None,
                    breaking: &[],
                    camera_entity: None,
                    spin: None,
                    weather: None,
                })
                .unwrap();
            worst = worst.max(stats.frame_ms);
            sum += stats.frame_ms;
            drawn = stats.block_entities_drawn;
        }
        let t = renderer.timings();
        println!(
            "50 campfires {label}: drawn {drawn}, frame avg {:.3} ms, worst {:.3} ms; \
             prepare {:.3} ms, cull {:.3} ms, encode {:.3} ms, submit {:.3} ms",
            sum / 100.0,
            worst,
            t.prepare_ms,
            t.cull_ms,
            t.encode_ms,
            t.submit_ms,
        );
    }
}
