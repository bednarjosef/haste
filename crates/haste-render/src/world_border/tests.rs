use super::geometry::{
    alpha, build_wall, closest_sides, distance_to_border, scroll_offset, Bounds, EAST, NORTH,
    SOUTH, WEST,
};

/// The uniform block must match `world_border.wgsl`'s `BorderUniforms`, which WGSL rounds to
/// 128 bytes (a 16-byte-aligned trailing `vec3`); wgpu rejects a binding of any other size.
#[test]
fn the_uniforms_match_the_wgsl_block_size() {
    assert_eq!(std::mem::size_of::<super::BorderUniforms>(), 128);
}

/// A 20-block square centred on the origin: x and z in [-10, 10].
fn square() -> Bounds {
    Bounds {
        min_x: -10.0,
        max_x: 10.0,
        min_z: -10.0,
        max_z: 10.0,
    }
}

/// Vanilla `getDistanceToBorder`: the nearest edge, negative outside.
#[test]
fn distance_is_to_the_nearest_edge() {
    let b = square();
    assert_eq!(distance_to_border(&b, 0.0, 0.0), 10.0);
    assert_eq!(distance_to_border(&b, 7.0, 0.0), 3.0);
    assert_eq!(distance_to_border(&b, 0.0, -9.0), 1.0);
    assert_eq!(distance_to_border(&b, 11.0, 0.0), -1.0);
}

/// `extract`'s alpha: (1 - d / renderDistance)^4, so it fades to nothing at the render
/// distance and is full on the wall itself (distance 0).
#[test]
fn alpha_is_the_fourth_power_of_the_remaining_fraction() {
    let b = square();
    // Distance 6 out of a render distance of 32.
    let a = alpha(&b, 4.0, 0.0, 32.0);
    assert!((a - (1.0f32 - 6.0 / 32.0).powi(4)).abs() < 1e-6, "{a}");
    assert_eq!(alpha(&b, 10.0, 0.0, 10.0), 1.0);
    assert_eq!(alpha(&b, 0.0, 0.0, 10.0), 0.0);
}

/// Outside the render-distance box, or farther than the render distance from the wall,
/// nothing is drawn.
#[test]
fn alpha_is_zero_far_from_the_border() {
    let b = square();
    assert_eq!(alpha(&b, 100.0, 0.0, 32.0), 0.0);
    assert_eq!(alpha(&b, 0.0, 0.0, 4.0), 0.0);
}

/// `closestBorder`: the four sides ordered nearest first, with their quad indices
/// (vanilla `Direction.get2DDataValue`: south 0, west 1, north 2, east 3).
#[test]
fn closest_sides_are_ordered_nearest_first() {
    let b = square();
    let sides = closest_sides(&b, 7.0, 0.0);
    assert_eq!(sides[0], (3.0, EAST));
    assert_eq!(sides[3], (17.0, WEST));
    // North and south are both 10 blocks away; the two sit between them in some order.
    assert!(sides[1..3].contains(&(10.0, NORTH)));
    assert!(sides[1..3].contains(&(10.0, SOUTH)));
}

/// The forcefield scrolls once every 3000 ms (vanilla `Util.getMillis() % 3000 / 3000`).
#[test]
fn the_texture_scrolls_every_three_seconds() {
    assert_eq!(scroll_offset(0), 0.0);
    assert_eq!(scroll_offset(1500), 0.5);
    assert_eq!(scroll_offset(3000), 0.0);
    assert_eq!(scroll_offset(4500), 0.5);
}

/// `rebuildWorldBorderBuffer`: the wall's origin is the render-distance-clipped minimum
/// (vanilla `lastMinX`/`lastMinZ`), and the south face sits at the inset border's far edge.
#[test]
fn the_wall_is_clipped_to_the_render_distance() {
    let b = square();
    let wall = build_wall(&b, 0.0, 0.0, 4.0, 4096.0, 0.0);
    assert_eq!(wall.origin_x, -4.0);
    assert_eq!(wall.origin_z, -4.0);
    // The south quad's corner 1: x = the clipped width (8), z = the inset south edge
    // (9.99) relative to the origin.
    let corner = wall.vertices[1];
    assert!((corner[0] - 8.0).abs() < 1e-5, "{corner:?}");
    assert!((corner[2] - 13.99).abs() < 1e-4, "{corner:?}");
    assert_eq!(corner[1], -4096.0);
}

/// The wall's u parity: a clipped minimum on an odd block starts half a texture over, and
/// the u span is half the width (vanilla `u0x`, `u1x`).
#[test]
fn the_wall_uvs_follow_vanillas_parity() {
    let b = square();
    // The clipped minimum is -4 (even): no offset; the u span is 8 / 2 = 4.
    let even = build_wall(&b, 0.0, 0.0, 4.0, 1.0, 0.0);
    assert_eq!(even.vertices[0][3], 0.0);
    assert_eq!(even.vertices[1][3], 4.0);
    // The clipped minimum is -3 (odd): half a texture over.
    let odd = build_wall(&b, 1.0, 0.0, 4.0, 1.0, 0.0);
    assert_eq!(odd.origin_x, -3.0);
    assert_eq!(odd.vertices[0][3], 0.5);
}
