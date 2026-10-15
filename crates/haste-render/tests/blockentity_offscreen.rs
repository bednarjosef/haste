//! Block entities checked offscreen: a chest and a shulker box on a stone floor, drawn closed
//! and open, and the lid animation between them. Saves `target/render-checks/chest.png` and
//! `target/render-checks/chest-open.png`. Needs the client jar, the datagen reports and a
//! Vulkan driver (source scripts/env.sh).

use haste_core::{BlockPos, BlockStateId, ChunkPos, Direction, SectionPos};
use haste_render::blockentity::{BlockEntityInstance, BlockEntityKind, ChestHalf, ChestMaterial};
use haste_render::{Camera, FrameInput, Renderer, SkyState, TickPair};

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

fn render(
    renderer: &mut Renderer,
    _world: &Flat,
    block_entities: &[BlockEntityInstance],
    camera: Camera,
) -> image::RgbaImage {
    let frame = FrameInput {
        camera,
        entities: &[],
        block_entities,
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
    renderer.read_back().unwrap()
}

#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn block_entities_offscreen() {
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

    // A chest and a shulker box on the floor at y = 1, seen from z = -4 at eye height. The
    // chest faces the camera (its front, the lock side, points south = +z = toward us).
    let chest = BlockEntityInstance {
        pos: BlockPos::new(0, 1, 0),
        facing: Direction::South,
        half: ChestHalf::Single,
        kind: BlockEntityKind::Chest(ChestMaterial::Normal),
        open: TickPair::fixed(0.0),
        light: (15, 0),
        sign_angle: 0.0,
        text: None,
        banner_phase: 0.0,
        banner: None,
        skull_anim: TickPair::fixed(0.0),
        anim: None,
        sherds: None,
        beam: None,
        gateway_beam: None,
        portal_faces: 0,
        piston: None,
    };
    let shulker = BlockEntityInstance {
        pos: BlockPos::new(2, 1, 0),
        facing: Direction::Up,
        half: ChestHalf::Single,
        kind: BlockEntityKind::Shulker(None),
        open: TickPair::fixed(0.0),
        light: (15, 0),
        sign_angle: 0.0,
        text: None,
        banner_phase: 0.0,
        banner: None,
        skull_anim: TickPair::fixed(0.0),
        anim: None,
        sherds: None,
        beam: None,
        gateway_beam: None,
        portal_faces: 0,
        piston: None,
    };
    // A sign on the floor at (0, 2, 2), facing the camera: the front text is drawn with the
    // default font into the entity stream. The board itself is the chunk's block model (drawn
    // by the mesher when the block is there; here only the text is drawn, the block below the
    // sign is a placeholder floor in this synthetic scene).
    let sign_text = std::sync::Arc::new(haste_render::blockentity::SignTextData {
        front: haste_render::blockentity::SignSideText {
            lines: [
                "Haste".to_string(),
                "signs work".to_string(),
                String::new(),
                "line 4".to_string(),
            ],
            color: 0,
            glowing: false,
        },
        back: haste_render::blockentity::SignSideText {
            lines: std::array::from_fn(|_| String::new()),
            color: 15,
            glowing: false,
        },
    });
    let sign = BlockEntityInstance {
        pos: BlockPos::new(-2, 1, 0),
        facing: Direction::South,
        half: ChestHalf::Single,
        kind: BlockEntityKind::Sign { hanging: false },
        open: TickPair::fixed(0.0),
        light: (15, 0),
        // Rotation segment 8 = 180 degrees: the front faces north, toward the camera (which
        // looks south from z = -4).
        sign_angle: 180.0,
        text: Some(sign_text),
        banner_phase: 0.0,
        banner: None,
        skull_anim: TickPair::fixed(0.0),
        anim: None,
        sherds: None,
        beam: None,
        gateway_beam: None,
        portal_faces: 0,
        piston: None,
    };
    // A standing banner next to the sign, red base with a blue stripe: pole, bar and waving
    // flag with the base and pattern layers.
    let banner = BlockEntityInstance {
        pos: BlockPos::new(-4, 1, 0),
        facing: Direction::South,
        half: ChestHalf::Single,
        kind: BlockEntityKind::Banner { wall: false },
        open: TickPair::fixed(0.0),
        light: (15, 0),
        sign_angle: 180.0,
        text: None,
        banner_phase: 0.25,
        banner: Some(std::sync::Arc::new(haste_render::blockentity::BannerData {
            base: 14,
            layers: vec![haste_render::blockentity::BannerLayer {
                pattern: haste_core::Identifier::minecraft("stripe_bottom"),
                color: 11,
            }],
        })),
        skull_anim: TickPair::fixed(0.0),
        anim: None,
        sherds: None,
        beam: None,
        gateway_beam: None,
        portal_faces: 0,
        piston: None,
    };
    // A skeleton skull on the floor between the sign and the banner, facing the camera: one
    // head cube in the block's bottom half.
    let skull = BlockEntityInstance {
        pos: BlockPos::new(-3, 1, 0),
        facing: Direction::South,
        half: ChestHalf::Single,
        kind: BlockEntityKind::Skull {
            kind: haste_render::blockentity::SkullKind::Skeleton,
            wall: false,
        },
        open: TickPair::fixed(0.0),
        light: (15, 0),
        // Rotation segment 8 = 180 degrees: the face points north, toward the camera.
        sign_angle: 180.0,
        text: None,
        banner_phase: 0.0,
        banner: None,
        skull_anim: TickPair::fixed(0.0),
        anim: None,
        sherds: None,
        beam: None,
        gateway_beam: None,
        portal_faces: 0,
        piston: None,
    };
    // The camera looks at the chest's centre from four blocks south and 1.5 up: the chest
    // lands mid-frame.
    let camera = Camera {
        pos: glam::DVec3::new(0.5, 2.6, -4.0),
        yaw: 0.0,
        pitch: 18.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    };

    // Warm-up renders: the section-visibility walk lags a frame (the previous mask is drawn
    // until the new one arrives), so the first frames cull everything.
    for _ in 0..4 {
        render(&mut renderer, &world, &[], camera);
    }

    let closed = render(
        &mut renderer,
        &world,
        &[chest.clone(), shulker.clone()],
        camera,
    );
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    std::fs::create_dir_all(&out).unwrap();
    closed.save(out.join("chest.png")).unwrap();
    let empty = render(&mut renderer, &world, &[], camera);
    empty.save(out.join("chest-empty.png")).unwrap();

    // The sign's text: drawn through the entity stream (one more block entity in the same
    // draw call). Its pixels differ from the empty scene where the text is.
    let signed = render(&mut renderer, &world, &[sign], camera);
    signed.save(out.join("sign.png")).unwrap();
    let mut sign_pixels = 0;
    for (x, y, p) in signed.enumerate_pixels() {
        if *p != *empty.get_pixel(x, y) {
            let _ = (x, y);
            sign_pixels += 1;
        }
    }
    assert!(
        sign_pixels > 30,
        "sign text draws {sign_pixels} differing pixels"
    );

    // The banner: the pole, bar and waving flag with the base and pattern layers, again one
    // draw with everything. Its pixels differ from the empty scene, and the flag's red base
    // dominates the banner's pixels (a red-dominant pixel count).
    let with_banner = render(&mut renderer, &world, &[banner], camera);
    with_banner.save(out.join("banner.png")).unwrap();
    let mut banner_pixels = 0;
    let mut red_pixels = 0;
    for (x, y, p) in with_banner.enumerate_pixels() {
        if *p != *empty.get_pixel(x, y) {
            let _ = (x, y);
            banner_pixels += 1;
            let [r, g, b, _] = p.0;
            if r as i32 - g.max(b) as i32 > 40 {
                red_pixels += 1;
            }
        }
    }
    assert!(
        banner_pixels > 500,
        "banner draws {banner_pixels} differing pixels"
    );
    assert!(
        red_pixels > 100,
        "banner base is red: {red_pixels} red pixels"
    );

    // The skull: one head cube in the lower half of the block, drawn with the skeleton
    // texture in the same entity draw call.
    let with_skull = render(&mut renderer, &world, &[skull], camera);
    with_skull.save(out.join("skull.png")).unwrap();
    let mut skull_pixels = 0;
    for (x, y, p) in with_skull.enumerate_pixels() {
        if *p != *empty.get_pixel(x, y) {
            let _ = (x, y);
            skull_pixels += 1;
        }
    }
    assert!(
        skull_pixels > 200,
        "skull draws {skull_pixels} differing pixels"
    );

    // The chest sits at the frame centre (the camera looks at it): its pixels there differ
    // from the empty scene, and they are the chest's brown, not the sky above it.
    let centre = closed.get_pixel(320, 240).0;
    assert_ne!(centre, empty.get_pixel(320, 240).0, "chest at the centre");
    let sky = closed.get_pixel(320, 30).0;
    assert_ne!(centre, sky, "chest is not sky");
    let (r, g, b) = (centre[0] as i32, centre[1] as i32, centre[2] as i32);
    assert!(
        r > g && g > b && r > 60,
        "chest centre is wooden brown, got {centre:?}"
    );

    // The shulker box, two blocks east of the chest: seen looking south, east is screen
    // left. One block is about 77 px at this distance (the camera is 4.5 blocks away); the
    // shulker centre is 2.5 blocks east of the camera.
    let shulker_x = (320.0 - 2.5 / 4.17 * 320.0) as u32;
    let shulker_pixel = closed.get_pixel(shulker_x.clamp(0, 639), 240).0;
    assert_ne!(
        shulker_pixel,
        empty.get_pixel(shulker_x.clamp(0, 639), 240).0,
        "shulker box east of the chest"
    );

    // The open chest: the lid swings up, so the frame differs from the closed one around the
    // chest (and the lid rises above the block, into what was sky).
    let mut open = chest.clone();
    open.open = TickPair::fixed(0.5);
    let half = render(
        &mut renderer,
        &world,
        &[open.clone(), shulker.clone()],
        camera,
    );
    half.save(out.join("chest-open.png")).unwrap();
    let mut differing = 0;
    for (x, y, p) in closed.enumerate_pixels() {
        if p != half.get_pixel(x, y) {
            differing += 1;
        }
    }
    assert!(differing > 2000, "lid moved {differing} pixels");
    // The shulker box itself is unchanged by the chest's lid.
    assert_eq!(
        closed.get_pixel(shulker_x.clamp(0, 639), 240),
        half.get_pixel(shulker_x.clamp(0, 639), 240)
    );

    // Fully open: more of the lid has moved up than at half open.
    open.open = TickPair::fixed(1.0);
    let full = render(
        &mut renderer,
        &world,
        &[open.clone(), shulker.clone()],
        camera,
    );
    let mut differing_full = 0;
    for (x, y, p) in closed.enumerate_pixels() {
        if p != full.get_pixel(x, y) {
            differing_full += 1;
        }
    }
    assert!(
        differing_full > differing,
        "full open ({differing_full}) moves more than half ({differing})"
    );
    full.save(out.join("chest-full.png")).unwrap();
}
