//! Arrows checked offscreen: a flying arrow, a stuck one mid-shake, a tipped and a spectral
//! one over a stone floor, drawn through the entity batch like every other entity. Saves
//! `target/render-checks/arrows.png`. Needs the client jar, the datagen reports and a Vulkan
//! driver (source scripts/env.sh).

use haste_core::model::{
    ArrowDecoration, BodyTransform, Decoration, EntityInstance, PoseOverrides,
};
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

/// One arrow instance: the decoration carries the whole render state (the box model is what
/// the tracker would hand over; the arrow path draws from the decoration instead).
fn arrow(id: i32, x: f64, y: f64, a: ArrowDecoration) -> EntityInstance {
    let model = haste_entity::models::box_for(0.5, 0.5);
    EntityInstance {
        id: EntityId(id),
        model,
        pos: glam::DVec3::new(x, y, 0.0),
        yaw: a.y_rot,
        pitch: a.x_rot,
        head_yaw: a.y_rot,
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
        width: 0.5,
        height: 0.5,
        is_discrete: false,
        name_tag_y: 0.5,
        hurt: 0.0,
        light: (15, 0),
        overlays: Default::default(),
        item: None,
        decoration: Some(Decoration::Arrow(a)),
        hands: [None, None],
        equipment: Vec::new(),
        head_item: None,
        hand_foils: Default::default(),
        head_item_foil: Default::default(),
    }
}

#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn arrows_offscreen() {
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

    // Four arrows 3-4 blocks in front of the camera at y = 1.5, flying to the +x of the frame
    // (the camera at z = -4 looks towards +z): a plain one, the same stuck mid-shake, a tipped
    // one and a spectral one.
    let deco = |y_rot, x_rot, shake, tipped, spectral| ArrowDecoration {
        x_rot,
        y_rot,
        shake,
        tipped,
        spectral,
    };
    let mut entities = vec![
        arrow(1, -1.5, 1.5, deco(90.0, 0.0, 0.0, false, false)),
        arrow(2, -0.5, 1.5, deco(90.0, -35.0, 5.0, false, false)),
        arrow(3, 0.5, 1.5, deco(90.0, 0.0, 0.0, true, false)),
        arrow(4, 1.5, 1.5, deco(90.0, 0.0, 0.0, false, true)),
    ];
    for e in entities.iter_mut() {
        e.sample_light(&world);
    }
    let frame = FrameInput {
        camera: Camera {
            pos: glam::DVec3::new(0.0, 2.2, -4.0),
            yaw: 0.0,
            pitch: 5.0,
            fov_deg: 70.0,
            hand_fov_deg: 70.0,
            roll_deg: 0.0,
            bob: haste_render::ViewBob::default(),
        },
        entities: &entities,
        block_entities: &[],
        gui: &Default::default(),
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
    img.save(out.join("arrows.png")).unwrap();

    // Each arrow sits in its own quarter of the frame: those columns differ from the empty
    // scene (sky or floor), so all four drew.
    let empty = {
        let mut frame = frame;
        frame.entities = &[];
        renderer.render(&frame).unwrap();
        renderer.read_back().unwrap()
    };
    let diffs = |x0: u32, x1: u32, y0: u32, y1: u32| {
        (x0..x1)
            .flat_map(|x| (y0..y1).map(move |y| (x, y)))
            .filter(|(x, y)| img.get_pixel(*x, *y).0 != empty.get_pixel(*x, *y).0)
            .count()
    };
    // The four arrows are spread across the frame's columns at roughly its vertical middle;
    // each quarter must hold a visible cluster of changed pixels.
    for (x0, which) in [
        (0u32, "plain"),
        (160, "shaking"),
        (320, "tipped"),
        (480, "spectral"),
    ] {
        assert!(
            diffs(x0, x0 + 160, 180, 320) > 20,
            "{which} arrow did not draw"
        );
    }
}

/// Fifty arrows' cost on the frame path: the CPU posing (`prepare_ms`, which runs per
/// entity) and the entities GPU sub-pass, averaged over 300 frames, against the same scene
/// without arrows. Timed like `haste-entity`'s ten-thousand-entities test: run with
/// `cargo test --release -p haste-render --test arrow_offscreen -- --ignored`.
#[test]
#[ignore = "timed test, run in release: cargo test --release -p haste-render --test arrow_offscreen -- --ignored"]
fn fifty_arrows_frame_cost() {
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
    let mut arrows: Vec<EntityInstance> = (0..50)
        .map(|i| {
            arrow(
                i,
                -4.0 + f64::from(i) * 0.16,
                1.5,
                ArrowDecoration {
                    x_rot: -20.0,
                    y_rot: 90.0,
                    shake: (i % 8) as f32,
                    tipped: i % 2 == 0,
                    spectral: i % 3 == 0,
                },
            )
        })
        .collect();
    let camera = Camera {
        pos: glam::DVec3::new(0.0, 2.2, -4.0),
        yaw: 0.0,
        pitch: 5.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    };
    let gui = haste_core::gui::GuiFrame::default();
    let average = |renderer: &mut Renderer, entities: &mut [EntityInstance]| {
        for e in entities.iter_mut() {
            e.sample_light(&world);
        }
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
        let (mut prep, mut ent, mut n) = (0.0f32, 0.0f32, 0usize);
        for _ in 0..300 {
            renderer.render(&frame).unwrap();
            let t = renderer.timings();
            prep += t.prepare_ms;
            ent += t.gpu_sub_ms[haste_render::timing::ENTITIES as usize].max(0.0);
            n += usize::from(t.gpu_sub_ms[haste_render::timing::ENTITIES as usize] > 0.0);
        }
        (prep / 300.0, if n > 0 { ent / n as f32 } else { 0.0 })
    };
    let empty = average(&mut renderer, &mut Vec::new());
    let full = average(&mut renderer, &mut arrows);
    eprintln!(
        "50 arrows: prepare {:.3} -> {:.3} ms (CPU posing), entities sub-pass {:.3} -> {:.3} ms (GPU)",
        empty.0, full.0, empty.1, full.1
    );
    // The posing of fifty arrows must stay well under a millisecond.
    assert!(
        full.0 - empty.0 < 1.0,
        "fifty arrows cost {:.3} vs {:.3} ms",
        full.0,
        empty.0
    );
}
