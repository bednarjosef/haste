// The pure parts of the world border wall (vanilla `WorldBorderRenderer` and
// `WorldBorderRenderState`): the alpha from the camera's distance, the sides ordered by
// distance, the texture scroll and the four wall quads' vertices with vanilla's offsets.

/// The border's square in blocks (vanilla `WorldBorderRenderState` minX..maxZ).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min_x: f64,
    pub max_x: f64,
    pub min_z: f64,
    pub max_z: f64,
}

/// Vanilla `WorldBorderRenderer.rebuildWorldBorderBuffer` keeps the wall 0.01 blocks inside
/// the border so its faces never z-fight the border's own edge.
const EDGE_INSET: f64 = 0.01;

/// The forcefield texture scrolls once every 3000 ms (vanilla `prepareDynamicTransforms`).
const SCROLL_PERIOD_MS: u128 = 3000;

/// Quad indices of the wall (vanilla `Direction.get2DDataValue`): south, west, north, east.
pub const SOUTH: usize = 0;
pub const WEST: usize = 1;
pub const NORTH: usize = 2;
pub const EAST: usize = 3;

/// The wall's vertices: four quads of six vertices each (two triangles, so one draw per
/// side reads six consecutive vertices), as `[x, y, z, u, v]`.
pub type WallVertices = [[f32; 5]; 24];

/// The wall's geometry as built: the origin the vertices are relative to (vanilla's
/// `lastMinX`/`lastMinZ`, the camera-clipped minimums) and the vertices.
#[derive(Clone, Copy, Debug)]
pub struct Wall {
    pub origin_x: f64,
    pub origin_z: f64,
    pub vertices: WallVertices,
}

/// Vanilla `WorldBorderRenderState.alpha` (`extract`): zero outside the render-distance box
/// around the border, otherwise `(1 - distance / renderDistance)^4` clamped to 0..=1.
pub fn alpha(bounds: &Bounds, x: f64, z: f64, render_distance: f64) -> f32 {
    let rd = render_distance;
    let outside_box = x < bounds.min_x - rd
        || x > bounds.max_x + rd
        || z < bounds.min_z - rd
        || z > bounds.max_z + rd;
    if outside_box {
        return 0.0;
    }
    // Clamp the fraction before the power: past the render distance it is negative, and an
    // even power would bring it back up.
    let fraction = (1.0 - distance_to_border(bounds, x, z) / rd).clamp(0.0, 1.0);
    fraction.powi(4) as f32
}

/// Vanilla `WorldBorder.getDistanceToBorder`: the nearest edge's distance, negative outside.
pub fn distance_to_border(bounds: &Bounds, x: f64, z: f64) -> f64 {
    (x - bounds.min_x)
        .min(bounds.max_x - x)
        .min(z - bounds.min_z)
        .min(bounds.max_z - z)
}

/// Vanilla `WorldBorderRenderState.closestBorder`: each side's distance from the camera
/// with its quad index, nearest first.
pub fn closest_sides(bounds: &Bounds, x: f64, z: f64) -> [(f64, usize); 4] {
    let mut sides = [
        (z - bounds.min_z, NORTH),
        (bounds.max_z - z, SOUTH),
        (x - bounds.min_x, WEST),
        (bounds.max_x - x, EAST),
    ];
    sides.sort_by(|a, b| a.0.total_cmp(&b.0));
    sides
}

/// The texture scroll offset for a wall-clock time (vanilla `Util.getMillis() % 3000 / 3000`).
pub fn scroll_offset(millis: u128) -> f32 {
    (millis % SCROLL_PERIOD_MS) as f32 / SCROLL_PERIOD_MS as f32
}

/// Vanilla `WorldBorderRenderer.rebuildWorldBorderBuffer`: the four wall quads around the
/// camera, `half_height` tall, clipped to the render distance. `v0` is the vertical texture
/// coordinate of the wall's bottom (vanilla's `-frac(cameraY / 2)`).
pub fn build_wall(
    bounds: &Bounds,
    cam_x: f64,
    cam_z: f64,
    render_distance: f64,
    half_height: f32,
    v0: f32,
) -> Wall {
    let rd = render_distance;
    let border_min_x = bounds.min_x + EDGE_INSET;
    let border_max_x = bounds.max_x - EDGE_INSET;
    let border_min_z = bounds.min_z + EDGE_INSET;
    let border_max_z = bounds.max_z - EDGE_INSET;
    let min_z = (cam_z - rd).floor().max(border_min_z);
    let max_z = (cam_z + rd).ceil().min(border_max_z);
    let u0z = (min_z.floor() as i64 & 1) as f32 * 0.5;
    let u1z = (max_z - min_z) as f32 / 2.0;
    let min_x = (cam_x - rd).floor().max(border_min_x);
    let max_x = (cam_x + rd).ceil().min(border_max_x);
    let u0x = (min_x.floor() as i64 & 1) as f32 * 0.5;
    let u1x = (max_x - min_x) as f32 / 2.0;
    let v1 = v0 + half_height;
    let h = half_height;
    let wx = (max_x - min_x) as f32;
    let wz = (max_z - min_z) as f32;
    let south = (border_max_z - min_z) as f32;
    let east = (border_max_x - min_x) as f32;

    // Corners in vanilla's order (south, west, north, east), each as two triangles.
    let quads: [[[f32; 5]; 4]; 4] = [
        [
            [0.0, -h, south, u0x, v1],
            [wx, -h, south, u1x + u0x, v1],
            [wx, h, south, u1x + u0x, v0],
            [0.0, h, south, u0x, v0],
        ],
        [
            [0.0, -h, 0.0, u0z, v1],
            [0.0, -h, wz, u1z + u0z, v1],
            [0.0, h, wz, u1z + u0z, v0],
            [0.0, h, 0.0, u0z, v0],
        ],
        [
            [wx, -h, 0.0, u0x, v1],
            [0.0, -h, 0.0, u1x + u0x, v1],
            [0.0, h, 0.0, u1x + u0x, v0],
            [wx, h, 0.0, u0x, v0],
        ],
        [
            [east, -h, wz, u0z, v1],
            [east, -h, 0.0, u1z + u0z, v1],
            [east, h, 0.0, u1z + u0z, v0],
            [east, h, wz, u0z, v0],
        ],
    ];
    let mut vertices = [[0.0; 5]; 24];
    for (q, corners) in quads.iter().enumerate() {
        let order = [0, 1, 2, 2, 3, 0];
        for (i, &c) in order.iter().enumerate() {
            vertices[q * 6 + i] = corners[c];
        }
    }
    Wall {
        origin_x: min_x,
        origin_z: min_z,
        vertices,
    }
}
