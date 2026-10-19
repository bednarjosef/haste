//! Boats checked offscreen: an oak boat, a cherry chest boat and a bamboo raft posed on a
//! stone floor, plus the water patch's depth-only draw. Saves
//! `target/render-checks/boats.png`. Needs the client jar, the datagen reports, the model
//! dump and a Vulkan driver (source scripts/env.sh).

use haste_core::model::{BodyTransform, EntityInstance, PoseOverrides};
use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId, ChunkPos, EntityId, SectionPos};
use haste_render::{Camera, FrameInput, Renderer, SkyState};

struct Flat {
    stone: BlockStateId,
}

impl BlockView for Flat {
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

/// The instance `haste-entity`'s store builds for a floating boat of `type_name`: the dumped
/// model, the water patch overlay (never a raft's), the 0.375 lift and the entity's yaw.
fn boat_instance(id: i32, type_name: &str, pos: glam::DVec3, yaw: f32) -> EntityInstance {
    let entry = haste_entity::boat::entry_for_name(type_name).unwrap();
    let model = haste_entity::boat::model(entry).unwrap();
    let overlays = match haste_entity::boat::water_patch_overlay(
        entry,
        &haste_entity::boat::BoatState::default(),
    ) {
        Some(patch) => [Some(patch), None],
        None => [None, None],
    };
    EntityInstance {
        id: EntityId(id),
        model,
        pos,
        yaw,
        pitch: 0.0,
        head_yaw: yaw,
        scale: 1.0,
        pose: PoseOverrides::default(),
        body: BodyTransform {
            pre_translate: glam::Vec3::new(0.0, 0.375, 0.0),
            ..Default::default()
        },
        body_scale: glam::Vec3::ONE,
        skin: None,
        nametag: None,
        below_name: None,
        shadow_radius: 0.8,
        shadow_strength: 1.0,
        invisible: false,
        display_fire: false,
        width: 1.375,
        height: 0.5625,
        is_discrete: false,
        name_tag_y: 0.5625,
        hurt: 0.0,
        light: (15, 0),
        overlays,
        item: None,
        decoration: None,
        hands: [None, None],
        equipment: Vec::new(),
        head_item: None,
    }
}

#[test]
#[ignore = "needs the client jar, datagen reports, the model dump and a Vulkan driver (source scripts/env.sh)"]
fn boats_offscreen() {
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

    // An oak boat ahead of the camera (turned to show bow and side), a cherry chest boat
    // beside it, a bamboo raft behind. All three draw from their dumped models.
    let camera = Camera {
        pos: glam::DVec3::new(0.0, 2.2, -4.0),
        yaw: 0.0,
        pitch: 12.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    };
    let mut entities = vec![
        boat_instance(
            1,
            "minecraft:oak_boat",
            glam::DVec3::new(0.0, 1.0, 0.0),
            30.0,
        ),
        boat_instance(
            2,
            "minecraft:cherry_chest_boat",
            glam::DVec3::new(-1.6, 1.0, 0.8),
            -15.0,
        ),
        boat_instance(
            3,
            "minecraft:bamboo_raft",
            glam::DVec3::new(1.6, 1.0, 1.2),
            60.0,
        ),
    ];
    for e in entities.iter_mut() {
        e.sample_light(&world);
    }
    let frame = FrameInput {
        camera,
        entities: &entities,
        block_entities: &[],
        gui: &haste_core::gui::GuiFrame::default(),
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
    // The same frame without the boats: whatever changed between the two is the boats (and
    // their shadows), not leftover state.
    renderer
        .render(&FrameInput {
            camera,
            entities: &[],
            block_entities: &[],
            gui: &haste_core::gui::GuiFrame::default(),
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
        })
        .unwrap();
    let empty = renderer.read_back().unwrap();
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    std::fs::create_dir_all(&out).unwrap();
    img.save(out.join("boats.png")).unwrap();

    // The three boats and their shadows cover a solid patch of the middle of the frame (a
    // boat is 1.75 blocks wide at 4 blocks' distance at 70 degrees: hundreds of pixels each;
    // the shadows add more). Looking into a hull also shows its floor and the stone through
    // it, so the diff counts only what the boats actually covered.
    let mut changed = 0;
    for y in 100..460 {
        for x in 60..580 {
            if img.get_pixel(x, y).0 != empty.get_pixel(x, y).0 {
                changed += 1;
            }
        }
    }
    assert!(
        changed > 2_000,
        "the boats covered only {changed} pixels of the 640x480 frame"
    );
}

/// Compares Haste's live shot of the lead's fixed boat scene against the vanilla reference
/// (`cache/oracle/26.3/screenshots/boats-ref.png`, recorded by the real 26.3 client at
/// 1408x828, spectator, feet 3003.5 200.5 3000.0, yaw 0, pitch 30, HUD off; Haste's shot is
/// `target/shots/script-boats-haste.png` from the same camera). Prints the overall match and
/// a per-region table over the three boats (oak at x 3000.5, chest boat at 3003.5, raft at
/// 3006.5 - left, centre and right of the frame), and saves the red-highlighted diff to
/// `target/render-checks/boats-ref-diff.png`.
#[test]
#[ignore = "needs the vanilla reference screenshot and target/shots/script-boats-haste.png (dev evidence)"]
fn boats_match_the_vanilla_reference() {
    let reference = haste_core::paths::oracle_dir().join("screenshots/boats-ref.png");
    let shot = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/shots/script-boats-haste.png");
    let reference = image::open(&reference)
        .unwrap_or_else(|e| {
            panic!(
                "opening the vanilla reference: {e} (path: {})",
                reference.display()
            )
        })
        .to_rgba8();
    let haste = image::open(&shot)
        .unwrap_or_else(|e| panic!("opening Haste's shot: {e} (path: {})", shot.display()))
        .to_rgba8();
    assert_eq!(
        reference.dimensions(),
        haste.dimensions(),
        "both shots must be 1408x828"
    );
    let (w, h) = reference.dimensions();

    // Per-pixel channel error, and the fraction of a region beyond tolerance 24 (vanilla
    // and Haste shade and fog slightly differently; a real mismatch lights a boat's whole
    // region up).
    const TOLERANCE: u8 = 24;
    let err = |a: &image::Rgba<u8>, b: &image::Rgba<u8>| -> u8 {
        (0..4)
            .map(|i| (a[i] as i32 - b[i] as i32).unsigned_abs())
            .max()
            .unwrap_or(0) as u8
    };
    let region = |x0: u32, x1: u32, y0: u32, y1: u32| -> (f64, f64) {
        let (mut differing, mut sum, mut n) = (0u64, 0u64, 0u64);
        for y in y0..y1 {
            for x in x0..x1 {
                let e = err(reference.get_pixel(x, y), haste.get_pixel(x, y));
                sum += e as u64;
                n += 1;
                if e > TOLERANCE {
                    differing += 1;
                }
            }
        }
        (differing as f64 / n as f64, sum as f64 / n as f64)
    };
    let (frac, mean) = region(0, w, 0, h);
    eprintln!("overall: {frac:.1}% beyond tolerance {TOLERANCE}, mean error {mean:.2}/255");
    // The three boats from the fixed camera (yaw 0, pitch 30): left, centre, right thirds of
    // the lower half; the top is sky, which only the fog and sun position can move.
    for (name, x0, x1) in [
        ("oak (left)", 60, 480),
        ("chest (centre)", 480, 930),
        ("raft (right)", 930, 1350),
    ] {
        let (frac, mean) = region(x0, x1, h / 3, h);
        eprintln!("{name}: {frac:.1}% beyond tolerance, mean error {mean:.2}");
    }
    let (frac, mean) = region(0, w, 0, h / 3);
    eprintln!("sky (top third): {frac:.1}% beyond tolerance, mean error {mean:.2}");

    // Save the diff (white within tolerance, red beyond) for the lead's eyes.
    let mut diff = image::RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let e = err(reference.get_pixel(x, y), haste.get_pixel(x, y));
            let c = if e > TOLERANCE {
                [255, 0, 0, 255]
            } else {
                [e, e, e, 255]
            };
            diff.put_pixel(x, y, image::Rgba(c));
        }
    }
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    std::fs::create_dir_all(&out).unwrap();
    diff.save(out.join("boats-ref-diff.png")).unwrap();
}
