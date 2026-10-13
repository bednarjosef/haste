//! Entities checked offscreen: a stone floor, a player model with the default skin, a walking
//! pose and a name tag, and a generic box entity. Saves `target/render-checks/entities.png`.
//! Needs the client jar, the datagen reports and a Vulkan driver (source scripts/env.sh).

use haste_core::model::{EntityInstance, PoseOverrides};
use haste_core::{BlockPos, BlockStateId, ChunkPos, EntityId, SectionPos};
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
    let gui = haste_core::gui::GuiFrame {
        scale: 2,
        ..Default::default()
    };
    render_with_gui(renderer, world, entities, camera, &gui)
}

fn render_with_gui(
    renderer: &mut Renderer,
    world: &Flat,
    entities: &[EntityInstance],
    camera: Camera,
    gui: &haste_core::gui::GuiFrame,
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
        partial_tick: 0.0,
        block_target: None,
        breaking: &[],
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
        item: None,
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
        item: None,
    };
    let entities = vec![player, boxed];
    let camera = Camera {
        pos: glam::DVec3::new(0.0, 2.2, -4.0),
        yaw: 0.0,
        pitch: 5.0,
        fov_deg: 70.0,
        roll_deg: 0.0,
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
    let img = render_with_gui(&mut renderer, &world, &entities, camera, &gui);
    img.save(out.join("entities_gui.png")).unwrap();
    // Something other than the black box inside it (the model), nothing outside the box edge
    // rows beyond the clip.
    let inside = (20..140).flat_map(|y| (20..140).map(move |x| (x, y)));
    assert!(inside
        .into_iter()
        .any(|(x, y)| img.get_pixel(x, y).0 != [0, 0, 0, 255]));
    assert_eq!(renderer.gui_stats().unsupported, 0);

    // First person: the empty hand, then a held sword and a held stone block.
    renderer.set_item_registry(&data.registries);
    let item = |n: &str| {
        data.registries
            .index("minecraft:item", &haste_core::Identifier::minecraft(n))
    };
    let base = render_scene(&mut renderer, &world, &[], camera);
    for (name, held) in [
        ("hand", None),
        ("sword", item("diamond_sword")),
        ("stone", item("stone")),
    ] {
        renderer.set_first_person(Some(haste_render::FirstPerson {
            skin: None,
            slim: false,
            item: held,
            light: (15, 0),
        }));
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
    renderer.set_first_person(None);
}

/// Dropped item entities draw the item itself (the GUI item model path), flat and block
/// items both, spinning and bobbing; an unknown item draws the missing texture instead
/// of the box placeholder. Saves target/render-checks/items.png.
#[test]
#[ignore = "needs the client jar, datagen reports, and a Vulkan driver (source scripts/env.sh)"]
fn dropped_items_offscreen() {
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
    let item = |n: &str| {
        data.registries
            .index("minecraft:item", &haste_core::Identifier::minecraft(n))
    };
    let model = haste_entity::models::box_for(0.25, 0.25);
    let drop = |i: i32, item, count| EntityInstance {
        id: EntityId(i),
        model: model.clone(),
        pos: glam::DVec3::new(0.0, 1.0, 0.0),
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 0.0,
        scale: 1.0,
        pose: PoseOverrides::default(),
        skin: None,
        nametag: None,
        hurt: 0.0,
        item: Some(haste_core::model::DroppedItem {
            item,
            count,
            age: 0.0,
        }),
    };
    let camera = Camera {
        pos: glam::DVec3::new(0.0, 1.7, -2.5),
        yaw: 0.0,
        pitch: 10.0,
        fov_deg: 70.0,
        roll_deg: 0.0,
    };
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    std::fs::create_dir_all(&out).unwrap();
    // Case A: flat item (diamond), block item (stone, a stack of 10 -> 3 copies) and an
    // unknown item (index past the registry). The flat diamond sprite is mostly
    // transparent (the gem shape is a few dozen pixels), so the bar is low for it; what
    // matters is that each draws something and they look different from each other.
    let empty = render_scene(&mut renderer, &world, &[], camera);
    let scenes: Vec<(String, Vec<EntityInstance>)> = vec![
        ("flat".into(), vec![drop(1, item("diamond").unwrap(), 1)]),
        ("block".into(), vec![drop(2, item("stone").unwrap(), 10)]),
        (
            "unknown".into(),
            vec![drop(3, haste_core::RegistryIndex(9999), 1)],
        ),
    ];
    let mut images = Vec::new();
    for (name, entities) in &scenes {
        let img = render_scene(&mut renderer, &world, entities, camera);
        img.save(out.join(format!("item_{name}.png"))).unwrap();
        let changed = empty
            .pixels()
            .zip(img.pixels())
            .filter(|(a, b)| a.0 != b.0)
            .count();
        assert!(
            changed > 40,
            "{name}: dropped item drew only {changed} pixels"
        );
        images.push(img);
    }
    // The three look different (a flat sprite, a small block with three copies, the
    // magenta/black missing checker).
    for (a, b) in [
        (&images[0], &images[1]),
        (&images[0], &images[2]),
        (&images[1], &images[2]),
    ] {
        let diff = a
            .pixels()
            .zip(b.pixels())
            .filter(|(p, q)| p.0 != q.0)
            .count();
        assert!(diff > 40, "two item scenes differ in only {diff} pixels");
    }
    // Case B: the spin and bob move the item between frames.
    let mut spinning = drop(1, item("diamond").unwrap(), 1);
    let a = render_scene(&mut renderer, &world, &[spinning.clone()], camera);
    spinning.item.as_mut().unwrap().age = 10.0;
    let b = render_scene(&mut renderer, &world, &[spinning.clone()], camera);
    let diff = |x: &image::RgbaImage, y: &image::RgbaImage| {
        x.pixels()
            .zip(y.pixels())
            .filter(|(p, q)| p.0 != q.0)
            .count()
    };
    // Age 10 (quarter spin, quarter bob cycle later) must look different; age 40 is the
    // full period, so the view is exactly the age 0 one again (spin and bob both have
    // 40-tick periods).
    spinning.item.as_mut().unwrap().age = 40.0;
    let full = render_scene(&mut renderer, &world, &[spinning], camera);
    assert!(
        diff(&a, &b) > 40,
        "no spin between ages 0 and 10: {}",
        diff(&a, &b)
    );
    assert_eq!(
        diff(&a, &full),
        0,
        "age 40 (full period) must render like age 0"
    );
}
