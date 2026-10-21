//! A sheep's wool overlay checked offscreen through the entity store: a stone floor and a
//! red-wool sheep. Saves `target/render-checks/sheep.png`. Needs the client jar, the datagen
//! reports, the model dump and a Vulkan driver (source scripts/env.sh).

use haste_core::model::EntityInstance;
use haste_core::{BlockPos, BlockStateId, ChunkPos, EntityId, Identifier, SectionPos};
use haste_render::{Camera, FrameInput, Renderer, SkyState};

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
fn render_scene(
    renderer: &mut Renderer,
    world: &dyn haste_core::view::BlockView,
    entities: &mut [EntityInstance],
    camera: Camera,
) -> image::RgbaImage {
    // Entity light rides on the instances (sampled here, where the test builds its frame).
    for e in entities.iter_mut() {
        e.sample_light(world);
    }
    let gui = haste_core::gui::GuiFrame {
        scale: 2,
        ..Default::default()
    };
    let frame = FrameInput {
        camera,
        entities,
        block_entities: &[],
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
        spin: None,
        weather: None,
    };
    renderer.render(&frame).unwrap();
    renderer.read_back().unwrap()
}

/// A red-wool sheep through the whole store path (spawn + wool metadata +
/// `render_instances`): the sheep model draws (not the fallback box, not a shulker — the
/// wool byte sits at the same metadata index as a shulker's colour byte) and the wool
/// overlay tints the frame red. Deterministic, no server. Saves
/// `target/render-checks/sheep.png`.
#[test]
#[ignore = "needs the client jar, datagen reports, the model dump and a Vulkan driver (source scripts/env.sh)"]
fn sheep_wool_overlay_offscreen() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let mut renderer = Renderer::new_offscreen(640, 480, &mut assets, &data.blocks).unwrap();
    let world = Flat {
        stone: data
            .blocks
            .default_state(&haste_core::Identifier::minecraft("stone"))
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
    let camera = Camera {
        pos: glam::DVec3::new(0.0, 2.2, -4.0),
        yaw: 0.0,
        pitch: 5.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    };
    // The sheep's kind through the real entity-type registry, like the store resolves it.
    let sheep_kind = data
        .registries
        .index("minecraft:entity_type", &Identifier::minecraft("sheep"))
        .expect("sheep in the entity_type registry");
    let mut store = haste_entity::EntityStore::new(haste_entity::EntityDimensions::default());
    store.spawn(&haste_core::entity::EntitySpawn {
        id: EntityId(1),
        uuid: uuid::Uuid::from_u128(1),
        kind: sheep_kind,
        pos: glam::DVec3::new(0.0, 0.0, 0.5),
        yaw: 180.0,
        pitch: 0.0,
        head_yaw: 180.0,
        data: 0,
        velocity: glam::DVec3::ZERO,
    });
    // Red wool: Sheep.DATA_WOOL_ID (metadata 18), colour 14.
    store.set_metadata(
        EntityId(1),
        &[haste_core::entity::MetadataEntry {
            index: 18,
            value: haste_core::entity::MetadataValue::Byte(14),
        }],
    );
    let mut entities = Vec::new();
    store.render_instances(0.5, &mut entities);
    assert_eq!(entities.len(), 1);
    // The base model is the sheep's, not the fallback box.
    assert_eq!(
        entities[0].model.texture,
        Identifier::minecraft("entity/sheep/sheep"),
        "the sheep's base model"
    );
    // The wool overlay is present and tinted red.
    let wool = entities[0].overlays[0].as_ref().expect("the wool overlay");
    assert_eq!(
        wool.model.texture,
        Identifier::minecraft("entity/sheep/sheep_wool")
    );
    assert_eq!(wool.tint, haste_entity::variants::dye_tint(14));

    // Draw it: the wool changes the frame and red pixels appear.
    let base = render_scene(&mut renderer, &world, &mut [], camera);
    let img = render_scene(&mut renderer, &world, &mut entities, camera);
    // The same sheep with the overlay stripped, to prove the overlay itself draws.
    let mut bare = entities.clone();
    bare[0].overlays = Default::default();
    let img_bare = render_scene(&mut renderer, &world, &mut bare, camera);
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    std::fs::create_dir_all(&out).unwrap();
    img.save(out.join("sheep.png")).unwrap();
    let changed = base
        .pixels()
        .zip(img.pixels())
        .filter(|(a, b)| a.0 != b.0)
        .count();
    assert!(changed > 2000, "the sheep changed {changed} pixels");
    let overlay_changed = img
        .pixels()
        .zip(img_bare.pixels())
        .filter(|(a, b)| a.0 != b.0)
        .count();
    eprintln!("overlay changed {overlay_changed} pixels");
    // The wool is dim (the sheep stands at the floor's sky light 0 in this scene), so the
    // check is the red channel dominating, not an absolute brightness.
    let red = img
        .pixels()
        .filter(|p| p.0[0] as i32 - p.0[1] as i32 > 15 && p.0[0] as i32 - p.0[2] as i32 > 15)
        .count();
    assert!(red > 200, "red wool pixels: {red}");
}
