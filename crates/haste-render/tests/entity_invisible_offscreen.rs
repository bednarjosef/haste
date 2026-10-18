//! Entities checked offscreen: an invisible entity draws no body, overlays or name tag
//! (vanilla `LivingEntityRenderer`), an invisible item frame keeps its item. Saves
//! `target/render-checks/invisible*.png`. Needs the client jar, the datagen reports and a
//! Vulkan driver (source scripts/env.sh).

use haste_core::model::{BodyTransform, EntityInstance, PoseOverrides};
use haste_core::{BlockPos, BlockStateId, ChunkPos, EntityId, SectionPos};
use haste_render::{Camera, Renderer, SkyState};

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
    entities: &[EntityInstance],
    camera: Camera,
) -> image::RgbaImage {
    let gui = haste_core::gui::GuiFrame {
        scale: 2,
        ..Default::default()
    };
    renderer.sample_entity_lights(world, entities);
    let frame = haste_render::FrameInput {
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

/// Invisible entities draw no body and no overlays (`LivingEntityRenderer.isBodyVisible` /
/// `getRenderType` -> null, `RenderLayer.coloredCutoutModelCopyLayerRender`), but keep the
/// equipment layers (`HumanoidArmorLayer`/`ItemInHandLayer` do not check invisibility): a
/// bare invisible entity changes no pixels at all, one with armour and a sword still draws
/// them. Saves `target/render-checks/invisible.png`.
#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn invisible_entities_offscreen() {
    use haste_core::model::{EquipmentPiece, EquipmentSlot};

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
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    };
    // The same humanoid as `entities_offscreen`, once visible and once invisible (as the
    // entity store hands it over: no name tag while invisible).
    let visible = EntityInstance {
        id: EntityId(1),
        model: haste_entity::models::humanoid(false),
        pos: glam::DVec3::new(0.0, 1.0, 0.0),
        yaw: 180.0,
        pitch: 0.0,
        head_yaw: 180.0,
        scale: 1.0,
        pose: PoseOverrides::default(),
        body: BodyTransform::default(),
        body_scale: glam::Vec3::ONE,
        skin: None,
        nametag: Some("Steve".into()),
        below_name: None,
        shadow_radius: 0.5,
        shadow_strength: 1.0,
        invisible: false,
        is_discrete: false,
        name_tag_y: 1.8,
        hurt: 0.0,
        overlays: Default::default(),
        item: None,
        decoration: None,
        hands: [None, None],
        equipment: Vec::new(),
        head_item: None,
    };
    let mut invisible = visible.clone();
    invisible.id = EntityId(2);
    invisible.invisible = true;
    invisible.nametag = None;
    // An invisible one that wears an iron chestplate and holds a sword: vanilla still draws
    // the equipment layers on invisible entities.
    let sword = data
        .registries
        .index("minecraft:item", &haste_core::Identifier::minecraft("iron_sword"))
        .expect("iron_sword in the item registry");
    let chest = data
        .registries
        .index("minecraft:item", &haste_core::Identifier::minecraft("iron_chestplate"))
        .expect("iron_chestplate in the item registry");
    let chest_eq = data.item_components.equippable(chest).expect("equippable");
    let mut armed = invisible.clone();
    armed.hands = [Some(sword), None];
    armed.equipment = vec![EquipmentPiece {
        slot: EquipmentSlot::Chest,
        item: Some(chest),
        asset: chest_eq.asset.clone(),
        dye: None,
    }];

    // Warm up the lazy atlas, then the empty scene to compare against.
    for _ in 0..2 {
        let _ = render_scene(&mut renderer, &world, &[], camera);
    }
    let empty = render_scene(&mut renderer, &world, &[], camera);
    let diff = |a: &image::RgbaImage, b: &image::RgbaImage| {
        a.pixels()
            .zip(b.pixels())
            .filter(|(p, q)| p.0 != q.0)
            .count()
    };
    // Visible: the body is drawn.
    let img = render_scene(&mut renderer, &world, std::slice::from_ref(&visible), camera);
    assert!(diff(&empty, &img) > 2000, "the visible body draws");
    assert_eq!(renderer.entities_drawn(), 1);
    // Invisible with nothing on: exactly the empty frame (no body, no name tag, no shadow).
    let img = render_scene(
        &mut renderer,
        &world,
        std::slice::from_ref(&invisible),
        camera,
    );
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    std::fs::create_dir_all(&out).unwrap();
    img.save(out.join("invisible.png")).unwrap();
    assert_eq!(diff(&empty, &img), 0, "the bare invisible body draws nothing");
    assert_eq!(renderer.entities_drawn(), 0, "no geometry was posed");
    // Invisible but equipped: the armour and sword still draw (vanilla keeps the
    // equipment layers), and nothing else does.
    let img = render_scene(&mut renderer, &world, std::slice::from_ref(&armed), camera);
    img.save(out.join("invisible_equipped.png")).unwrap();
    assert!(
        diff(&empty, &img) > 100,
        "the equipment draws on an invisible entity"
    );
    assert_eq!(renderer.entities_drawn(), 1);
    // The same equipped entity visible draws strictly more (its body under the equipment).
    let invisible_changed = diff(&empty, &img);
    let mut visible_armed = armed.clone();
    visible_armed.invisible = false;
    let img = render_scene(
        &mut renderer,
        &world,
        std::slice::from_ref(&visible_armed),
        camera,
    );
    assert!(
        diff(&empty, &img) > invisible_changed,
        "the visible body draws more than the invisible one's equipment"
    );
}
