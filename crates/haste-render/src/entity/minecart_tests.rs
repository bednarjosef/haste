//! The minecart renderer's pure parts: the jitter seed's formula and the TNT cargo's swell,
//! flash and white-overlay numbers, against vanilla's
//! (`AbstractMinecartRenderer.submit`, `TntRenderer.getSwellAmount`/`isLit`,
//! `OverlayTexture`), and the two render paths' poses (`newRender`, `oldRender`).
use super::minecart::{cart_pose, offset_seed, tnt_lit, tnt_swell, TNT_WHITE_OVERLAY};
use glam::{DVec3, Mat4, Vec3};
use haste_core::model::{BodyTransform, EntityInstance, MinecartDecoration, PoseOverrides};

/// Vanilla `entity.getId() * 493286711L` then `seed² * 4392167121L + seed * 98761L`, with
/// Java's wrapping long arithmetic: id 0 gives 0, and the three jitter bits the renderer
/// extracts always land inside the ±0.00175 the formula's `(v + 0.5) / 8 - 0.5` times 0.004
/// can produce.
#[test]
fn offset_seed_wraps_like_java_longs() {
    assert_eq!(offset_seed(0), 0);
    // The same product chain with explicit wrapping (guards a regression to i32 math or a
    // dropped wrap, which would change the jitter entirely).
    let java = |id: i64| {
        let seed = id.wrapping_mul(493286711);
        seed.wrapping_mul(seed)
            .wrapping_mul(4392167121)
            .wrapping_add(seed.wrapping_mul(98761))
    };
    for id in [1, 7, 1000, -3, i32::MAX as i64, i32::MIN as i64] {
        assert_eq!(offset_seed(id as i32), java(id), "entity id {id}");
    }
    for id in 1..64 {
        let seed = offset_seed(id);
        for shift in [16, 20, 24] {
            let v = ((seed >> shift) & 7) as f32;
            let offset = ((v + 0.5) / 8.0 - 0.5) * 0.004;
            assert!(
                (-0.002..0.002).contains(&offset),
                "id {id} shift {shift}: {offset}"
            );
        }
    }
}

/// `TntRenderer.getSwellAmount`: `(1 - fuse/10)` clamped to 0..1, fourth power, times 0.3.
#[test]
fn tnt_swell_follows_vanillas_curve() {
    assert!(
        (tnt_swell(10.0) - 0.0).abs() < 1e-6,
        "at ten ticks: nothing"
    );
    assert!(
        (tnt_swell(5.0) - 0.018_75).abs() < 1e-6,
        "half way: 0.5^4 * 0.3"
    );
    assert!(
        (tnt_swell(-1.0) - 0.3).abs() < 1e-6,
        "the last tick: full swell"
    );
    assert!((tnt_swell(0.0) - 0.3).abs() < 1e-6, "at zero: full swell");
}

/// `TntRenderer.isLit`: on while the fuse counts and its five-tick phase is even.
#[test]
fn tnt_lit_flashes_in_five_tick_phases() {
    assert!(!tnt_lit(-1.0), "not primed");
    assert!(tnt_lit(0.0));
    assert!(tnt_lit(4.9));
    assert!(!tnt_lit(5.0), "the second phase is dark");
    assert!(!tnt_lit(9.9));
    assert!(tnt_lit(10.0), "and on again");
}

/// The white overlay's strength is vanilla's `OverlayTexture` white row at full progress:
/// alpha `(1 - 15/15 * 0.75) * 255 = 63` of 255.
#[test]
fn tnt_white_overlay_is_the_overlay_textures_alpha() {
    assert!((TNT_WHITE_OVERLAY - 63.0 / 255.0).abs() < 1e-6);
}

/// A cart instance with the given body ops and yaw, and its decoration with the given rail
/// positions.
fn cart(
    body: BodyTransform,
    yaw: f32,
    rail: Option<(DVec3, DVec3, DVec3)>,
) -> (EntityInstance, MinecartDecoration) {
    let decoration = MinecartDecoration {
        display: None,
        display_offset: 6,
        hurt_time: 0.0,
        hurt_dir: 1.0,
        damage_time: 0.0,
        fuse: -1.0,
        rail,
    };
    let instance = EntityInstance {
        id: haste_core::EntityId(1),
        model: haste_entity::minecart::model(),
        pos: DVec3::new(0.5, 64.1, 0.5),
        yaw,
        pitch: 0.0,
        head_yaw: 0.0,
        scale: 1.0,
        pose: PoseOverrides::default(),
        body,
        body_scale: Vec3::ONE,
        skin: None,
        nametag: None,
        below_name: None,
        shadow_radius: 0.7,
        shadow_strength: 1.0,
        invisible: false,
        display_fire: false,
        width: 0.98,
        height: 0.7,
        is_discrete: false,
        name_tag_y: 0.7,
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
    };
    (instance, decoration)
}

fn assert_mat_close(got: Mat4, want: Mat4) {
    assert!(got.abs_diff_eq(want, 1e-5), "{got:?} is not {want:?}");
}

/// `newRender`: `rotateDegrees(YP, yRot)`, `rotateDegrees(ZP, -xRot)`, then
/// `translate(0, 0.375, 0)` in that turned frame (so a pitched cart's lift tilts with it).
#[test]
fn a_new_behaviour_cart_is_posed_by_new_render() {
    let (e, c) = cart(
        haste_entity::new_minecart_behavior::new_render(90.0, 30.0),
        0.0,
        None,
    );
    let want = Mat4::from_rotation_y(90f32.to_radians())
        * Mat4::from_rotation_z((-30f32).to_radians())
        * Mat4::from_translation(Vec3::new(0.0, 0.375, 0.0));
    assert_mat_close(cart_pose(&e, &c), want);
    // The lift is tilted by the pitch: not straight up.
    let lift = cart_pose(&e, &c).transform_point3(Vec3::ZERO);
    assert!((lift.y - 0.375 * 30f32.to_radians().cos()).abs() < 1e-5);
}

/// `oldRender` off the rails: `translate(0, 0.375, 0)` first, then `180 - yRot` and the
/// pitch; a cart without body ops keeps this path.
#[test]
fn an_old_behaviour_cart_keeps_old_render() {
    let (e, c) = cart(BodyTransform::default(), 30.0, None);
    let want = Mat4::from_translation(Vec3::new(0.0, 0.375, 0.0))
        * Mat4::from_rotation_y(150f32.to_radians())
        * Mat4::from_rotation_z(0.0);
    assert_mat_close(cart_pose(&e, &c), want);
}
