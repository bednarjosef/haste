//! The thrown-item renderer offscreen (`ThrownItemRenderer`, `DragonFireballRenderer`): a
//! snowball, an egg, an ender pearl and a splash potion hovering like NoGravity projectiles
//! plus a dragon fireball, all drawn through the entity pass, and a release-timed
//! frame-cost check of a 50-projectile scene against the generic box the entity path drew
//! before.
use haste_core::model::{
    BodyTransform, Decoration, EntityInstance, PoseOverrides, ThrownItemDecoration,
};
use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId, ChunkPos, DVec3, EntityId, Identifier, RegistryIndex};
use haste_render::{Camera, FrameInput, Renderer, SkyState, ViewBob};

/// A stone floor at y = 0, air above.
struct Floor {
    stone: BlockStateId,
}

impl BlockView for Floor {
    fn block_state(&self, p: BlockPos) -> BlockStateId {
        if p.y < 0 {
            self.stone
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

/// One hovering projectile at (x, 1.3, 0.5): the entity the store would build for it (the
/// generic box stands in for the model, which the decoration path does not draw).
fn projectile(
    id: i32,
    x: f64,
    item: RegistryIndex,
    scale: f32,
    full_bright: bool,
) -> EntityInstance {
    EntityInstance {
        id: EntityId(id),
        model: haste_entity::models::box_for(0.25, 0.25),
        pos: DVec3::new(x, 1.3, 0.5),
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 0.0,
        scale: 1.0,
        pose: PoseOverrides::default(),
        body: BodyTransform::default(),
        body_scale: DVec3::ONE.as_vec3(),
        skin: None,
        nametag: None,
        below_name: None,
        shadow_radius: 0.0,
        shadow_strength: 1.0,
        invisible: false,
        display_fire: false,
        width: 0.25,
        height: 0.25,
        is_discrete: false,
        name_tag_y: 0.25,
        hurt: 0.0,
        light: (15, 0),
        overlays: Default::default(),
        item: None,
        decoration: Some(Decoration::ThrownItem(ThrownItemDecoration {
            item,
            scale,
            full_bright,
        })),
        hands: [None, None],
        equipment: Vec::new(),
        head_item: None,
    }
}

/// The renderer and the stone floor, meshed and settled (the jar, the reports and a Vulkan
/// driver must be present). The item registry is set like the client does at startup, so the
/// projectiles' item models resolve.
fn scene() -> (Renderer, Floor, haste_data::GameData) {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let mut renderer = Renderer::new_offscreen(640, 480, &mut assets, &data.blocks).unwrap();
    renderer.set_item_registry(&data.registries);
    let world = Floor {
        stone: data
            .blocks
            .default_state(&Identifier::minecraft("stone"))
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
    (renderer, world, data)
}

/// The frame the scene renders with, from a camera 4 blocks south of the projectiles.
/// `gui` outlives the returned frame (the caller owns it).
fn frame<'a>(
    entities: &'a mut [EntityInstance],
    world: &Floor,
    gui: &'a haste_core::gui::GuiFrame,
) -> FrameInput<'a> {
    for e in entities.iter_mut() {
        e.sample_light(world);
    }
    FrameInput {
        camera: Camera {
            pos: DVec3::new(0.0, 2.6, -4.0),
            yaw: 0.0,
            pitch: 12.0,
            fov_deg: 70.0,
            hand_fov_deg: 70.0,
            roll_deg: 0.0,
            bob: ViewBob::default(),
        },
        entities,
        block_entities: &[],
        gui,
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
    }
}

/// The empty HUD frame the scenes render with.
fn gui() -> haste_core::gui::GuiFrame {
    haste_core::gui::GuiFrame {
        scale: 2,
        ..Default::default()
    }
}

/// The registry index of `minecraft:<name>`.
fn item_of(data: &haste_data::GameData, name: &str) -> RegistryIndex {
    data.registries
        .index("minecraft:item", &Identifier::minecraft(name))
        .unwrap_or_else(|| panic!("no item {name}"))
}

#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn thrown_items_offscreen() {
    let (mut renderer, world, data) = scene();
    // The four projectiles of the brief (NoGravity, hovering): a snowball, an egg, an ender
    // pearl and a splash potion — scale 1, not full-bright — plus a burning small fireball
    // at its 0.75 scale (`AbstractHurtingProjectile` burns and smokes) and a dragon fireball
    // behind them.
    let dragon = projectile(6, 0.0, RegistryIndex(0), 1.0, false);
    let mut small_fireball = projectile(5, -3.75, item_of(&data, "fire_charge"), 0.75, true);
    small_fireball.display_fire = true;
    small_fireball.width = 0.3125;
    small_fireball.height = 0.3125;
    let mut entities = vec![
        projectile(1, -2.25, item_of(&data, "snowball"), 1.0, false),
        projectile(2, -0.75, item_of(&data, "egg"), 1.0, false),
        projectile(3, 0.75, item_of(&data, "ender_pearl"), 1.0, false),
        projectile(4, 2.25, item_of(&data, "splash_potion"), 1.0, false),
        small_fireball,
        EntityInstance {
            decoration: Some(Decoration::DragonFireball),
            pos: DVec3::new(0.75, 1.3, 3.0),
            ..dragon
        },
    ];
    let gui = gui();
    let item_frame = frame(&mut entities, &world, &gui);
    renderer.render(&item_frame).unwrap();
    let img = renderer.read_back().unwrap();
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    std::fs::create_dir_all(&out).unwrap();
    img.save(out.join("thrown_items.png")).unwrap();

    // Each sprite covers part of the floor behind it; an empty scene leaves the floor.
    let empty_frame = FrameInput {
        entities: &[],
        ..item_frame
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
    // The five small sprites and the two-scaled dragon fireball quad.
    assert!(
        changed > 400,
        "the sprites cover the floor ({changed} pixels)"
    );
    // The camera-facing sprites sit in one row: the changed span is wide, not tall.
    let (w, h) = (bounds.1 .0 - bounds.0 .0, bounds.1 .1 - bounds.0 .1);
    assert!(w > 3 * h, "a row of camera-facing sprites, {w}x{h}");

    // The burning small fireball (`EntityRenderDispatcher`'s `FlameFeatureRenderer` after
    // `ThrownItemRenderer.submit`): with `display_fire` the flames add pixels around the
    // sprite that the same scene without them does not have.
    let mut calm = entities.clone();
    calm[4].display_fire = false; // the small fireball, entity id 5
    let calm_frame = frame(&mut calm, &world, &gui);
    renderer.render(&calm_frame).unwrap();
    let calm = renderer.read_back().unwrap();
    let fire = img
        .enumerate_pixels()
        .filter(|(x, y, p)| p.0 != calm.get_pixel(*x, *y).0)
        .count();
    eprintln!("fire overlay pixels: {fire}");
    assert!(fire > 100, "the small fireball burns ({fire} pixels)");
}

/// The per-frame entity cost of ~50 thrown items (the brief's measurement): this branch's
/// sprite path against the generic box the entity path drew before. Release only; prints
/// average and worst frame.
#[test]
#[ignore = "timed test, run in release: cargo test --release -p haste-render --test thrown_item_offscreen -- --ignored --nocapture"]
fn fifty_thrown_items_frame_time() {
    let (mut renderer, world, data) = scene();
    let items = [
        "snowball",
        "egg",
        "ender_pearl",
        "splash_potion",
        "experience_bottle",
    ];
    let mut projectiles: Vec<EntityInstance> = (0..50)
        .map(|i| {
            projectile(
                i,
                -7.0 + f64::from(i % 25) * 0.6,
                item_of(&data, items[(i % 5) as usize]),
                1.0,
                false,
            )
        })
        .collect();
    // The "before" look: the same entities as the generic boxes the entity store fell back
    // to before the thrown-item renderer existed.
    let mut boxes: Vec<EntityInstance> = projectiles
        .iter()
        .map(|e| EntityInstance {
            decoration: None,
            ..e.clone()
        })
        .collect();
    let gui = gui();
    let item_frame = frame(&mut projectiles, &world, &gui);
    let box_frame = frame(&mut boxes, &world, &gui);
    let measure = |renderer: &mut Renderer, frame: &FrameInput| {
        let mut worst = std::time::Duration::ZERO;
        let mut total = std::time::Duration::ZERO;
        const FRAMES: u32 = 200;
        for _ in 0..FRAMES {
            let start = std::time::Instant::now();
            renderer.render(frame).unwrap();
            let d = start.elapsed();
            worst = worst.max(d);
            total += d;
        }
        (total / FRAMES, worst)
    };
    // Warm both paths (texture rects, the item models, the display transforms).
    for _ in 0..10 {
        renderer.render(&item_frame).unwrap();
        renderer.render(&box_frame).unwrap();
    }
    let (avg, worst) = measure(&mut renderer, &item_frame);
    let (box_avg, box_worst) = measure(&mut renderer, &box_frame);
    eprintln!(
        "50 thrown items: avg {avg:?} worst {worst:?} (boxes before: avg {box_avg:?} worst {box_worst:?})"
    );
}
