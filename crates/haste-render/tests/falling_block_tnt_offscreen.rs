//! Falling blocks and primed TNT checked offscreen (vanilla `FallingBlockRenderer` /
//! `TntRenderer`): a falling sand block, a landed one (which must draw nothing), a fresh
//! TNT, one in a dark flash band, and a swollen one near the end of its fuse. Saves
//! `target/render-checks/falling-block-tnt.png`. Needs the client jar, the datagen
//! reports and a Vulkan driver (source scripts/env.sh).

use haste_core::model::{
    BodyTransform, Decoration, EntityInstance, FallingBlockDecoration, PoseOverrides,
    PrimedTntDecoration,
};
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

/// One moving-block entity (the model is unused; the renderer draws the decoration).
fn instance(id: i32, pos: glam::DVec3, decoration: Decoration) -> EntityInstance {
    EntityInstance {
        id: EntityId(id),
        model: haste_entity::models::box_for(1.0, 1.0),
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
        // `FallingBlockRenderer`/`TntRenderer`: shadow radius 0.5.
        shadow_radius: 0.5,
        shadow_strength: 1.0,
        invisible: false,
        display_fire: false,
        width: 0.98,
        height: 0.98,
        is_discrete: false,
        name_tag_y: 1.0,
        hurt: 0.0,
        light: (15, 0),
        overlays: Default::default(),
        item: None,
        decoration: Some(decoration),
        hands: [None, None],
        equipment: Vec::new(),
        head_item: None,
    }
}

fn render_scene(
    renderer: &mut Renderer,
    world: &dyn haste_core::view::BlockView,
    shapes: &haste_data::BlockShapes,
    entities: &mut [EntityInstance],
    camera: Camera,
) -> image::RgbaImage {
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
    // `sample_world` extracts the entity shadows (the falling blocks' radius 0.5 discs on
    // the floor below), like the app's frame path.
    renderer.sample_world(world, shapes, &frame);
    renderer.render(&frame).unwrap();
    renderer.read_back().unwrap()
}

fn diff(a: &image::RgbaImage, b: &image::RgbaImage) -> u64 {
    a.pixels()
        .zip(b.pixels())
        .filter(|(a, b)| a.0 != b.0)
        .count() as u64
}

/// The vanilla shadow texture (`misc/shadow.png`): the disc's alpha profile, which the
/// shadow pass multiplies into the piece alpha (vanilla `ShadowFeatureRenderer`).
#[test]
#[ignore = "needs the client jar (source scripts/env.sh)"]
fn shadow_texture_profile() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let img = assets
        .texture_any(&Identifier::minecraft("misc/shadow"))
        .unwrap();
    let (w, h) = (img.width(), img.height());
    eprintln!("shadow.png {w}x{h}");
    for (x, y) in [
        (w / 2, h / 2),
        (w / 2, h / 4),
        (w / 4, h / 2),
        (w / 4, h / 4),
        (3 * w / 4, 3 * h / 4),
        (w - 1, h - 1),
    ] {
        let p = img.get_pixel(x, y);
        eprintln!("  ({x},{y}): rgba {:?}", p.0);
    }
    let opaque = img.pixels().filter(|p| p.0[3] > 250).count();
    eprintln!("  fully opaque texels: {opaque} of {}", w * h);
}

#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn falling_blocks_and_tnt_offscreen() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let mut renderer = Renderer::new_offscreen(640, 480, &mut assets, &data.blocks).unwrap();
    renderer.set_item_registry(&data.registries);
    let world = Flat {
        stone: data
            .blocks
            .default_state(&Identifier::minecraft("stone"))
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
        // The live comparison camera: looking north (yaw 180) down 15 degrees, like the
        // falling-ref.png shot, from 4.5 blocks in front of the entities.
        pos: glam::DVec3::new(1.0, 2.62, 4.5),
        yaw: 180.0,
        pitch: 15.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    };
    let sand = data
        .blocks
        .default_state(&Identifier::minecraft("sand"))
        .expect("sand state");
    let tnt = data
        .blocks
        .default_state(&Identifier::minecraft("tnt"))
        .expect("tnt state");
    // The row: falling sand hovering half a block above the floor, a landed sand (draws
    // nothing), then a fresh TNT (lit band), one in a dark band, and one swollen in its
    // last ticks.
    let moving = |id, x, d| instance(id, glam::DVec3::new(x, 1.0, 0.0), d);
    let mut entities = vec![
        instance(
            1,
            glam::DVec3::new(-2.0, 1.5, 0.0),
            Decoration::FallingBlock(FallingBlockDecoration {
                block_state: sand,
                landed: false,
            }),
        ),
        moving(
            2,
            -0.5,
            Decoration::FallingBlock(FallingBlockDecoration {
                block_state: sand,
                landed: true,
            }),
        ),
        moving(
            3,
            1.0,
            Decoration::PrimedTnt(PrimedTntDecoration {
                block_state: tnt,
                fuse: 81.0,
            }),
        ),
        moving(
            4,
            2.5,
            Decoration::PrimedTnt(PrimedTntDecoration {
                block_state: tnt,
                fuse: 76.0,
            }),
        ),
        moving(
            5,
            4.0,
            Decoration::PrimedTnt(PrimedTntDecoration {
                block_state: tnt,
                fuse: 3.0,
            }),
        ),
    ];
    let base = render_scene(&mut renderer, &world, &data.shapes, &mut [], camera);
    let img = render_scene(&mut renderer, &world, &data.shapes, &mut entities, camera);
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    std::fs::create_dir_all(&out).unwrap();
    img.save(out.join("falling-block-tnt.png")).unwrap();
    // The falling sand and the three TNT blocks draw (the landed sand does not).
    assert!(
        diff(&base, &img) > 2000,
        "the falling sand and TNT changed too few pixels"
    );
    // Their vanilla shadows draw too (`FallingBlockRenderer`/`TntRenderer` radius 0.5):
    // discs under each entity, the hovering sand's faded by its half block of height
    // (`extractShadowPiece`'s `pow - height * 0.5`), the landed one's hidden with it.
    assert!(
        renderer.shadow_quad_count() >= 4,
        "the moving blocks cast {} shadow quads",
        renderer.shadow_quad_count()
    );
    renderer.set_entity_shadows(false);
    let no_shadow = render_scene(&mut renderer, &world, &data.shapes, &mut entities, camera);
    renderer.set_entity_shadows(true);
    assert_eq!(renderer.shadow_quad_count(), 0, "shadows off draws none");
    let shadow_pixels = diff(&img, &no_shadow);
    assert!(
        shadow_pixels > 200,
        "the shadows changed {shadow_pixels} pixels of the floor"
    );
    // The strength: over the pixels the shadow darkened, the mean on/off ratio is the
    // piece alpha at work — vanilla's hovering-sand disc centre is ~0.32
    // (`pow - height*0.5` at 0.63, times 0.5 brightness), the ref screenshot shows ~0.4.
    {
        let mut sum = [0u64; 2];
        let mut n = 0u64;
        for (a, b) in img.pixels().zip(no_shadow.pixels()) {
            if a.0 != b.0 {
                sum[0] += u64::from(a.0[0]) + u64::from(a.0[1]) + u64::from(a.0[2]);
                sum[1] += u64::from(b.0[0]) + u64::from(b.0[1]) + u64::from(b.0[2]);
                n += 1;
            }
        }
        let (mean_on, mean_off) = (
            sum[0] as f64 / (3.0 * n as f64),
            sum[1] as f64 / (3.0 * n as f64),
        );
        let darkening = 1.0 - mean_on / mean_off;
        eprintln!("shadow darkening over {n} pixels: {darkening:.3}");
        assert!(
            (0.27..0.36).contains(&darkening),
            "the disc darkens {darkening}, expected the vanilla ~0.32"
        );
    }
    // The landed sand draws nothing: dropping it from the scene changes not one pixel.
    entities.remove(1);
    let without_landed = render_scene(&mut renderer, &world, &data.shapes, &mut entities, camera);
    assert_eq!(
        diff(&img, &without_landed),
        0,
        "a landed falling block drew something"
    );
    // The flash band changes the TNT's colour: fuse 81 is lit, 76 is dark. (The landed
    // sand is gone from the list, so index 2 is the dark-band TNT.)
    entities[2].decoration = Some(Decoration::PrimedTnt(PrimedTntDecoration {
        block_state: tnt,
        fuse: 81.0,
    }));
    let lit = render_scene(&mut renderer, &world, &data.shapes, &mut entities, camera);
    assert!(
        diff(&img, &lit) > 200,
        "the white flash changed too few pixels"
    );
    // The last ten ticks swell the block: fuse 3 next to fuse 81 (index 3).
    entities[3].decoration = Some(Decoration::PrimedTnt(PrimedTntDecoration {
        block_state: tnt,
        fuse: 81.0,
    }));
    let unswollen = render_scene(&mut renderer, &world, &data.shapes, &mut entities, camera);
    assert!(
        diff(&img, &unswollen) > 200,
        "the swell changed too few pixels"
    );
}
