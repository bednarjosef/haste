// The extruded cloud cell mesh (vanilla `CloudRenderer`'s texture analysis and
// `buildMesh`/`buildExtrudedCell`): the `clouds.png` cells with their wrapped
// neighbour flags, and the faces each cell draws around the camera.

use haste_assets::Assets;
use haste_core::Identifier;

use super::{CloudVertex, CELL_HEIGHT, CELL_SIZE};

/// A clouds.png texel is a cloud cell when its alpha is at least this (vanilla
/// `isCellEmpty`: alpha < 10 is empty).
const CELL_EMPTY_ALPHA: u8 = 10;

/// The face directions of an extruded cell, vanilla `Direction.get3DDataValue` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Dir {
    Down = 0,
    Up = 1,
    North = 2,
    South = 3,
    West = 4,
    East = 5,
}

impl Dir {
    const ALL: [Dir; 6] = [
        Dir::Down,
        Dir::Up,
        Dir::North,
        Dir::South,
        Dir::West,
        Dir::East,
    ];
    /// The face shade vanilla's `clouds.vsh` gives the direction (bottom 0.7, top 1.0,
    /// north/south 0.8, west/east 0.9).
    fn shade(self) -> f32 {
        match self {
            Dir::Down => 0.7,
            Dir::Up => 1.0,
            Dir::North | Dir::South => 0.8,
            Dir::West | Dir::East => 0.9,
        }
    }
    /// The four corners of the face in cell units (vanilla `clouds.vsh`'s vertex table).
    pub(super) fn corners(self) -> [[f32; 3]; 4] {
        match self {
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
}

/// One cell of `clouds.png` (vanilla `packCellData`): the pixel colour is unused (the
/// colour comes from the `cloud_color` attribute), so only the four neighbour-empty
/// flags are kept, `false` for an empty cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Cell {
    pub(super) empty: bool,
    pub(super) north: bool,
    pub(super) east: bool,
    pub(super) south: bool,
    pub(super) west: bool,
}

impl Cell {
    fn from_pixels(pixels: &image::RgbaImage, x: u32, y: u32) -> Cell {
        let (w, h) = pixels.dimensions();
        let is_empty = |x: u32, y: u32| pixels.get_pixel(x, y).0[3] < CELL_EMPTY_ALPHA;
        if is_empty(x, y) {
            return Cell {
                empty: true,
                north: false,
                east: false,
                south: false,
                west: false,
            };
        }
        // The neighbours wrap around the texture (vanilla's floorMod lookups).
        let north = is_empty(x, (y + h - 1) % h);
        let south = is_empty(x, (y + 1) % h);
        let west = is_empty((x + w - 1) % w, y);
        let east = is_empty((x + 1) % w, y);
        Cell {
            empty: false,
            north,
            east,
            south,
            west,
        }
    }
}

/// The clouds texture as cells (vanilla `TextureData`): one `Cell` per texel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloudTexture {
    pub(super) cells: Vec<Cell>,
    pub(super) width: u32,
    pub(super) height: u32,
}

impl CloudTexture {
    /// Reads `environment/clouds.png` and analyses the neighbours of every cell. `None`
    /// when the texture is missing (no clouds are drawn).
    pub fn load(assets: &mut Assets) -> Option<CloudTexture> {
        let img = match assets.texture(&Identifier::minecraft("environment/clouds")) {
            Ok(img) => img,
            Err(e) => {
                tracing::warn!("clouds texture: {e:#}");
                return None;
            }
        };
        Some(Self::from_image(&img))
    }

    pub(super) fn from_image(img: &image::RgbaImage) -> CloudTexture {
        let (w, h) = img.dimensions();
        let mut cells = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                cells.push(Cell::from_pixels(img, x, y));
            }
        }
        CloudTexture {
            cells,
            width: w,
            height: h,
        }
    }

    pub(super) fn cell(&self, x: i32, z: i32) -> Cell {
        self.cells
            .get(
                x.rem_euclid(self.width as i32) as usize
                    + z.rem_euclid(self.height as i32) as usize * self.width as usize,
            )
            .copied()
            .unwrap_or(Cell {
                empty: true,
                north: false,
                east: false,
                south: false,
                west: false,
            })
    }
}

/// Where the camera sits relative to the cloud layer (vanilla `RelativeCameraPos`; the
/// extruded cell draws different faces from below, inside and above).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelativeCameraPos {
    AboveClouds,
    InsideClouds,
    BelowClouds,
}

impl RelativeCameraPos {
    /// Vanilla `prepare`: above when the layer's top is below the camera, below when its
    /// bottom is above the camera, inside otherwise.
    pub fn of(camera_y: f64, cloud_bottom: f64) -> RelativeCameraPos {
        let bottom = (cloud_bottom - camera_y) as f32;
        let top = bottom + CELL_HEIGHT;
        if top < 0.0 {
            RelativeCameraPos::AboveClouds
        } else if bottom > 0.0 {
            RelativeCameraPos::BelowClouds
        } else {
            RelativeCameraPos::InsideClouds
        }
    }
}

/// Vanilla `buildMesh`: every cell of the disc of `radius_cells` cells around the
/// camera's cell (x² + z² <= radius²), taking the cloud state from the texture with
/// wraparound.
pub(super) fn build_mesh(
    out: &mut Vec<CloudVertex>,
    texture: &CloudTexture,
    center_x: i32,
    center_z: i32,
    relative_pos: RelativeCameraPos,
    radius_cells: i32,
) {
    for ring in 0..=2 * radius_cells {
        for x in -ring..=ring {
            let z = ring - x.abs();
            if z >= 0 && z <= radius_cells && x * x + z * z <= radius_cells * radius_cells {
                if z != 0 {
                    try_build_cell(out, texture, center_x, center_z, x, -z, relative_pos);
                }
                try_build_cell(out, texture, center_x, center_z, x, z, relative_pos);
            }
        }
    }
}

/// Vanilla `tryBuildCell`: the cell's texture state decides whether anything is drawn.
pub(super) fn try_build_cell(
    out: &mut Vec<CloudVertex>,
    texture: &CloudTexture,
    center_x: i32,
    center_z: i32,
    x: i32,
    z: i32,
    relative_pos: RelativeCameraPos,
) {
    let cell = texture.cell(center_x + x, center_z + z);
    if cell.empty {
        return;
    }
    build_extruded_cell(out, x, z, cell, relative_pos);
}

/// Vanilla `buildExtrudedCell`: top and bottom by the camera's position relative to the
/// layer, the side faces of empty neighbours only on the far side of the camera (north
/// for z > 0 and so on), and all six faces of the nine cells around the camera reversed
/// (vanilla's `FLAG_INSIDE_FACE`) so they are visible from inside a cloud.
pub(super) fn build_extruded_cell(
    out: &mut Vec<CloudVertex>,
    x: i32,
    z: i32,
    cell: Cell,
    relative_pos: RelativeCameraPos,
) {
    if relative_pos != RelativeCameraPos::BelowClouds {
        encode_face(out, x, z, Dir::Up, false);
    }
    if relative_pos != RelativeCameraPos::AboveClouds {
        encode_face(out, x, z, Dir::Down, false);
    }
    if cell.north && z > 0 {
        encode_face(out, x, z, Dir::North, false);
    }
    if cell.south && z < 0 {
        encode_face(out, x, z, Dir::South, false);
    }
    if cell.west && x > 0 {
        encode_face(out, x, z, Dir::West, false);
    }
    if cell.east && x < 0 {
        encode_face(out, x, z, Dir::East, false);
    }
    if x.abs() <= 1 && z.abs() <= 1 {
        for dir in Dir::ALL {
            encode_face(out, x, z, dir, true);
        }
    }
}

/// Vanilla `encodeFace` + the vertex shader's position math: the face's corners in
/// camera-cell-relative blocks (the per-frame offset inside the cell comes from the
/// uniform), two triangles per quad like vanilla's quad index buffer. `inside` faces
/// reverse the vertex order (vanilla's `FLAG_INSIDE_FACE`).
fn encode_face(out: &mut Vec<CloudVertex>, x: i32, z: i32, dir: Dir, inside: bool) {
    let shade = dir.shade();
    let corner = |c: [f32; 3]| CloudVertex {
        pos: [
            (x as f32 + c[0]) * CELL_SIZE,
            c[1] * CELL_HEIGHT,
            (z as f32 + c[2]) * CELL_SIZE,
        ],
        shade,
    };
    let c = dir.corners();
    // Vanilla's sequential quad index buffer: (0, 1, 2) and (0, 2, 3); inside faces use
    // the reversed order (3 - quadVertex), which is the same two triangles wound the
    // other way.
    let order: [usize; 6] = if inside {
        [3, 2, 1, 3, 1, 0]
    } else {
        [0, 1, 2, 0, 2, 3]
    };
    for i in order {
        out.push(corner(c[i]));
    }
}
