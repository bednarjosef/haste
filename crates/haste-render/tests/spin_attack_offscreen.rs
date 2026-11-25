//! The riptide spin offscreen (`SpinAttackEffectLayer` over a player in
//! `LivingEntityRenderer.setupRotations`' auto-spin pose) and a thrown trident beside it
//! (`ThrownTridentRenderer`, model from the recorder's dump), drawn through the entity pass.
//! Saves `target/render-checks/spin_attack.png` and `spin_attack_off.png`. Needs the client jar,
//! the datagen reports, the model dump and a Vulkan driver (source scripts/env.sh).
use haste_core::model::{BodyOp, BodyTransform, EntityInstance, EntityOverlay, PoseOverrides};
use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId, ChunkPos, DVec3, EntityId, Identifier};
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

/// An entity instance with `model` at `pos` and no layers.
fn instance(
    id: i32,
    model: std::sync::Arc<haste_core::model::CubeModel>,
    pos: DVec3,
) -> EntityInstance {
    EntityInstance {
        id: EntityId(id),
        model,
        pos,
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 0.0,
        scale: 1.0,
        pose: PoseOverrides::default(),
        body: BodyTransform::default(),
        body_scale: glam::Vec3::ONE,
        skin: None,
        nametag: None,
        below_name: None,
        shadow_radius: 0.0,
        shadow_strength: 1.0,
        invisible: false,
        display_fire: false,
        width: 0.6,
        height: 1.8,
        is_discrete: false,
        name_tag_y: 1.8,
        hurt: 0.0,
        light: (15, 0),
        overlays: Default::default(),
        item: None,
        decoration: None,
        hands: [None, None],
        equipment: Vec::new(),
        head_item: None,
        hand_foils: Default::default(),
        head_item_foil: Default::default(),
    }
}

/// A player in the auto-spin pose at `age` ticks (`setupRotations`: X by -90 - xRot, then Y by
/// age * -75, xRot 0 here), with the spin layer posed for the same age when `spin`.
fn spinning_player(age: f32, spin: bool) -> EntityInstance {
    let skin = haste_entity::skin::default_skin(uuid::Uuid::from_u128(7));
    let mut p = instance(
        1,
        haste_entity::models::humanoid(skin.model.is_slim()),
        DVec3::new(0.0, 0.6, 0.0),
    );
    p.yaw = 90.0;
    p.skin = Some(skin.body.clone());
    p.body.push(BodyOp::RotateX((-90.0f32).to_radians()));
    p.body.push(BodyOp::RotateY((age * -75.0).to_radians()));
    if spin {
        haste_entity::spin_attack::pose(&mut p.pose, age);
        p.overlays[1] = Some(EntityOverlay {
            model: haste_entity::spin_attack::layer(),
            tint: [1.0; 3],
        });
    }
    p
}

/// A trident lying level towards +X (yRot -90), as `ThrownTridentRenderer` poses it, with the
/// trident layer from the dump and its texture.
fn trident() -> Option<EntityInstance> {
    let lib = haste_entity::model_dump::library()?;
    let mut model = (*lib.get("minecraft:trident#main", 1.0)?).clone();
    model.texture = Identifier::minecraft("entity/trident/trident");
    let mut t = instance(2, std::sync::Arc::new(model), DVec3::new(-1.0, 0.3, 2.0));
    t.body = haste_entity::trident::body_transform(-90.0, 0.0);
    Some(t)
}

fn render(
    renderer: &mut Renderer,
    world: &Floor,
    entities: &mut [EntityInstance],
) -> image::RgbaImage {
    for e in entities.iter_mut() {
        e.sample_light(world);
    }
    let gui = haste_core::gui::GuiFrame {
        scale: 2,
        ..Default::default()
    };
    let frame = FrameInput {
        camera: Camera {
            pos: DVec3::new(0.0, 2.2, -3.5),
            yaw: 0.0,
            pitch: 18.0,
            fov_deg: 70.0,
            hand_fov_deg: 70.0,
            roll_deg: 0.0,
            bob: ViewBob::default(),
        },
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

#[test]
#[ignore = "needs the client jar, datagen reports, the model dump and a Vulkan driver (source scripts/env.sh)"]
fn spin_attack_offscreen() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let mut renderer = Renderer::new_offscreen(640, 480, &mut assets, &data.blocks).unwrap();
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
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    std::fs::create_dir_all(&out).unwrap();
    let age = 3.0;
    let mut on: Vec<EntityInstance> = std::iter::once(spinning_player(age, true))
        .chain(trident())
        .collect();
    let with = render(&mut renderer, &world, &mut on);
    with.save(out.join("spin_attack.png")).unwrap();
    let mut off: Vec<EntityInstance> = std::iter::once(spinning_player(age, false))
        .chain(trident())
        .collect();
    let without = render(&mut renderer, &world, &mut off);
    without.save(out.join("spin_attack_off.png")).unwrap();
    // The layer adds its pale streaks around the player: a good number of pixels change.
    let changed = with
        .pixels()
        .zip(without.pixels())
        .filter(|(a, b)| {
            a.0[..3]
                .iter()
                .zip(&b.0[..3])
                .any(|(x, y)| x.abs_diff(*y) > 24)
        })
        .count();
    eprintln!("spin layer changed {changed} pixels");
    assert!(changed > 2000, "the spin layer drew only {changed} pixels");
}
