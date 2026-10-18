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
