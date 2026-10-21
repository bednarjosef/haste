// CPU tests of the cloud face records; the GPU tests (draw gating, the cloud range
// option, the screenshot equality with the expanded mesh) live in `tests_gpu.rs`.

use super::mesh::{build_extruded_cell, build_faces, Cell, Dir};
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

/// One decoded quad vertex — the shape the old CPU-expanded mesh carried (and what the
/// GPU's `clouds.wgsl` `vs_main` computes): the camera-cell-relative position in blocks
/// and the face shade.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct DecodedVertex {
    pub pos: [f32; 3],
    pub shade: f32,
}

/// The unit corners of a face (vanilla `clouds.vsh`'s vertex table), one row per
/// direction in `Direction.get3DDataValue` order.
pub(super) fn corners(dir: Dir) -> [[f32; 3]; 4] {
    match dir {
        Dir::Down => [
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 0.0],
        ],
        Dir::Up => [
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 0.0],
        ],
        Dir::North => [
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
        ],
        Dir::South => [
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
            [0.0, 0.0, 1.0],
        ],
        Dir::West => [
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 1.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0],
        ],
        Dir::East => [
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 1.0, 1.0],
            [1.0, 0.0, 1.0],
        ],
    }
}

/// The face shade vanilla's `clouds.vsh` gives the direction (its `faceColors`: bottom
/// 0.7, top 1.0, north/south 0.8, west/east 0.9).
pub(super) fn shade(dir: Dir) -> f32 {
    match dir {
        Dir::Down => 0.7,
        Dir::Up => 1.0,
        Dir::North | Dir::South => 0.8,
        Dir::West | Dir::East => 0.9,
    }
}

fn sign(byte: u32) -> i32 {
    let v = (byte & 0xFF) as i32;
    if v < 128 {
        v
    } else {
        v - 256
    }
}

/// `clouds.wgsl`'s `vs_main` on the CPU: expands each face record (vanilla
/// `encodeFace`'s three bytes) into the quad's six vertices, so the record data can be
/// checked against vanilla's face tables — and rendered as the reference mesh by
/// `tests_gpu.rs`.
pub(super) fn decode_faces(records: &[u32]) -> Vec<DecodedVertex> {
    let mut out = Vec::with_capacity(records.len() * 6);
    for &record in records {
        let dir_and_flags = (record >> 16) & 0xFF;
        let dir = Dir::from_index(dir_and_flags);
        let inside = dir_and_flags & 16 != 0;
        let top_color = dir_and_flags & 32 != 0;
        let cell_x = sign(record) * 2 + (dir_and_flags >> 7 & 1) as i32;
        let cell_z = sign(record >> 8) * 2 + (dir_and_flags >> 6 & 1) as i32;
        // The two triangles of the quad: (0, 1, 2) and (0, 2, 3); inside faces visit
        // the corners in the opposite order.
        let order: [usize; 6] = if inside {
            [3, 2, 1, 3, 1, 0]
        } else {
            [0, 1, 2, 0, 2, 3]
        };
        let cs = corners(dir);
        for i in order {
            let c = cs[i];
            out.push(DecodedVertex {
                pos: [
                    (cell_x as f32 + c[0]) * CELL_SIZE,
                    c[1] * CELL_HEIGHT,
                    (cell_z as f32 + c[2]) * CELL_SIZE,
                ],
                shade: if top_color { 1.0 } else { shade(dir) },
            });
        }
    }
    out
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

/// The cloud range option's radius in cells (vanilla `CloudRenderer.prepare`:
/// `ceil(range * 16 / 12)`), for the smallest, a middle and the default range.
#[test]
fn radius_comes_from_the_cloud_range_option() {
    assert_eq!(radius_cells(2), 3, "2 chunks = 32 blocks = 2.67 cells");
    assert_eq!(
        radius_cells(12),
        16,
        "12 chunks = 192 blocks = 16 cells exactly"
    );
    assert_eq!(
        radius_cells(128),
        171,
        "the default 128 chunks = 2048 blocks"
    );
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
    let shades_of = |records: &Vec<u32>| {
        decode_faces(records)
            .chunks(6)
            .map(|face| face[0].shade)
            .collect::<Vec<f32>>()
    };
    // Far cell at (5, 5), camera below: bottom (0.7), north (0.8, z > 0) and west
    // (0.9, x > 0) sides.
    let mut records = Vec::new();
    build_extruded_cell(
        &mut records,
        5,
        5,
        cell_all_sides,
        RelativeCameraPos::BelowClouds,
    );
    assert_eq!(shades_of(&records), vec![0.7, 0.8, 0.9]);

    // Camera above: the top (1.0) instead of the bottom.
    records.clear();
    build_extruded_cell(
        &mut records,
        5,
        5,
        cell_all_sides,
        RelativeCameraPos::AboveClouds,
    );
    assert_eq!(shades_of(&records), vec![1.0, 0.8, 0.9]);

    // Camera inside the layer: both top and bottom.
    records.clear();
    build_extruded_cell(
        &mut records,
        5,
        5,
        cell_all_sides,
        RelativeCameraPos::InsideClouds,
    );
    assert_eq!(shades_of(&records), vec![1.0, 0.7, 0.8, 0.9]);

    // At (5, -5) the sides flip to south (z < 0) and west stays (x > 0).
    records.clear();
    build_extruded_cell(
        &mut records,
        5,
        -5,
        cell_all_sides,
        RelativeCameraPos::BelowClouds,
    );
    assert_eq!(shades_of(&records), vec![0.7, 0.8, 0.9]);

    // Solid neighbours: no side faces at all.
    let solid = Cell {
        empty: false,
        north: false,
        east: false,
        south: false,
        west: false,
    };
    records.clear();
    build_extruded_cell(&mut records, 5, 5, solid, RelativeCameraPos::BelowClouds);
    assert_eq!(shades_of(&records), vec![0.7]);

    // Interior faces: the cell at the camera draws all six directions again (1 regular +
    // 6 inside = 7 faces), a cell two cells away draws none extra.
    records.clear();
    build_extruded_cell(&mut records, 0, 0, solid, RelativeCameraPos::BelowClouds);
    assert_eq!(records.len(), 7);
    records.clear();
    build_extruded_cell(&mut records, 2, 0, solid, RelativeCameraPos::BelowClouds);
    assert_eq!(records.len(), 1);
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
    let mut records = Vec::new();
    build_extruded_cell(&mut records, 0, 0, solid, RelativeCameraPos::BelowClouds);
    let out = decode_faces(&records);
    // Faces in emission order: Down (regular), then the six inside faces starting with
    // Down, Up, North, South, West, East.
    let down = &out[0..6];
    let down_inside = &out[6..12];
    // The regular face's triangles are (0,1,2) and (0,2,3); the inside face's are the
    // same corners wound the other way: (3,2,1) and (3,1,0). Corners in blocks: the
    // cell size (12, 4, 12) times the unit corner.
    let corners =
        corners(Dir::Down).map(|c| [c[0] * CELL_SIZE, c[1] * CELL_HEIGHT, c[2] * CELL_SIZE]);
    // Regular: c0, c1, c2, c0, c2, c3.
    for (i, c) in [0usize, 1, 2, 0, 2, 3].iter().enumerate() {
        assert_eq!(down[i].pos, corners[*c]);
    }
    // Inside: c3, c2, c1, c3, c1, c0.
    for (i, c) in [3usize, 2, 1, 3, 1, 0].iter().enumerate() {
        assert_eq!(down_inside[i].pos, corners[*c]);
    }
}

/// The records only cover the disc of radius cells (vanilla `buildMesh`'s x² + z² <= r²)
/// and read cells with texture wraparound.
#[test]
fn records_cover_the_radius_disc_and_wrap_the_texture() {
    // 4x4 texture of solid cells, radius 2: the disc x²+z²<=4 holds 13 cells
    // ((0,0), (±1,0), (0,±1), (±2,0), (0,±2), (±1,±1)). From below, each draws its
    // bottom; the nine cells with |x|,|z| <= 1 also draw the six inside faces.
    let tex = solid(4, 4);
    let mut records = Vec::new();
    build_faces(
        &mut records,
        &tex,
        0,
        0,
        RelativeCameraPos::BelowClouds,
        2,
        true,
    );
    assert_eq!(records.len(), 13 + 9 * 6, "13 cells, 9 with interior faces");
    assert_eq!(decode_faces(&records).len(), (13 + 9 * 6) * 6);

    // Radius 1 around cell (100, 100): the texture wraps at 4, so the cells read
    // (100,100) == (0,0) mod 4; the disc holds 5 cells, all with interior faces.
    let mut records = Vec::new();
    build_faces(
        &mut records,
        &tex,
        100,
        100,
        RelativeCameraPos::BelowClouds,
        1,
        true,
    );
    assert_eq!(records.len(), 5 + 5 * 6);
}

/// The decoded positions are camera-cell-relative blocks: the cell (-1, 2) spans x -12..0,
/// z 24..36, and the y spans the 4-block cell height (inside the layer, so both the top
/// and the bottom faces exist). The cell's low bit travels in the flags byte.
#[test]
fn decoded_positions_are_camera_cell_relative_blocks() {
    let solid = Cell {
        empty: false,
        north: false,
        east: false,
        south: false,
        west: false,
    };
    let mut records = Vec::new();
    build_extruded_cell(&mut records, -1, 2, solid, RelativeCameraPos::InsideClouds);
    let out = decode_faces(&records);
    let x = |i: usize| out.iter().map(|v| v.pos[i]).collect::<Vec<f32>>();
    assert_eq!(x(0).iter().cloned().fold(f32::MIN, f32::max), 0.0);
    assert_eq!(x(0).iter().cloned().fold(f32::MAX, f32::min), -12.0);
    assert_eq!(x(1).iter().cloned().fold(f32::MAX, f32::min), 0.0);
    assert_eq!(x(1).iter().cloned().fold(f32::MIN, f32::max), 4.0);
    assert_eq!(x(2).iter().cloned().fold(f32::MIN, f32::max), 36.0);
    assert_eq!(x(2).iter().cloned().fold(f32::MAX, f32::min), 24.0);
    // A cell at the default radius' edge still packs and decodes: (-171, 171) halves to
    // the signed bytes -86/86 with the low bits in the flags byte.
    let mut far = Vec::new();
    build_extruded_cell(&mut far, -171, 171, solid, RelativeCameraPos::BelowClouds);
    let out = decode_faces(&far);
    let xs = out.iter().map(|v| v.pos[0]).collect::<Vec<f32>>();
    assert_eq!(
        xs.iter().cloned().fold(f32::MAX, f32::min),
        -171.0 * CELL_SIZE
    );
    assert_eq!(
        xs.iter().cloned().fold(f32::MIN, f32::max),
        -170.0 * CELL_SIZE
    );
    assert!(out.iter().all(|v| v.pos[2] >= 171.0 * CELL_SIZE));
    assert!(out.iter().all(|v| v.pos[2] <= 172.0 * CELL_SIZE));
}

/// The flat cells of the FAST mode (vanilla `CloudRenderer.buildFlatCell`): one down face
/// per non-empty cell of the same disc as the fancy records, at the layer's bottom (y 0
/// in cell units), shaded with the top colour (vanilla's `FLAG_USE_TOP_COLOR` in
/// `clouds.vsh`, `faceColors[1]`), regular winding, and the camera's position relative
/// to the layer changes nothing (the flat cells do not branch on it).
#[test]
fn flat_cells_draw_one_top_shaded_down_face() {
    // 4x4 texture of solid cells, radius 2: the disc holds 13 cells, one quad each.
    let tex = solid(4, 4);
    let mut records = Vec::new();
    build_faces(
        &mut records,
        &tex,
        0,
        0,
        RelativeCameraPos::AboveClouds,
        2,
        false,
    );
    assert_eq!(records.len(), 13);
    let out = decode_faces(&records);
    assert_eq!(out.len(), 13 * 6);
    let mut below = Vec::new();
    build_faces(
        &mut below,
        &tex,
        0,
        0,
        RelativeCameraPos::BelowClouds,
        2,
        false,
    );
    assert_eq!(records, below);
    for face in out.chunks(6) {
        // The top colour (1.0), not the down face's own 0.7.
        assert_eq!(face[0].shade, 1.0);
        // The face lies at the layer's bottom.
        for v in face {
            assert_eq!(v.pos[1], 0.0);
        }
        // The corners are the down face's, in the regular order, relative to the cell
        // the quad sits in (its min corner).
        let base = [
            face.iter().map(|v| v.pos[0]).fold(f32::MAX, f32::min),
            0.0,
            face.iter().map(|v| v.pos[2]).fold(f32::MAX, f32::min),
        ];
        let corners = corners(Dir::Down).map(|c| {
            [
                base[0] + c[0] * CELL_SIZE,
                c[1] * CELL_HEIGHT,
                base[2] + c[2] * CELL_SIZE,
            ]
        });
        for (i, c) in [0usize, 1, 2, 0, 2, 3].iter().enumerate() {
            assert_eq!(face[i].pos, corners[*c]);
        }
    }
    // A single flat cell is exactly the one face (vanilla's buildFlatCell).
    let mut one = Vec::new();
    super::flat::build_flat_cell(&mut one, 3, -2);
    assert_eq!(one.len(), 1);
    assert_eq!(decode_faces(&one)[0].shade, 1.0);
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

/// A jar with a small `clouds.png` (two solid texels, the rest empty) plus the entries
/// `Assets::open` checks, in a fresh temp dir.
pub(super) fn synthetic_assets() -> haste_assets::Assets {
    use std::io::Write;
    // One directory per call: tests run in parallel threads of one process, and a shared jar
    // was rewritten by one test while another read it ("failed to fill whole buffer").
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("haste-render-clouds-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let jar = dir.join("client.jar");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&jar).unwrap());
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("assets/minecraft/blockstates/stone.json", options)
        .unwrap();
    zip.write_all(b"{\"variants\":{\"\":{\"model\":\"minecraft:block/stone\"}}}")
        .unwrap();
    zip.start_file("assets/minecraft/lang/en_us.json", options)
        .unwrap();
    zip.write_all(b"{}").unwrap();
    let mut png = Vec::new();
    let mut img = image::RgbaImage::new(4, 4);
    img.put_pixel(1, 1, image::Rgba([255, 255, 255, 255]));
    img.put_pixel(2, 2, image::Rgba([255, 255, 255, 255]));
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    zip.start_file("assets/minecraft/textures/environment/clouds.png", options)
        .unwrap();
    zip.write_all(&png).unwrap();
    zip.finish().unwrap();
    haste_assets::Assets::open(&jar, &dir.join("assets"), "test").unwrap()
}
