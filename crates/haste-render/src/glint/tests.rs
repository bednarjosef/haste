//! The glint texturing against vanilla's `TextureTransform.setupGlintTexturing`, and the
//! glint mode an item quad gets.

use super::*;

/// Vanilla's offsets by hand: `millis = (long)(getMillis() * glintSpeed * 8.0)`, then
/// `millis % 110000 / 110000f` and `millis % 30000 / 30000f`.
#[test]
fn layer_offsets_follow_vanilla_s_clock() {
    assert_eq!(glint_layer_offsets(0, 0.5), [0.0, 0.0]);
    // 1 s at the default speed: 4000 glint milliseconds.
    assert_eq!(
        glint_layer_offsets(1_000, 0.5),
        [4_000.0 / 110_000.0, 4_000.0 / 30_000.0]
    );
    // 50 s: 200000 -> 90000 into the long cycle, 20000 into the short one.
    assert_eq!(
        glint_layer_offsets(50_000, 0.5),
        [90_000.0 / 110_000.0, 20_000.0 / 30_000.0]
    );
    // Full speed wraps the long cycle after 13.75 s.
    assert_eq!(
        glint_layer_offsets(13_751, 1.0),
        [8.0 / 110_000.0, 20_008.0 / 30_000.0]
    );
    // The clock truncates to whole milliseconds (`(long)`), and speed 0 stops it.
    assert_eq!(glint_layer_offsets(1, 0.1), [0.0, 0.0]);
    assert_eq!(glint_layer_offsets(123_456, 0.0), [0.0, 0.0]);
}

/// The vanilla matrix applied to (u, v, 0, 1) by hand: translation(-o0, o1) after
/// rotateZ(pi/18) after scale(s).
#[test]
fn texture_matrix_is_translate_rotate_scale() {
    let o = [0.25, 0.75];
    let m = glint_texture_matrix(8.0, o);
    let (u, v) = (0.01f32, 0.02f32);
    let p = m * glam::Vec4::new(u, v, 0.0, 1.0);
    let (n, c) = (std::f32::consts::PI / 18.0).sin_cos();
    let want = [
        c * 8.0 * u - n * 8.0 * v - 0.25,
        n * 8.0 * u + c * 8.0 * v + 0.75,
    ];
    assert!((p.x - want[0]).abs() < 1e-6 && (p.y - want[1]).abs() < 1e-6);
    assert_eq!(p.z, 0.0);
}

/// The shader's coordinates (mirrored by `glint_coords`) equal vanilla's matrix applied to
/// vanilla's UV0 at several times, for each texturing.
#[test]
fn shader_coordinates_match_vanilla_s_matrix() {
    for millis in [0u64, 1_234, 59_999, 3_600_000] {
        let params = GlintParams::at(millis, DEFAULT_GLINT_SPEED, DEFAULT_GLINT_STRENGTH);
        let o = glint_layer_offsets(millis, DEFAULT_GLINT_SPEED);
        // An item sprite at (300, 40) in our atlas, 16x16; texel (305.5, 47) is local
        // (5.5, 7), vanilla's UV0 relative to the sprite origin in the 1024x512 items atlas.
        let rect = [300, 40, 16, 16];
        let uv = [305.5, 47.0];
        let cases = [
            (GlintMode::ItemsSheet, 8.0, [5.5 / 1024.0, 7.0 / 512.0]),
            (GlintMode::BlocksSheet, 8.0, [5.5 / 2048.0, 7.0 / 2048.0]),
            (GlintMode::Entity, 0.5, [5.5 / 16.0, 7.0 / 16.0]),
            (GlintMode::Armor, 0.16, [5.5 / 16.0, 7.0 / 16.0]),
        ];
        for (mode, scale, uv0) in cases {
            let want = glint_texture_matrix(scale, o) * glam::Vec4::new(uv0[0], uv0[1], 0.0, 1.0);
            let got = glint_coords(mode, uv, rect, &params);
            assert!(
                (got[0] - want.x).abs() < 1e-5 && (got[1] - want.y).abs() < 1e-5,
                "{mode:?} at {millis}: {got:?} vs {want:?}"
            );
        }
    }
}

#[test]
fn glint_strength_is_the_glint_alpha() {
    assert_eq!(GlintParams::at(0, 0.5, 0.75).alpha, 0.75);
    assert_eq!(GlintParams::at(0, 0.5, 0.0).alpha, 0.0);
}

#[test]
fn item_quads_pick_their_sheet_by_sprite() {
    assert_eq!(
        item_glint_mode(FoilType::Standard, "minecraft:item/diamond_sword"),
        GlintMode::ItemsSheet
    );
    assert_eq!(
        item_glint_mode(FoilType::Standard, "minecraft:trims/items/helmet_trim_gold"),
        GlintMode::ItemsSheet
    );
    assert_eq!(
        item_glint_mode(FoilType::Standard, "minecraft:block/stone"),
        GlintMode::BlocksSheet
    );
    assert_eq!(
        item_glint_mode(FoilType::Special, "item/compass_00"),
        GlintMode::ItemsSheet
    );
    assert_eq!(
        item_glint_mode(FoilType::None, "minecraft:item/diamond_sword"),
        GlintMode::None
    );
}

#[test]
fn mark_sets_the_mode_on_the_range() {
    let v = EntityVertex {
        pos: [0.0; 3],
        uv: [0.0; 2],
        color: [1.0; 4],
        rect: [0; 4],
        light: [15, 15, 0, 0],
    };
    let mut vs = [v; 4];
    mark(&mut vs[1..3], GlintMode::Armor);
    let modes: Vec<u16> = vs.iter().map(|v| v.light[2]).collect();
    assert_eq!(modes, [0, 4, 4, 0]);
    assert_eq!(vs[1].light[..2], [15, 15]);
}
