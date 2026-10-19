//! Boats checked offscreen: an oak boat, a cherry chest boat and a bamboo raft posed on a
//! stone floor, plus the water patch's depth-only draw. Saves
//! `target/render-checks/boats.png`. Needs the client jar, the datagen reports, the model
//! dump and a Vulkan driver (source scripts/env.sh).

use haste_core::model::{BodyTransform, EntityInstance, PoseOverrides};
use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId, ChunkPos, EntityId, SectionPos};
use haste_render::{Camera, FrameInput, Renderer, SkyState};

struct Flat {
    stone: BlockStateId,
}

impl BlockView for Flat {
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

/// The instance `haste-entity`'s store builds for a floating boat of `type_name`: the dumped
/// model, the water patch overlay (never a raft's), the 0.375 lift and the entity's yaw.
fn boat_instance(id: i32, type_name: &str, pos: glam::DVec3, yaw: f32) -> EntityInstance {
    let entry = haste_entity::boat::entry_for_name(type_name).unwrap();
    let model = haste_entity::boat::model(entry).unwrap();
    let overlays = match haste_entity::boat::water_patch_overlay(
        entry,
        &haste_entity::boat::BoatState::default(),
    ) {
        Some(patch) => [Some(patch), None],
        None => [None, None],
    };
    EntityInstance {
        id: EntityId(id),
        model,
        pos,
        yaw,
        pitch: 0.0,
        head_yaw: yaw,
        scale: 1.0,
        pose: PoseOverrides::default(),
        body: BodyTransform {
            pre_translate: glam::Vec3::new(0.0, 0.375, 0.0),
            ..Default::default()
        },
        body_scale: glam::Vec3::ONE,
        skin: None,
        nametag: None,
        below_name: None,
        shadow_radius: 0.8,
        shadow_strength: 1.0,
        invisible: false,
        display_fire: false,
        width: 1.375,
        height: 0.5625,
        is_discrete: false,
        name_tag_y: 0.5625,
        hurt: 0.0,
        light: (15, 0),
        overlays,
        item: None,
        decoration: None,
        hands: [None, None],
        equipment: Vec::new(),
        head_item: None,
    }
}

#[test]
#[ignore = "needs the client jar, datagen reports, the model dump and a Vulkan driver (source scripts/env.sh)"]
fn boats_offscreen() {
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

    // An oak boat ahead of the camera (turned to show bow and side), a cherry chest boat
    // beside it, a bamboo raft behind. All three draw from their dumped models.
    let camera = Camera {
        pos: glam::DVec3::new(0.0, 2.2, -4.0),
        yaw: 0.0,
        pitch: 12.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    };
    let mut entities = vec![
        boat_instance(
            1,
            "minecraft:oak_boat",
            glam::DVec3::new(0.0, 1.0, 0.0),
            30.0,
        ),
        boat_instance(
            2,
            "minecraft:cherry_chest_boat",
            glam::DVec3::new(-1.6, 1.0, 0.8),
            -15.0,
        ),
        boat_instance(
            3,
            "minecraft:bamboo_raft",
            glam::DVec3::new(1.6, 1.0, 1.2),
            60.0,
        ),
    ];
    for e in entities.iter_mut() {
        e.sample_light(&world);
    }
    let frame = FrameInput {
        camera,
        entities: &entities,
        block_entities: &[],
        gui: &haste_core::gui::GuiFrame::default(),
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
    let img = renderer.read_back().unwrap();
    // The same frame without the boats: whatever changed between the two is the boats (and
    // their shadows), not leftover state.
    renderer
        .render(&FrameInput {
            camera,
            entities: &[],
            block_entities: &[],
            gui: &haste_core::gui::GuiFrame::default(),
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
        })
        .unwrap();
    let empty = renderer.read_back().unwrap();
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    std::fs::create_dir_all(&out).unwrap();
    img.save(out.join("boats.png")).unwrap();

    // The three boats and their shadows cover a solid patch of the middle of the frame (a
    // boat is 1.75 blocks wide at 4 blocks' distance at 70 degrees: hundreds of pixels each;
    // the shadows add more). Looking into a hull also shows its floor and the stone through
    // it, so the diff counts only what the boats actually covered.
    let mut changed = 0;
    for y in 100..460 {
        for x in 60..580 {
            if img.get_pixel(x, y).0 != empty.get_pixel(x, y).0 {
                changed += 1;
            }
        }
    }
    assert!(
        changed > 2_000,
        "the boats covered only {changed} pixels of the 640x480 frame"
    );
}
