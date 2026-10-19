//! The minecart renderer offscreen (`AbstractMinecartRenderer`): a plain, a chest, a furnace
//! and a TNT cart on an east-west rail line, the cart model, the cargo blocks and the
//! rail-snapped pose all drawn through the entity pass.
use haste_core::model::{BodyTransform, Decoration, EntityInstance, PoseOverrides};
use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId, ChunkPos, DVec3, EntityId};
use haste_render::{Camera, FrameInput, Renderer, SkyState, ViewBob};

/// A stone floor at y = 0 with an east-west rail line on it (rails at y = 0 for
/// -3 <= x <= 3, z = 0), air above.
struct RailFloor {
    stone: BlockStateId,
    rail: BlockStateId,
}

impl BlockView for RailFloor {
    fn block_state(&self, p: BlockPos) -> BlockStateId {
        if p.y < 0 {
            self.stone
        } else if p.y == 0 && (-3..=3).contains(&p.x) && p.z == 0 {
            self.rail
        } else {
            BlockStateId(0)
        }
    }
    fn is_chunk_loaded(&self, _: ChunkPos) -> bool {
        true
    }
    fn sky_light(&self, _: BlockPos) -> u8 {
        15
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

/// One minecart instance at (x, 1.06, 0.5): on the rail's line, like the client poses it.
fn cart(id: i32, x: f64, variant: haste_entity::minecart::Variant) -> EntityInstance {
    EntityInstance {
        id: EntityId(id),
        model: haste_entity::minecart::model(),
        pos: DVec3::new(x, 1.06, 0.5),
        yaw: 90.0,
        pitch: 0.0,
        head_yaw: 0.0,
        scale: 1.0,
        pose: PoseOverrides::default(),
        body: BodyTransform::default(),
        body_scale: DVec3::ONE.as_vec3(),
        skin: None,
        nametag: None,
        below_name: None,
        shadow_radius: 0.7,
        shadow_strength: 1.0,
        invisible: false,
        display_fire: false,
        width: 0.98,
        height: 0.7,
        is_discrete: false,
        name_tag_y: 0.7,
        hurt: 0.0,
        light: (15, 0),
        overlays: Default::default(),
        item: None,
        decoration: Some(Decoration::Minecart(haste_entity::minecart::decoration(
            &haste_entity::Metadata::default(),
            variant,
            0.0,
        ))),
        hands: [None, None],
        equipment: Vec::new(),
        head_item: None,
    }
}

#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn minecarts_offscreen() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let mut renderer = Renderer::new_offscreen(640, 480, &mut assets, &data.blocks).unwrap();
    let world = RailFloor {
        stone: data
            .blocks
            .default_state(&haste_core::Identifier::minecraft("stone"))
            .unwrap(),
        rail: data
            .blocks
            .state(
                &haste_core::Identifier::minecraft("rail"),
                &[("shape", "east_west"), ("waterlogged", "false")],
            )
            .unwrap(),
    };
    let dirty: Vec<haste_core::SectionPos> = (-1..=1)
        .flat_map(|x| (-1..=1).map(move |z| haste_core::SectionPos::new(x, 0, z)))
        .collect();
    renderer.update_world(&world, &dirty, ChunkPos::new(0, 0), 4);
    for _ in 0..200 {
        renderer.update_world(&world, &[], ChunkPos::new(0, 0), 4);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let mut entities = vec![
        cart(1, -2.5, haste_entity::minecart::Variant::Plain),
        cart(2, -1.0, haste_entity::minecart::Variant::Chest),
        cart(3, 0.5, haste_entity::minecart::Variant::Furnace),
        cart(4, 2.0, haste_entity::minecart::Variant::Tnt),
    ];
    // The rail snapping the client does per frame (`OldMinecartBehavior.getPos`).
    haste_entity::minecart::snap_rails(&mut entities, &world, &data.blocks);
    for e in &mut entities {
        e.sample_light(&world);
    }
    let camera = Camera {
        pos: DVec3::new(0.0, 2.6, -4.0),
        yaw: 0.0,
        pitch: 12.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: ViewBob::default(),
    };
    let frame = FrameInput {
        camera,
        entities: &entities,
        block_entities: &[],
        gui: &haste_core::gui::GuiFrame {
            scale: 2,
            ..Default::default()
        },
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
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    std::fs::create_dir_all(&out).unwrap();
    img.save(out.join("minecarts.png")).unwrap();

    // Each cart covers part of the floor its cargo sits over (the model and the cargo are
    // darker than the stone); an empty scene leaves the floor and rails.
    let empty_frame = FrameInput {
        entities: &[],
        ..frame
    };
    renderer.render(&empty_frame).unwrap();
    let empty = renderer.read_back().unwrap();
    let mut bounds = ((u32::MAX, u32::MAX), (0u32, 0u32));
    let mut changed = 0u32;
    for (x, y, p) in img.enumerate_pixels() {
        if p.0 != empty.get_pixel(x, y).0 {
            changed += 1;
            bounds.0 = (bounds.0 .0.min(x), bounds.0 .1.min(y));
            bounds.1 = (bounds.1 .0.max(x), bounds.1 .1.max(y));
        }
    }
    eprintln!(
        "changed pixels: {changed}, bounds {:?}..{:?}",
        bounds.0, bounds.1
    );
    assert!(changed > 1000, "the carts and their cargo cover the floor");
    // The carts face along the rail: the plain cart's side (not its narrow end) meets the
    // camera, so a wide span of the centre row differs.
    let (top, bottom) = (bounds.0 .1, bounds.1 .1);
    let centre_row = (bounds.0 .0..=bounds.1 .0)
        .filter(|x| {
            img.get_pixel(*x, (top + bottom) / 2).0 != empty.get_pixel(*x, (top + bottom) / 2).0
        })
        .count();
    assert!(
        centre_row > 120,
        "four carts side-on span more than a quarter of the row ({centre_row} pixels)"
    );
}
