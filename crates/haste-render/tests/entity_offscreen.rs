//! Entities checked offscreen: a stone floor, a player model with the default skin, a walking
//! pose and a name tag, and a generic box entity. Saves `target/render-checks/entities.png`.
//! Needs the client jar, the datagen reports and a Vulkan driver (source scripts/env.sh).

use haste_core::model::{
    Decoration, EntityInstance, ItemFrameDecoration, PaintingDecoration, PoseOverrides,
};
use haste_core::{BlockPos, BlockStateId, ChunkPos, Direction, EntityId, Identifier, SectionPos};
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
    world: &Flat,
    entities: &[EntityInstance],
    camera: Camera,
) -> image::RgbaImage {
    render_scene_at(renderer, world, entities, camera, 0.0)
}

fn render_scene_at(
    renderer: &mut Renderer,
    world: &Flat,
    entities: &[EntityInstance],
    camera: Camera,
    partial_tick: f32,
) -> image::RgbaImage {
    let gui = haste_core::gui::GuiFrame {
        scale: 2,
        ..Default::default()
    };
    render_with_gui(renderer, world, entities, camera, &gui, partial_tick)
}

fn render_with_gui(
    renderer: &mut Renderer,
    world: &Flat,
    entities: &[EntityInstance],
    camera: Camera,
    gui: &haste_core::gui::GuiFrame,
    partial_tick: f32,
) -> image::RgbaImage {
    render_with_gui_skip(renderer, world, entities, camera, gui, partial_tick, None)
}

/// Like [`render_with_gui`], with `FrameInput::camera_entity` (the world pass skips it).
fn render_with_gui_skip(
    renderer: &mut Renderer,
    world: &Flat,
    entities: &[EntityInstance],
    camera: Camera,
    gui: &haste_core::gui::GuiFrame,
    partial_tick: f32,
    camera_entity: Option<haste_core::EntityId>,
) -> image::RgbaImage {
    renderer.sample_entity_lights(world, entities);
    let frame = FrameInput {
        camera,
        entities,
        gui,
        sky: SkyState {
            time_of_day: 6000,
            rain: 0.0,
            dimension_has_sky: true,
            fog_start: 100.0,
            fog_end: 200.0,
        },
        partial_tick,
        block_target: None,
        breaking: &[],
        camera_entity,
    };
    renderer.render(&frame).unwrap();
    renderer.read_back().unwrap()
}

#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn entities_offscreen() {
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

    let mut walk = PoseOverrides::default();
    walk.rotations
        .push(("right_arm".into(), glam::Vec3::new(-0.6, 0.0, 0.0)));
    walk.rotations
        .push(("left_arm".into(), glam::Vec3::new(0.6, 0.0, 0.0)));
    walk.rotations
        .push(("right_leg".into(), glam::Vec3::new(0.5, 0.0, 0.0)));
    walk.rotations
        .push(("left_leg".into(), glam::Vec3::new(-0.5, 0.0, 0.0)));
    walk.rotations
        .push(("head".into(), glam::Vec3::new(0.0, 0.5, 0.0)));
    let (skin, slim) = haste_render::default_skin(0x1234_5678_9abc_def0_1234_5678_9abc_def0);
    let player = EntityInstance {
        id: EntityId(1),
        model: haste_entity::models::humanoid(slim),
        pos: glam::DVec3::new(0.0, 1.0, 0.0),
        // Facing the camera (camera looks toward +z from z = -4, so the player faces -z).
        yaw: 180.0,
        pitch: 0.0,
        head_yaw: 180.0,
        scale: 1.0,
        pose: walk,
        skin: Some(skin.clone()),
        nametag: Some("Steve".into()),
        hurt: 0.0,
        decoration: None,
    };
    let boxed = EntityInstance {
        id: EntityId(2),
        model: haste_entity::models::box_for(0.9, 1.4),
        pos: glam::DVec3::new(-2.0, 1.0, 1.0),
        yaw: 30.0,
        pitch: 0.0,
        head_yaw: 30.0,
        scale: 1.0,
        pose: PoseOverrides::default(),
        skin: None,
        nametag: None,
        hurt: 0.0,
        decoration: None,
    };
    let entities = vec![player, boxed];
    let camera = Camera {
        pos: glam::DVec3::new(0.0, 2.2, -4.0),
        yaw: 0.0,
        pitch: 5.0,
        fov_deg: 70.0,
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    };
    let img = render_scene(&mut renderer, &world, &entities, camera);
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    std::fs::create_dir_all(&out).unwrap();
    img.save(out.join("entities.png")).unwrap();
    eprintln!("player skin {skin} (slim {slim})");

    // The player stands in the middle of the frame: its pixels differ from sky and floor
    // at the screen centre column around the chest.
    let sky = img.get_pixel(320, 20).0;
    let centre = img.get_pixel(320, 230).0;
    assert_ne!(centre, sky, "player body at the centre");
    // Seen from behind, the player is drawn too (back faces of the skin), and moving it away
    // leaves the floor/sky there.
    let empty = render_scene(&mut renderer, &world, &[], camera);
    assert_ne!(
        empty.get_pixel(320, 230).0,
        centre,
        "the player covered the centre"
    );

    // The inventory player: GuiCommand::Entity in a dark box, looking at a mouse up-right.
    use haste_core::gui::{GuiCommand, GuiFrame};
    let gui = GuiFrame {
        commands: vec![
            GuiCommand::Fill {
                x1: 10,
                y1: 10,
                x2: 70,
                y2: 90,
                color: 0xFF00_0000,
            },
            GuiCommand::Entity {
                entity: EntityId(1),
                x1: 10,
                y1: 10,
                x2: 70,
                y2: 90,
                scale: 30.0,
                mouse_x: 120.0,
                mouse_y: 15.0,
            },
        ],
        scale: 2,
        width: 320,
        height: 240,
        cursor: None,
    };
    let img = render_with_gui(&mut renderer, &world, &entities, camera, &gui, 0.0);
    img.save(out.join("entities_gui.png")).unwrap();
    // Something other than the black box inside it (the model), nothing outside the box edge
    // rows beyond the clip.
    let inside = (20..140).flat_map(|y| (20..140).map(move |x| (x, y)));
    assert!(inside
        .into_iter()
        .any(|(x, y)| img.get_pixel(x, y).0 != [0, 0, 0, 255]));
    assert_eq!(renderer.gui_stats().unsupported, 0);

    // First person with the same entities: `camera_entity` skips the player in the world pass
    // (the centre shows floor, not the body), and the GUI entity still draws it in its box.
    let first = render_with_gui_skip(
        &mut renderer,
        &world,
        &entities,
        camera,
        &gui,
        1.0,
        Some(EntityId(1)),
    );
    assert_eq!(first.get_pixel(320, 230).0, empty.get_pixel(320, 230).0);
    let gui_inside = (20..140).flat_map(|y| (20..140).map(move |x| (x, y)));
    assert!(gui_inside
        .into_iter()
        .any(|(x, y)| first.get_pixel(x, y).0 != [0, 0, 0, 255]));
    assert_eq!(renderer.gui_stats().unsupported, 0);

    // First person: the empty hand, then a held sword and a held stone block.
    renderer.set_item_registry(&data.registries);
    let item = |n: &str| {
        data.registries
            .index("minecraft:item", &haste_core::Identifier::minecraft(n))
    };
    let base = render_scene(&mut renderer, &world, &[], camera);
    let hand = |held: Option<haste_core::RegistryIndex>| haste_render::FirstPerson {
        skin: None,
        slim: false,
        item: held,
        light: (15, 0),
        attack: haste_render::TickPair::fixed(0.0),
        equip: haste_render::TickPair::fixed(1.0),
        x_bob: haste_render::TickPair::fixed(0.0),
        y_bob: haste_render::TickPair::fixed(0.0),
        bob: haste_render::TickPair::fixed(0.0),
        walk_dist: haste_render::TickPair::fixed(0.0),
    };
    for (name, held) in [
        ("hand", None),
        ("sword", item("diamond_sword")),
        ("stone", item("stone")),
    ] {
        renderer.set_first_person(Some(hand(held)));
        let img = render_scene(&mut renderer, &world, &[], camera);
        img.save(out.join(format!("first_person_{name}.png")))
            .unwrap();
        let changed = base
            .pixels()
            .zip(img.pixels())
            .filter(|(a, b)| a.0 != b.0)
            .count();
        assert!(
            changed > 2000,
            "{name}: first-person view drew {changed} pixels"
        );
    }
    // A swing moves the hand: the same hand at the swing's start, middle and end (the swing runs
    // over 6 ticks, so partial 0.33 and 0.66 are its second and fourth ticks).
    renderer.set_first_person(Some(hand(item("diamond_sword"))));
    let still = render_scene(&mut renderer, &world, &[], camera);
    renderer.set_first_person(Some(haste_render::FirstPerson {
        attack: haste_render::TickPair::fixed(0.33),
        ..hand(item("diamond_sword"))
    }));
    let swung = render_scene_at(&mut renderer, &world, &[], camera, 0.33);
    let moved = still
        .pixels()
        .zip(swung.pixels())
        .filter(|(a, b)| a.0 != b.0)
        .count();
    assert!(moved > 500, "the swing moved {moved} pixels");
    renderer.set_first_person(None);
}

/// A hanging decoration instance (the model is unused; the renderer draws the decoration).
fn decoration(id: i32, pos: glam::DVec3, decoration: Decoration) -> EntityInstance {
    EntityInstance {
        id: EntityId(id),
        model: haste_entity::models::box_for(1.0, 1.0),
        pos,
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 0.0,
        scale: 1.0,
        pose: PoseOverrides::default(),
        skin: None,
        nametag: None,
        hurt: 0.0,
        decoration: Some(decoration),
    }
}

/// Item frames (one with a diamond, one glow) and a 2x2 painting drawn offscreen: the same
/// scene as the RCON screenshot but deterministic (no server). Saves
/// `target/render-checks/frames.png`.
#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn frames_and_paintings_offscreen() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let mut renderer = Renderer::new_offscreen(640, 480, &mut assets, &data.blocks).unwrap();
    renderer.set_item_registry(&data.registries);
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
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    };
    let diamond = data
        .registries
        .index("minecraft:item", &Identifier::minecraft("diamond"))
        .expect("diamond item");
    let entities = vec![
        decoration(
            1,
            glam::DVec3::new(0.0, 1.5, 0.5),
            Decoration::ItemFrame(ItemFrameDecoration {
                facing: Direction::North,
                rotation: 1,
                glow: false,
                item: Some(diamond),
            }),
        ),
        decoration(
            2,
            glam::DVec3::new(1.6, 1.5, 0.5),
            Decoration::ItemFrame(ItemFrameDecoration {
                facing: Direction::North,
                rotation: 0,
                glow: true,
                item: None,
            }),
        ),
        decoration(
            3,
            glam::DVec3::new(-2.0, 1.5, 0.5),
            Decoration::Painting(PaintingDecoration {
                facing: Direction::North,
                width: 2,
                height: 2,
                front: Some(Identifier::minecraft("painting/match")),
            }),
        ),
    ];
    let base = render_scene(&mut renderer, &world, &[], camera);
    let img = render_scene(&mut renderer, &world, &entities, camera);
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    std::fs::create_dir_all(&out).unwrap();
    img.save(out.join("frames.png")).unwrap();
    let changed = base
        .pixels()
        .zip(img.pixels())
        .filter(|(a, b)| a.0 != b.0)
        .count();
    assert!(
        changed > 2000,
        "the decorations changed {changed} pixels (item frame, glow frame, painting)"
    );
}
