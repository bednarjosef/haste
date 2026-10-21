//! The totem pop's curve (`ScreenEffectRenderer.renderItemActivationAnimation`) at several
//! points of the countdown, and the `ITEMS_3D` shade frame.

use glam::{Mat3, Mat4, Vec3};

use super::item_activation::*;
use super::vertex::entity_shade;

fn activation(ticks: i32, off_x: f32, off_y: f32) -> ItemActivation {
    ItemActivation {
        item: haste_core::RegistryIndex(0),
        ticks,
        off_x,
        off_y,
    }
}

/// Where the pose puts the item's origin.
fn origin(ticks: i32, partial: f32, off: (f32, f32), aspect: f32) -> Vec3 {
    item_activation_pose(&activation(ticks, off.0, off.1), partial, aspect)
        .transform_point3(Vec3::ZERO)
}

fn close(a: Vec3, b: Vec3) -> bool {
    a.abs_diff_eq(b, 1e-3)
}

#[test]
fn starts_far_ahead_in_the_middle() {
    // 40 ticks left: progress 0, so no drift and the item sits 10 blocks ahead.
    let p = origin(40, 0.0, (1.0, -1.0), 1.0);
    assert!(close(p, Vec3::new(0.0, 0.0, -10.0)), "{p}");
}

#[test]
fn a_quarter_in_it_has_flown_close_and_drifts() {
    // 30 ticks left: progress 0.25, the polynomial gives 0.448486, so z = -10 + 9 sin(pi *
    // 0.448486) and the drift is 0.3 |sin(2 pi * 0.448486)| times the offsets.
    let p = origin(30, 0.0, (1.0, -1.0), 2.0);
    let s = 0.448_486_3 * std::f32::consts::PI;
    let drift = 0.3 * (2.0 * s).sin().abs();
    let want = Vec3::new(drift * 2.0, -drift, -10.0 + 9.0 * s.sin());
    assert!(close(p, want), "{p} vs {want}");
    assert!((p.z - -1.1176).abs() < 1e-3, "{p}");
}

#[test]
fn halfway_it_is_nearest_the_camera() {
    // 20 ticks left: the polynomial is 0.498438, nearly a half turn of the sine.
    let p = origin(20, 0.0, (0.0, 0.0), 1.0);
    assert!((p.z - -1.0001).abs() < 1e-3, "{p}");
}

#[test]
fn it_ends_back_in_the_distance() {
    // The last tick at partial 1: progress 1, the polynomial is exactly 1 and sin(pi) is 0.
    let p = origin(1, 1.0, (1.0, 1.0), 1.0);
    assert!(close(p, Vec3::new(0.0, 0.0, -10.0)), "{p}");
}

#[test]
fn the_partial_tick_moves_between_ticks() {
    let at_30 = origin(30, 0.0, (0.0, 0.0), 1.0).z;
    let at_29 = origin(29, 0.0, (0.0, 0.0), 1.0).z;
    let half = origin(30, 0.5, (0.0, 0.0), 1.0).z;
    assert!(half > at_30.min(at_29) && half < at_30.max(at_29), "{half}");
}

#[test]
fn the_vertical_drift_ignores_the_aspect_ratio() {
    let square = origin(30, 0.0, (0.0, 1.0), 1.0).y;
    let wide = origin(30, 0.0, (0.0, 1.0), 2.0).y;
    assert!(
        square.abs() > 0.05 && (wide - square).abs() < 1e-5,
        "{wide} vs {square}"
    );
}

#[test]
fn it_is_scaled_and_spun() {
    // Halfway: scale 0.8 and a spin of 900 |sin| degrees about Y; the wobble's
    // 6 cos(4) degrees tilts it a little about X and Z.
    let m = item_activation_pose(&activation(20, 0.0, 0.0), 0.0, 1.0);
    let x = Mat3::from_mat4(m) * Vec3::X;
    assert!((x.length() - 0.8).abs() < 1e-4, "{x}");
    let s = (0.498_437_5 * std::f32::consts::PI).sin();
    let wobble = (6.0 * 4.0f32.cos()).to_radians();
    let want = Mat4::from_scale(Vec3::splat(0.8))
        * Mat4::from_rotation_y((900.0 * s).to_radians())
        * Mat4::from_rotation_x(wobble)
        * Mat4::from_rotation_z(wobble);
    let got = Mat3::from_mat4(m);
    assert!(
        got.abs_diff_eq(Mat3::from_mat4(want), 1e-4),
        "{got} vs {want}"
    );
}

/// `Lighting.Entry.ITEMS_3D`'s light directions, built as vanilla's `Lighting` does.
fn items_3d_lights() -> [Vec3; 2] {
    let pose = Mat3::from_diagonal(Vec3::new(1.0, -1.0, 1.0))
        * Mat3::from_rotation_y(1.082_104_1)
        * Mat3::from_rotation_x(3.237_585_8)
        * Mat3::from_rotation_y(-std::f32::consts::FRAC_PI_8)
        * Mat3::from_rotation_x(std::f32::consts::FRAC_PI_4 * 3.0);
    [
        pose * Vec3::new(0.2, 1.0, -0.7).normalize(),
        pose * Vec3::new(-0.2, 1.0, 0.7).normalize(),
    ]
}

#[test]
fn the_shade_frame_gives_the_items_3d_shade() {
    let q = items_3d_shade_frame();
    assert!(
        (q.determinant() - 1.0).abs() < 1e-4,
        "a rotation keeps the winding"
    );
    let [l0, l1] = items_3d_lights();
    for n in [
        Vec3::X,
        Vec3::Y,
        Vec3::Z,
        -Vec3::X,
        -Vec3::Y,
        -Vec3::Z,
        Vec3::new(0.3, -0.5, 0.8).normalize(),
    ] {
        let vanilla = ((n.dot(l0).max(0.0) + n.dot(l1).max(0.0)) * 0.6 + 0.4).min(1.0);
        let ours = entity_shade(q * n);
        assert!((ours - vanilla).abs() < 1e-4, "{n}: {ours} vs {vanilla}");
    }
}
