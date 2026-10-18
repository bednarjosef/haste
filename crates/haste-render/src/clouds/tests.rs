use super::mesh::{build_extruded_cell, build_mesh, Cell, Dir};
use super::*;

fn solid(w: u32, h: u32) -> CloudTexture {
    // Every cell solid with no empty neighbours: sides are never drawn.
    CloudTexture {
        cells: vec![
            Cell {
                empty: false,
                north: false,
                east: false,
                south: false,
                west: false
            };
            (w * h) as usize
        ],
        width: w,
        height: h,
    }
}

fn image(px: &[[u8; 4]], w: u32, h: u32) -> image::RgbaImage {
    image::RgbaImage::from_raw(w, h, px.iter().flatten().copied().collect::<Vec<u8>>()).unwrap()
}

/// A cell is empty when its alpha is below 10, and the neighbour flags wrap around the
/// texture (vanilla `isCellEmpty` with floorMod lookups).
#[test]
fn cells_read_empty_and_wrapped_neighbours_from_the_texture() {
    // 2x2, one solid texel: its four neighbours are the empty texels, wrapped.
    let px = [
        [0, 0, 0, 0],
        [255, 255, 255, 255], //
        [0, 0, 0, 0],
        [0, 0, 0, 9],
    ];
    let tex = CloudTexture::from_image(&image(&px, 2, 2));
    assert!(tex.cell(0, 0).empty, "alpha 0 texel is empty");
    assert!(tex.cell(1, 1).empty, "alpha 9 is below the 10 threshold");
    let c = tex.cell(1, 0);
    assert!(!c.empty);
    assert!(c.north, "north of (1,0) is (1,1)");
    assert!(c.south, "south of (1,0) is (1,1) wrapped");
    assert!(c.west, "west of (1,0) is (0,0)");
    assert!(c.east, "east of (1,0) is (0,0) wrapped");
}

/// The camera's position relative to the layer (vanilla `prepare`): above when the
/// camera is past the layer's top, below when under its bottom, inside otherwise.
#[test]
fn relative_camera_position_splits_at_the_layer() {
    use RelativeCameraPos::*;
    // Layer bottom 192.33, cells 4 tall: top at 196.33.
    assert_eq!(RelativeCameraPos::of(200.0, 192.33), AboveClouds);
    assert_eq!(RelativeCameraPos::of(196.34, 192.33), AboveClouds);
    assert_eq!(RelativeCameraPos::of(196.33, 192.33), InsideClouds);
    assert_eq!(RelativeCameraPos::of(192.33, 192.33), InsideClouds);
    assert_eq!(RelativeCameraPos::of(192.32, 192.33), BelowClouds);
    assert_eq!(RelativeCameraPos::of(100.0, 192.33), BelowClouds);
}

/// Which faces an extruded cell draws (vanilla `buildExtrudedCell`): from below the layer
/// the bottoms (the camera below sees the 0.7 shade), from above the tops, from inside
/// both; the empty-neighbour sides only on the far side of the camera (north for z > 0,
/// south for z < 0, west for x > 0, east for x < 0 — exactly one of each pair at any far
/// cell); the interior faces only for the nine cells around the camera.
#[test]
fn extruded_cells_draw_the_vanilla_faces() {
    let cell_all_sides = Cell {
        empty: false,
        north: true,
        east: true,
        south: true,
        west: true,
    };
    let shades_of = |out: &Vec<CloudVertex>| {
        out.chunks(6)
            .map(|face| face[0].shade)
            .collect::<Vec<f32>>()
    };
    // Far cell at (5, 5), camera below: bottom (0.7), north (0.8, z > 0) and west
    // (0.9, x > 0) sides.
    let mut out = Vec::new();
    build_extruded_cell(
        &mut out,
        5,
        5,
        cell_all_sides,
        RelativeCameraPos::BelowClouds,
    );
    assert_eq!(shades_of(&out), vec![0.7, 0.8, 0.9]);

    // Camera above: the top (1.0) instead of the bottom.
    out.clear();
    build_extruded_cell(
        &mut out,
        5,
        5,
        cell_all_sides,
        RelativeCameraPos::AboveClouds,
    );
    assert_eq!(shades_of(&out), vec![1.0, 0.8, 0.9]);

    // Camera inside the layer: both top and bottom.
    out.clear();
    build_extruded_cell(
        &mut out,
        5,
        5,
        cell_all_sides,
        RelativeCameraPos::InsideClouds,
    );
    assert_eq!(shades_of(&out), vec![1.0, 0.7, 0.8, 0.9]);

    // At (5, -5) the sides flip to south (z < 0) and west stays (x > 0).
    out.clear();
    build_extruded_cell(
        &mut out,
        5,
        -5,
        cell_all_sides,
        RelativeCameraPos::BelowClouds,
    );
    assert_eq!(shades_of(&out), vec![0.7, 0.8, 0.9]);

    // Solid neighbours: no side faces at all.
    let solid = Cell {
        empty: false,
        north: false,
        east: false,
        south: false,
        west: false,
    };
    out.clear();
    build_extruded_cell(&mut out, 5, 5, solid, RelativeCameraPos::BelowClouds);
    assert_eq!(shades_of(&out), vec![0.7]);

    // Interior faces: the cell at the camera draws all six directions again (1 regular +
    // 6 inside = 7 faces), a cell two cells away draws none extra.
    out.clear();
    build_extruded_cell(&mut out, 0, 0, solid, RelativeCameraPos::BelowClouds);
    assert_eq!(out.len(), 7 * 6);
    out.clear();
    build_extruded_cell(&mut out, 2, 0, solid, RelativeCameraPos::BelowClouds);
    assert_eq!(out.len(), 6);
}

/// The interior faces reverse their winding (vanilla's FLAG_INSIDE_FACE): the inside face
/// visits the quad's corners in the opposite order.
#[test]
fn inside_faces_reverse_the_winding() {
    let solid = Cell {
        empty: false,
        north: false,
        east: false,
        south: false,
        west: false,
    };
    let mut out = Vec::new();
    build_extruded_cell(&mut out, 0, 0, solid, RelativeCameraPos::BelowClouds);
    // Faces in emission order: Down (regular), then the six inside faces starting with
    // Down, Up, North, South, West, East.
    let down = &out[0..6];
    let down_inside = &out[6..12];
    // The regular face's triangles are (0,1,2) and (0,2,3); the inside face's are the
    // same corners wound the other way: (3,2,1) and (3,1,0). Corners in blocks: the
    // cell size (12, 4, 12) times the unit corner.
    let corners = Dir::Down
        .corners()
        .map(|c| [c[0] * CELL_SIZE, c[1] * CELL_HEIGHT, c[2] * CELL_SIZE]);
    // Regular: c0, c1, c2, c0, c2, c3.
    for (i, c) in [0usize, 1, 2, 0, 2, 3].iter().enumerate() {
        assert_eq!(down[i].pos, corners[*c]);
    }
    // Inside: c3, c2, c1, c3, c1, c0.
    for (i, c) in [3usize, 2, 1, 3, 1, 0].iter().enumerate() {
        assert_eq!(down_inside[i].pos, corners[*c]);
    }
}

/// The mesh only covers the disc of radius cells (vanilla `buildMesh`'s x² + z² <= r²)
/// and reads cells with texture wraparound.
#[test]
fn mesh_covers_the_radius_disc_and_wraps_the_texture() {
    // 4x4 texture of solid cells, radius 2: the disc x²+z²<=4 holds 13 cells
    // ((0,0), (±1,0), (0,±1), (±2,0), (0,±2), (±1,±1)). From below, each draws its
    // bottom; the nine cells with |x|,|z| <= 1 also draw the six inside faces.
    let tex = solid(4, 4);
    let mut out = Vec::new();
    build_mesh(&mut out, &tex, 0, 0, RelativeCameraPos::BelowClouds, 2);
    assert_eq!(
        out.len(),
        (13 + 9 * 6) * 6,
        "13 cells, 9 with interior faces"
    );

    // Radius 1 around cell (100, 100): the texture wraps at 4, so the cells read
    // (100,100) == (0,0) mod 4; the disc holds 5 cells, all with interior faces.
    let mut out = Vec::new();
    build_mesh(&mut out, &tex, 100, 100, RelativeCameraPos::BelowClouds, 1);
    assert_eq!(out.len(), (5 + 5 * 6) * 6);
}

/// Vertex positions are camera-cell-relative blocks: the cell (-1, 2) spans x -12..0,
/// z 24..36, and the y spans the 4-block cell height (inside the layer, so both the top
/// and the bottom faces exist).
#[test]
fn vertex_positions_are_camera_cell_relative_blocks() {
    let solid = Cell {
        empty: false,
        north: false,
        east: false,
        south: false,
        west: false,
    };
    let mut out = Vec::new();
    build_extruded_cell(&mut out, -1, 2, solid, RelativeCameraPos::InsideClouds);
    let x = |i: usize| out.iter().map(|v| v.pos[i]).collect::<Vec<f32>>();
    assert_eq!(x(0).iter().cloned().fold(f32::MIN, f32::max), 0.0);
    assert_eq!(x(0).iter().cloned().fold(f32::MAX, f32::min), -12.0);
    assert_eq!(x(1).iter().cloned().fold(f32::MAX, f32::min), 0.0);
    assert_eq!(x(1).iter().cloned().fold(f32::MIN, f32::max), 4.0);
    assert_eq!(x(2).iter().cloned().fold(f32::MIN, f32::max), 36.0);
    assert_eq!(x(2).iter().cloned().fold(f32::MAX, f32::min), 24.0);
}

/// The cloud colour chain (`CloudRenderer.prepare`'s `color` from the `cloud_color`
/// attribute): white 0.8 by day, the night multiplier (0.1, 0.1, 0.15) at night, then the
/// weather layers (rain: blend to grey x0.24 by 0.5; thunder: grey x0.095 by 0.94).
#[test]
fn cloud_colour_follows_the_cloud_color_attribute() {
    // Day: the overworld base, white at alpha 0.8.
    let day = cloud_color(6000, 0.0, 0.0);
    assert_eq!(day, [1.0, 1.0, 1.0, 0.8]);
    // Night: the timeline's (0.1, 0.1, 0.15) multiply, alpha untouched.
    let night = cloud_color(18000, 0.0, 0.0);
    for i in 0..3 {
        assert!((night[i] - [0.1, 0.1, 0.15][i]).abs() < 1e-6, "{night:?}");
    }
    assert_eq!(night[3], 0.8);
    // Rain (level 1, no thunder): lerp(0.5, colour, grey * 0.24) with grey = 0.3 + 0.59 +
    // 0.11 = 1 for white, so the target is 0.24 everywhere: 1 + (0.24 - 1) * 0.5 = 0.62.
    let rain = cloud_color(6000, 1.0, 0.0);
    for i in 0..3 {
        assert!((rain[i] - 0.62).abs() < 1e-6, "{rain:?}");
    }
    assert_eq!(rain[3], 0.8);
    // Full thunder (rain 1, thunder 1, so the rain layer's weight is 0): lerp(0.94,
    // white, grey * 0.095) = 1 + (0.095 - 1) * 0.94 = 0.149.
    let storm = cloud_color(6000, 1.0, 1.0);
    for i in 0..3 {
        assert!(
            (storm[i] - (1.0 + (0.095 - 1.0) * 0.94)).abs() < 1e-6,
            "{storm:?}"
        );
    }
}

/// The cloud drift advances with game time (not time of day), 0.03 blocks per tick plus
/// the partial tick, and wraps at the clouds.png width in blocks (256 cells x 12).
#[test]
fn cloud_drift_uses_game_time_plus_partial() {
    assert_eq!(cloud_drift(0, 0.0), 0.0);
    assert!((cloud_drift(100, 0.5) - 3.015).abs() < 1e-4);
    // Game time, not time of day: two worlds with the same time of day but 100 ticks of
    // game time apart have their clouds 3 blocks apart.
    assert!((cloud_drift(100, 0.0) - 3.0).abs() < 1e-4);
    // The wrap: gameTime % (256 * 400) ticks == 3072 blocks of drift.
    assert!((cloud_drift(256 * 400, 0.0)).abs() < 1e-4);
    assert!((cloud_drift(256 * 400 + 100, 0.0) - 3.0).abs() < 1e-4);
}
