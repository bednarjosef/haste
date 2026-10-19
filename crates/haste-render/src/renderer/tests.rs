use super::readback::{texel_rows_to_rgba, CaptureFlag, CapturePhase};
use super::*;

#[test]
#[ignore = "needs the client jar, datagen reports, and a Vulkan driver (source scripts/env.sh)"]
fn offscreen_render_flat_world() {
    let jar = haste_core::paths::client_jar();
    if !jar.exists() {
        eprintln!("ignored: needs the client jar in the cache");
        return;
    }
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::cache_dir().join("assets"), "26.3")
            .unwrap();
    let states = haste_assets::BlocksJsonStates::load(
        &haste_core::paths::datagen_reports().join("blocks.json"),
    )
    .unwrap();

    let mut renderer = Renderer::new_offscreen(320, 240, &mut assets, &states).unwrap();

    // A flat stone world at y=0.
    struct Flat;
    impl haste_core::view::BlockView for Flat {
        fn block_state(&self, p: haste_core::BlockPos) -> haste_core::BlockStateId {
            if p.y == 0 {
                haste_core::BlockStateId(1)
            } else {
                haste_core::BlockStateId::AIR
            }
        }
        fn is_chunk_loaded(&self, _: haste_core::ChunkPos) -> bool {
            true
        }
        fn sky_light(&self, p: haste_core::BlockPos) -> u8 {
            if p.y <= 0 {
                0
            } else {
                15
            }
        }
        fn block_light(&self, _: haste_core::BlockPos) -> u8 {
            0
        }
        fn min_y(&self) -> i32 {
            -64
        }
        fn height(&self) -> i32 {
            384
        }
    }

    // Mesh and upload a few sections around the origin.
    let dirty: Vec<SectionPos> = (-1..=1)
        .flat_map(|x| (-1..=1).map(move |z| SectionPos::new(x, 0, z)))
        .collect();
    renderer.update_world(&Flat, &dirty, ChunkPos::new(0, 0), 4);
    // Meshing is asynchronous: keep draining until the sections are uploaded.
    for _ in 0..200 {
        renderer.update_world(&Flat, &[], ChunkPos::new(0, 0), 4);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    let frame = FrameInput {
        camera: Camera {
            pos: glam::DVec3::new(8.0, 20.0, 8.0),
            yaw: 0.0,
            pitch: 60.0,
            fov_deg: 70.0,
            hand_fov_deg: 70.0,
            roll_deg: 0.0,
            bob: ViewBob::default(),
        },
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
    };
    // The section visibility walk runs off the render thread and lands a frame later
    // (vanilla draws the previous graph until the async update is swapped in), so keep
    // rendering until its mask arrives instead of asserting on the empty first frame.
    let mut stats = renderer.render(&frame).unwrap();
    for _ in 0..200 {
        if stats.chunk_sections_drawn > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        stats = renderer.render(&frame).unwrap();
    }
    eprintln!(
        "offscreen frame: {:.2} ms, {} drawn / {} sections, {} draw calls",
        stats.frame_ms, stats.chunk_sections_drawn, stats.chunk_sections_total, stats.draw_calls
    );
    assert!(stats.chunk_sections_drawn > 0, "no sections drawn");

    let image = renderer.read_back().unwrap();
    assert_eq!(image.dimensions(), (320, 240));
    // The frame must not be entirely the clear colour: some pixel differs
    // from the sky blue (0.47, 0.65, 1.0) => (120, 165, 255).
    let sky = image::Rgba([120, 165, 255, 255]);
    let any_different = image.pixels().any(|p| p.0 != sky.0);
    assert!(
        any_different,
        "frame is entirely clear colour; terrain not rendered"
    );
}

/// The armour stand renders end to end: a tracked stand (full iron armour, ShowArms, a
/// custom pose and a held sword) drawn offscreen through the real jar texture and equipment
/// assets changes the frame against the same world without it (`ArmorStandRenderer`
/// through the store, the batched entity pass and the armour layers).
#[test]
#[ignore = "needs the client jar, datagen reports, and a Vulkan driver (source scripts/env.sh)"]
fn offscreen_renders_armor_stand_with_armour() {
    let jar = haste_core::paths::client_jar();
    if !jar.exists() {
        eprintln!("ignored: needs the client jar in the cache");
        return;
    }
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::cache_dir().join("assets"), "26.3")
            .unwrap();
    let states = haste_assets::BlocksJsonStates::load(
        &haste_core::paths::datagen_reports().join("blocks.json"),
    )
    .unwrap();
    let mut renderer = Renderer::new_offscreen(320, 240, &mut assets, &states).unwrap();
    let data = haste_data::GameData::load().unwrap();
    let registries = &data.registries;
    renderer.set_item_registry(registries);

    // A flat stone world at y=0 (the world the other offscreen test uses).
    struct Flat;
    impl haste_core::view::BlockView for Flat {
        fn block_state(&self, p: haste_core::BlockPos) -> haste_core::BlockStateId {
            if p.y == 0 {
                haste_core::BlockStateId(1)
            } else {
                haste_core::BlockStateId::AIR
            }
        }
        fn is_chunk_loaded(&self, _: haste_core::ChunkPos) -> bool {
            true
        }
        fn sky_light(&self, p: haste_core::BlockPos) -> u8 {
            if p.y <= 0 {
                0
            } else {
                15
            }
        }
        fn block_light(&self, _: haste_core::BlockPos) -> u8 {
            0
        }
        fn min_y(&self) -> i32 {
            -64
        }
        fn height(&self) -> i32 {
            384
        }
    }
    let dirty: Vec<SectionPos> = (-1..=1)
        .flat_map(|x| (-1..=1).map(move |z| SectionPos::new(x, 0, z)))
        .collect();
    renderer.update_world(&Flat, &dirty, ChunkPos::new(0, 0), 4);
    for _ in 0..200 {
        renderer.update_world(&Flat, &[], ChunkPos::new(0, 0), 4);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    // The stand: full iron armour, a sword, ShowArms and a saluting pose, tracked through
    // the store like the client tracks any entity.
    let mut store = haste_entity::EntityStore::new(haste_entity::EntityDimensions::default());
    store.set_item_components(data.item_components.clone());
    let kind = *haste_entity::registry::entity_type_names()
        .and_then(|names| {
            names
                .iter()
                .find(|(_, name)| **name == "minecraft:armor_stand")
                .map(|(k, _)| k)
        })
        .expect("armor stand in the entity registry");
    let mut spawn = haste_core::entity::EntitySpawn {
        id: haste_core::EntityId(1),
        uuid: uuid::Uuid::new_v4(),
        kind,
        pos: glam::DVec3::new(8.0, 1.0, 8.0),
        yaw: 200.0,
        pitch: 0.0,
        head_yaw: 200.0,
        data: 0,
        velocity: glam::DVec3::ZERO,
    };
    store.spawn(&spawn);
    spawn.id = haste_core::EntityId(2);
    spawn.pos = glam::DVec3::new(6.5, 1.0, 8.0);
    store.spawn(&spawn); // a second stand next to it: small, in a golden helmet
                         // Client flags (index 15): ShowArms (0x04).
    store.set_metadata(
        haste_core::EntityId(1),
        &[haste_core::entity::MetadataEntry {
            index: 15,
            value: haste_core::entity::MetadataValue::Byte(0x04),
        }],
    );
    // A pose: the right arm raised to greet (`Pose.RightArm` -> index 19).
    store.set_metadata(
        haste_core::EntityId(1),
        &[haste_core::entity::MetadataEntry {
            index: 19,
            value: haste_core::entity::MetadataValue::Rotations([-170.0, 0.0, 10.0]),
        }],
    );
    let item = |name: &str| haste_core::entity::ItemStackData {
        count: 1,
        item: registries.index("minecraft:item", &haste_core::Identifier::minecraft(name)),
        components_raw: Default::default(),
    };
    // Wire slots: 0 mainhand, 2 feet, 3 legs, 4 chest, 5 head.
    for (wire, name) in [
        (0u8, "iron_sword"),
        (2, "iron_boots"),
        (3, "iron_leggings"),
        (4, "iron_chestplate"),
        (5, "iron_helmet"),
    ] {
        store.set_equipment(haste_core::EntityId(1), wire, item(name));
    }
    // The second stand: Small (0x01) in the client flags byte, in a golden helmet.
    store.set_metadata(
        haste_core::EntityId(2),
        &[haste_core::entity::MetadataEntry {
            index: 15,
            value: haste_core::entity::MetadataValue::Byte(0x01),
        }],
    );
    store.set_equipment(haste_core::EntityId(2), 5, item("golden_helmet"));
    let mut instances = Vec::new();
    store.render_instances(0.5, &mut instances);
    assert_eq!(instances.len(), 2);
    assert_eq!(
        instances[0].equipment.len(),
        4,
        "the four armour pieces resolved through the equippable component"
    );
    assert!(instances[0].hands[0].is_some(), "the held sword");

    let camera = Camera {
        pos: glam::DVec3::new(7.5, 3.6, 11.0),
        yaw: 180.0,
        pitch: 12.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: ViewBob::default(),
    };
    let gui = haste_core::gui::GuiFrame::default();
    fn base_frame<'a>(
        camera: Camera,
        gui: &'a haste_core::gui::GuiFrame,
        entities: &'a [haste_core::model::EntityInstance],
    ) -> FrameInput<'a> {
        FrameInput {
            camera,
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
            partial_tick: 0.5,
            block_target: None,
            breaking: &[],
            camera_entity: None,
            spin: None,
            weather: None,
        }
    }
    // The section visibility walk lands a frame later; keep rendering until it has.
    let mut stats = renderer.render(&base_frame(camera, &gui, &[])).unwrap();
    for _ in 0..200 {
        if stats.chunk_sections_drawn > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        stats = renderer.render(&base_frame(camera, &gui, &[])).unwrap();
    }
    assert!(stats.chunk_sections_drawn > 0, "no sections drawn");
    let without = renderer.read_back().unwrap();
    let _ = renderer
        .render(&base_frame(camera, &gui, &instances))
        .unwrap();
    let with = renderer.read_back().unwrap();
    std::fs::create_dir_all("target/shots").unwrap();
    with.save("target/shots/offscreen-stands.png").unwrap();
    let mut changed = 0;
    for (a, b) in without.pixels().zip(with.pixels()) {
        if a != b {
            changed += 1;
        }
    }
    // The two stands with their armour and sword cover a solid share of the frame at three
    // and a half blocks; a missing model, texture or layer would change nothing.
    assert!(
        changed > 1000,
        "the stands changed only {changed} of 76800 pixels; nothing drew"
    );
    // The held sword draws: the same scene with empty hands must differ (from this angle
    // the body hides most of it, so a tip's worth of pixels is a pass).
    let mut bare = instances.clone();
    for i in bare.iter_mut() {
        i.hands = [None, None];
    }
    let _ = renderer.render(&base_frame(camera, &gui, &bare)).unwrap();
    let bare_img = renderer.read_back().unwrap();
    let sword = with
        .pixels()
        .zip(bare_img.pixels())
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        sword > 100,
        "clearing the hands changed only {sword} pixels; the sword did not draw"
    );

    // The frame-path cost of a stand-heavy scene: the armoured instance cloned into a
    // 5x10 grid in front of the camera (50 stands, all in full armour with a sword — the
    // worst case), timed over warm frames against the same scene without them.
    let mut crowd = Vec::with_capacity(50);
    for i in 0..50u32 {
        let mut inst = instances[0].clone();
        inst.id = haste_core::EntityId((100 + i) as i32);
        inst.pos = glam::DVec3::new(5.5 + f64::from(i % 5), 1.0, 3.0 + f64::from(i / 5));
        crowd.push(inst);
    }
    let _ = renderer.render(&base_frame(camera, &gui, &crowd)).unwrap();
    let start = std::time::Instant::now();
    let frames = 100;
    for _ in 0..frames {
        renderer.render(&base_frame(camera, &gui, &crowd)).unwrap();
    }
    let with_ms = start.elapsed().as_secs_f32() * 1000.0 / frames as f32;
    let start = std::time::Instant::now();
    for _ in 0..frames {
        renderer.render(&base_frame(camera, &gui, &[])).unwrap();
    }
    let without_ms = start.elapsed().as_secs_f32() * 1000.0 / frames as f32;
    // ...and the same crowd as the generic boxes unknown types drew before the stand had
    // its own model, for the before/after pair.
    let boxes: Vec<_> = crowd
        .iter()
        .map(|inst| {
            let mut b = inst.clone();
            b.model = haste_entity::models::box_for(0.5, 1.975);
            b.equipment.clear();
            b.hands = [None, None];
            b
        })
        .collect();
    let _ = renderer.render(&base_frame(camera, &gui, &boxes)).unwrap();
    let start = std::time::Instant::now();
    for _ in 0..frames {
        renderer.render(&base_frame(camera, &gui, &boxes)).unwrap();
    }
    let boxes_ms = start.elapsed().as_secs_f32() * 1000.0 / frames as f32;
    eprintln!(
        "offscreen frame with 50 armoured stands: {with_ms:.2} ms, as the old boxes: \
         {boxes_ms:.2} ms, without: {without_ms:.2} ms (+{:.2} ms for the stands, \
         +{:.2} ms for the boxes)",
        with_ms - without_ms,
        boxes_ms - without_ms
    );
}

// Readback unpacking: 256-byte row padding and BGRA swap, no GPU needed.
#[test]
fn texel_rows_unpads_rows() {
    // 3x1 RGBA pixels, stride padded past 12 bytes.
    let row = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
    let stride = 16;
    let mut data = row.to_vec();
    data.resize(stride, 0);
    let out = texel_rows_to_rgba(&data, 3, 1, stride, false);
    // The colour is kept and the alpha forced opaque (vanilla `Screenshot.takeScreenshot`).
    assert_eq!(out, vec![1, 2, 3, 255, 5, 6, 7, 255, 9, 10, 11, 255]);
}

#[test]
fn texel_rows_swap_bgra() {
    // Two BGRA pixels in one padded row: r and b swap per pixel.
    let row = [10u8, 20, 30, 255, 40, 50, 60, 128];
    let stride = 16;
    let mut data = row.to_vec();
    data.resize(stride, 0);
    let out = texel_rows_to_rgba(&data, 2, 1, stride, true);
    assert_eq!(out, vec![30, 20, 10, 255, 60, 50, 40, 255]);
}

// The async capture state machine's shared half, no GPU needed: the map callback
// (a clone of the flag) advances Pending to Ready or Failed; take_capture polls.
#[test]
fn capture_flag_advances_pending_to_ready_or_failed() {
    let flag = CaptureFlag::default();
    assert_eq!(flag.get(), CapturePhase::Pending);
    flag.clone().set(CapturePhase::Ready);
    assert_eq!(flag.get(), CapturePhase::Ready);
    flag.clone().set(CapturePhase::Failed);
    assert_eq!(flag.get(), CapturePhase::Failed);
}

fn camera_with(bob: ViewBob, roll: f32) -> Camera {
    Camera {
        pos: glam::DVec3::ZERO,
        yaw: 0.0,
        pitch: 0.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: roll,
        bob,
    }
}

/// The look direction (Minecraft: yaw 0 = +z, pitch +90 = straight down) lands on view -Z
/// at every angle, including straight up and down, where a look-at would degenerate.
#[test]
fn view_rotation_looks_along_minus_z_at_every_pitch() {
    for &(yaw, pitch) in &[
        (0.0f32, 0.0f32),
        (90.0, 30.0),
        (-135.0, -60.0),
        (37.0, 90.0),
        (37.0, -90.0),
        (180.0, 89.9),
    ] {
        let mut cam = camera_with(ViewBob::default(), 0.0);
        cam.yaw = yaw;
        cam.pitch = pitch;
        let (y, p) = (yaw.to_radians(), pitch.to_radians());
        let forward = glam::Vec3::new(-p.cos() * y.sin(), -p.sin(), p.cos() * y.cos());
        let v = Renderer::view_rotation(&cam).transform_vector3(forward);
        assert!(
            (v - glam::Vec3::NEG_Z).length() < 1e-5,
            "{yaw} {pitch}: {v}"
        );
        // The camera's right never flips: at yaw 0 the player's left (+x) is view -X.
        let left = glam::Vec3::new(y.cos(), 0.0, y.sin());
        let l = Renderer::view_rotation(&cam).transform_vector3(left);
        assert!(
            (l - glam::Vec3::NEG_X).length() < 1e-5,
            "{yaw} {pitch}: {l}"
        );
    }
}

#[test]
fn view_transform_is_identity_without_effects() {
    let m = Renderer::view_transform(&camera_with(ViewBob::default(), 0.0));
    assert!(m.abs_diff_eq(glam::Mat4::IDENTITY, 1e-6));
}

#[test]
fn view_transform_bobs_in_the_camera_plane() {
    // `bobView` translates x right / y up in view space, then rolls about Z and pitches
    // about X. With only a translation the matrix is that translation.
    let bob = ViewBob {
        x: 0.25,
        y: -0.5,
        ..ViewBob::default()
    };
    let m = Renderer::view_transform(&camera_with(bob, 0.0));
    let want = glam::Mat4::from_translation(glam::Vec3::new(0.25, -0.5, 0.0));
    assert!(m.abs_diff_eq(want, 1e-6));
}

#[test]
fn view_transform_hurt_tilt_turns_the_roll_axis_by_hurt_dir() {
    // `bobHurt` rotates the z-roll axis by `-hurtDir`, rolls, then turns back. A 90
    // degree hurtDir turns the z-roll into a roll about the (rotated) axis.
    let bob = ViewBob {
        hurt_tilt_deg: 30.0,
        hurt_dir_deg: 90.0,
        ..ViewBob::default()
    };
    let m = Renderer::view_transform(&camera_with(bob, 0.0));
    let r = |axis: glam::Vec3, deg: f32| glam::Mat4::from_axis_angle(axis, deg.to_radians());
    let want = r(glam::Vec3::Y, -90.0) * r(glam::Vec3::Z, 30.0) * r(glam::Vec3::Y, 90.0);
    assert!(m.abs_diff_eq(want, 1e-6));
}

#[test]
fn a_spectator_camera_inside_a_solid_block_turns_the_cull_off() {
    // `Camera.extractRenderState`: smartCull is only disabled for a spectator camera
    // that is inside a solid-render block; everyone else keeps the occlusion walk.
    assert!(super::frame_setup::cull_sections(false, false));
    assert!(super::frame_setup::cull_sections(false, true));
    assert!(super::frame_setup::cull_sections(true, false));
    assert!(!super::frame_setup::cull_sections(true, true));
}

#[test]
fn empty_section_bits_set_clear_and_drop_their_column() {
    // The walk's see-through data for unmeshed sections (vanilla
    // `SectionOcclusionGraph.updateEmptySections`): setting reports a change once,
    // repeating does not, clearing reports one and drops the column with its last bit.
    let mut empty = crate::occlusion::EmptyColumns::new();
    let column = ChunkPos::new(3, -7);
    assert!(super::terrain_feed::set_empty_bit(
        &mut empty, column, 2, true
    ));
    assert!(!super::terrain_feed::set_empty_bit(
        &mut empty, column, 2, true
    ));
    assert!(super::terrain_feed::set_empty_bit(
        &mut empty, column, 5, true
    ));
    assert_eq!(empty.get(&column), Some(&(0b100100)));
    assert!(super::terrain_feed::set_empty_bit(
        &mut empty, column, 2, false
    ));
    assert_eq!(empty.get(&column), Some(&(1 << 5)));
    assert!(super::terrain_feed::set_empty_bit(
        &mut empty, column, 5, false
    ));
    assert!(empty.get(&column).is_none());
    assert!(!super::terrain_feed::set_empty_bit(
        &mut empty, column, 5, false
    ));
}
