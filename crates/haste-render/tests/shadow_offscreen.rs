//! Entity shadows over invisible-render-shape blocks, checked offscreen (vanilla
//! `EntityRenderer.extractShadowPiece`): a shadow over a barrier floor emits no quads, over
//! stone it does. Needs the client jar, the datagen reports and a Vulkan driver (source
//! scripts/env.sh).

use haste_core::model::{BodyTransform, EntityInstance, PoseOverrides};
use haste_core::{BlockPos, BlockStateId, ChunkPos, EntityId, Identifier};
use haste_render::{Camera, FrameInput, Renderer, SkyState};

/// A one-block floor at y=0 (the state under test), air above, full sky light.
struct Flat {
    floor: BlockStateId,
}

impl haste_core::view::BlockView for Flat {
    fn block_state(&self, p: BlockPos) -> BlockStateId {
        if p.y == 0 {
            self.floor
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

/// The shadow quads a pig standing one block above `floor` batches (vanilla
/// `EntityRenderer.extractShadow`/`extractShadowPiece`). The world is only read, so no
/// terrain needs meshing.
fn shadow_quads_over_floor(
    floor: BlockStateId,
    assets: &mut haste_assets::Assets,
    data: &haste_data::GameData,
) -> usize {
    let mut renderer = Renderer::new_offscreen(640, 480, assets, &data.blocks).unwrap();
    let world = Flat { floor };
    let entities = vec![EntityInstance {
        id: EntityId(1),
        model: haste_entity::models::box_for(0.9, 0.9),
        pos: glam::DVec3::new(1.5, 1.0, 0.5),
        yaw: 180.0,
        pitch: 0.0,
        head_yaw: 180.0,
        scale: 1.0,
        pose: PoseOverrides::default(),
        body: BodyTransform::default(),
        body_scale: glam::Vec3::ONE,
        skin: None,
        nametag: None,
        below_name: None,
        shadow_radius: 0.7,
        shadow_strength: 1.0,
        invisible: false,
        is_discrete: false,
        name_tag_y: 0.9,
        hurt: 0.0,
        light: (15, 0),
        overlays: Default::default(),
        item: None,
        decoration: None,
        hands: [None, None],
        equipment: Vec::new(),
        head_item: None,
    }];
    let camera = Camera {
        pos: glam::DVec3::new(0.0, 3.2, -4.0),
        yaw: 0.0,
        pitch: 32.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    };
    let gui = haste_core::gui::GuiFrame {
        scale: 2,
        ..Default::default()
    };
    let frame = FrameInput {
        camera,
        entities: &entities,
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
    renderer.sample_world(&world, &data.shapes, &frame);
    renderer.shadow_quad_count()
}

/// `EntityRenderer.extractShadowPiece` skips the block below when its render shape is
/// invisible (barrier) before the collision-shape and brightness tests, so a shadow over a
/// barrier floor emits no quads while over stone it does.
#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn shadow_skips_invisible_blocks() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let barrier = data
        .blocks
        .default_state(&Identifier::minecraft("barrier"))
        .unwrap();
    let stone = data
        .blocks
        .default_state(&Identifier::minecraft("stone"))
        .unwrap();
    assert_eq!(
        shadow_quads_over_floor(barrier, &mut assets, &data),
        0,
        "a barrier floor casts no shadow"
    );
    assert!(
        shadow_quads_over_floor(stone, &mut assets, &data) > 0,
        "a stone floor casts a shadow"
    );
}
