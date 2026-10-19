//! The fire overlay's geometry, pinned against vanilla `FlameFeatureRenderer.prepare`
//! (26.3): the layer count over the bounding box, each layer's radius, y span and z
//! offset, which of `fire_0`/`fire_1` the layer draws, and the per-pair u swap.
use super::flame::emit_geometry;
use super::*;

/// One layer's quad, straight from the vertex buffer (vertices 6k..6k+6, order
/// 0,1,2,0,2,3): the left-bottom corner vertex is index 0.
fn corner(out: &[EntityVertex], layer: usize) -> (f32, f32, f32, [u16; 4], [f32; 2]) {
    let v = &out[layer * 6];
    (v.pos[0], v.pos[1], v.pos[2], v.rect, v.uv)
}

#[test]
fn layers_follow_vanillas_flame_loop() {
    // A player-sized box (0.6 x 1.8): s = 0.84, h = 1.8/0.84 = 2.14, int_h = 2 so the
    // z offset starts at 0.3 - 0.04; five layers, one per 0.45 of height.
    let mut out = Vec::new();
    emit_geometry(
        &mut out,
        Vec3::ZERO,
        0.6,
        1.8,
        glam::Quat::IDENTITY,
        (15, 15),
        [0, 0, 16, 16],
        [100, 0, 16, 16],
    );
    assert_eq!(
        out.len() / 6,
        5,
        "1.8 / 0.84 tall in 0.45 steps is 5 layers"
    );
    let s = 0.6_f32 * 1.4;
    for ss in 0..5 {
        let (x, y, z, rect, uv) = corner(&out, ss);
        let r = 0.5_f32 * 0.9_f32.powi(ss as i32);
        assert!((x - s * -r).abs() < 1e-4, "layer {ss} radius");
        assert!(
            (y - s * 0.45 * ss as f32).abs() < 1e-4,
            "layer {ss} y start"
        );
        assert!(
            (z - s * (0.3 - 2.0 * 0.02 - 0.03 * ss as f32)).abs() < 1e-4,
            "layer {ss} z offset"
        );
        // `ss % 2 == 0 ? fire1 : fire2`: the rect names which sprite the layer drew.
        let want = if ss % 2 == 0 { 0 } else { 100 };
        assert_eq!(rect[0], want, "layer {ss} sprite");
        // The u swap (`ss / 2 % 2 == 0`): layers 0-1 and 4 swapped, 2-3 not. The left
        // corner's u is the sprite's left edge when swapped, its right edge when not
        // (atlas coordinates: the rect's base plus 0 or the full width).
        let swapped = ss / 2 % 2 == 0;
        let want_u = want as f32 + if swapped { 0.0 } else { 16.0 };
        assert!((uv[0] - want_u).abs() < 1e-3, "layer {ss} u swap");
        // v spans the whole (first-frame) sprite: bottom corner at v = 1.
        assert!((uv[1] - 16.0).abs() < 1e-3, "layer {ss} v span");
    }
    // The top of the last layer: 4 * 0.45 + 1.4 = 3.2 up, scaled.
    let top = out[4 * 6 + 2].pos[1];
    assert!((top - 0.6_f32 * 1.4 * (4.0 * 0.45 + 1.4)).abs() < 1e-4);
}

#[test]
fn small_fireball_two_layers() {
    // The small fireball's 0.3125 x 0.3125 box: h = 0.3125/0.4375 = 0.71, int_h = 0 so the
    // z offset starts at the plain 0.3; two layers.
    let mut out = Vec::new();
    emit_geometry(
        &mut out,
        Vec3::ZERO,
        0.3125,
        0.3125,
        glam::Quat::IDENTITY,
        (15, 15),
        [0, 0, 16, 16],
        [100, 0, 16, 16],
    );
    assert_eq!(out.len() / 6, 2);
    let s = 0.3125_f32 * 1.4;
    let (x, y, z, rect, _) = corner(&out, 0);
    assert!((x - s * -0.5).abs() < 1e-4);
    assert!((y - 0.0).abs() < 1e-4);
    assert!(
        (z - s * 0.3).abs() < 1e-4,
        "int_h = 0 keeps the plain 0.3 z offset"
    );
    assert_eq!(rect[0], 0);
    // The second layer steps 0.45 up and 0.03 back.
    let (_, y1, z1, rect1, _) = corner(&out, 1);
    assert!((y1 - s * 0.45).abs() < 1e-4);
    assert!((z1 - s * (0.3 - 0.03)).abs() < 1e-4);
    assert_eq!(rect1[0], 100, "the second layer draws fire_1");
}

/// A zero-size box draws nothing (vanilla's `h > 0` loop).
#[test]
fn empty_box_draws_no_fire() {
    let mut out = Vec::new();
    emit_geometry(
        &mut out,
        Vec3::ZERO,
        0.0,
        0.0,
        glam::Quat::IDENTITY,
        (15, 15),
        [0, 0, 16, 16],
        [100, 0, 16, 16],
    );
    assert!(out.is_empty());
}
