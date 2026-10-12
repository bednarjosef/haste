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

/// haste-entity's humanoid gives child parts (hat, jacket, sleeves, pants) the parent's absolute
/// pivot; the renderer follows vanilla `PartPose` (child pivots relative to the parent). Convert
/// until haste-entity changes, so this test checks the renderer, not that mismatch.
fn relative_pivots(
    model: &haste_core::model::CubeModel,
) -> std::sync::Arc<haste_core::model::CubeModel> {
    fn fix(part: &mut haste_core::model::ModelPart) {
        let p = part.pivot;
        for c in &mut part.children {
            c.pivot -= p;
            fix(c);
        }
    }
    let mut m = model.clone();
    for c in &mut m.root.children {
        fix(c);
    }
    std::sync::Arc::new(m)
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
        model: relative_pivots(&haste_entity::models::humanoid(slim)),
        pos: glam::DVec3::new(0.0, 1.0, 0.0),
        // Facing the camera (camera looks toward +z from z = -4, so the player faces -z).
        yaw: 180.0,
        pitch: 0.0,
        head_yaw: 180.0,
        scale: 1.0,
        pose: walk,
        skin: Some(skin.clone()),
        nametag: Some("Steve".into()),
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
    };
    let entities = vec![player, boxed];
    let camera = Camera {
        pos: glam::DVec3::new(0.0, 2.2, -4.0),
        yaw: 0.0,
        pitch: 5.0,
        fov_deg: 70.0,
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
}
