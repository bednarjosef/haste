// The cloud cell face records (vanilla `CloudRenderer`'s texture analysis and
// `buildMesh`/`buildExtrudedCell`): the `clouds.png` cells with their wrapped
// neighbour flags, and — like vanilla — one compact 3-byte record per face the
// cells draw around the camera, decoded and expanded to quads by the vertex
// shader (`clouds.wgsl`, vanilla `clouds.vsh`).

use haste_assets::Assets;
use haste_core::Identifier;

use super::CELL_HEIGHT;

/// A clouds.png texel is a cloud cell when its alpha is at least this (vanilla
/// `isCellEmpty`: alpha < 10 is empty).
const CELL_EMPTY_ALPHA: u8 = 10;

/// The face directions of an extruded cell, vanilla `Direction.get3DDataValue` order
/// (the record's low three bits).
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

    /// The direction of a record's low three bits (vanilla
    /// `Direction.get3DDataValue`'s order).
    #[cfg(test)]
    pub(super) fn from_index(bits: u32) -> Dir {
        Self::ALL[(bits & 7) as usize]
    }
}

/// Vanilla `encodeFace`'s flags. `INSIDE` reverses the quad's winding (visible from
/// within a cloud); `USE_TOP_COLOR` shades the face with the top colour whatever its
/// direction (the flat cells' down faces).
pub(super) const FLAG_INSIDE_FACE: u32 = 16;
pub(super) const FLAG_USE_TOP_COLOR: u32 = 32;

/// One face record, vanilla `encodeFace`'s three bytes packed into the low three bytes
/// of a u32 (the instance buffer reads whole records): `x >> 1` and `z >> 1` (signed
/// bytes; the half-resolution cells reach ±127), and `dir | flags | (x & 1) << 7 |
/// (z & 1) << 6`. The cell's low bit travels in the flags byte, exactly vanilla's
/// layout, decoded by `clouds.wgsl`.
pub(super) fn pack_face(x: i32, z: i32, dir: Dir, top_color: bool, inside: bool) -> u32 {
    let mut dir_and_flags = dir as u32;
    if inside {
        dir_and_flags |= FLAG_INSIDE_FACE;
    }
    if top_color {
        dir_and_flags |= FLAG_USE_TOP_COLOR;
    }
    dir_and_flags |= (x as u32 & 1) << 7;
    dir_and_flags |= (z as u32 & 1) << 6;
    ((x >> 1) as u8 as u32) | ((z >> 1) as u8 as u32) << 8 | dir_and_flags << 16
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
/// wraparound, emits its faces' records. `fancy` is vanilla's `extrude`: the extruded
/// boxes of the FANCY mode or the flat cells of the FAST one.
pub(super) fn build_faces(
    out: &mut Vec<u32>,
    texture: &CloudTexture,
    center_x: i32,
    center_z: i32,
    relative_pos: RelativeCameraPos,
    radius_cells: i32,
    fancy: bool,
) {
    for ring in 0..=2 * radius_cells {
        for x in -ring..=ring {
            let z = ring - x.abs();
            if z >= 0 && z <= radius_cells && x * x + z * z <= radius_cells * radius_cells {
                if z != 0 {
                    try_build_cell(out, texture, center_x, center_z, x, -z, relative_pos, fancy);
                }
                try_build_cell(out, texture, center_x, center_z, x, z, relative_pos, fancy);
            }
        }
    }
}

/// Vanilla `tryBuildCell`: the cell's texture state decides whether anything is drawn.
/// The parameter list mirrors vanilla's own.
#[allow(clippy::too_many_arguments)]
pub(super) fn try_build_cell(
    out: &mut Vec<u32>,
    texture: &CloudTexture,
    center_x: i32,
    center_z: i32,
    x: i32,
    z: i32,
    relative_pos: RelativeCameraPos,
    fancy: bool,
) {
    let cell = texture.cell(center_x + x, center_z + z);
    if cell.empty {
        return;
    }
    if fancy {
        build_extruded_cell(out, x, z, cell, relative_pos);
    } else {
        super::flat::build_flat_cell(out, x, z);
    }
}

/// Vanilla `buildExtrudedCell`: top and bottom by the camera's position relative to the
/// layer, the side faces of empty neighbours only on the far side of the camera (north
/// for z > 0 and so on), and all six faces of the nine cells around the camera reversed
/// (vanilla's `FLAG_INSIDE_FACE`) so they are visible from inside a cloud.
pub(super) fn build_extruded_cell(
    out: &mut Vec<u32>,
    x: i32,
    z: i32,
    cell: Cell,
    relative_pos: RelativeCameraPos,
) {
    if relative_pos != RelativeCameraPos::BelowClouds {
        out.push(pack_face(x, z, Dir::Up, false, false));
    }
    if relative_pos != RelativeCameraPos::AboveClouds {
        out.push(pack_face(x, z, Dir::Down, false, false));
    }
    if cell.north && z > 0 {
        out.push(pack_face(x, z, Dir::North, false, false));
    }
    if cell.south && z < 0 {
        out.push(pack_face(x, z, Dir::South, false, false));
    }
    if cell.west && x > 0 {
        out.push(pack_face(x, z, Dir::West, false, false));
    }
    if cell.east && x < 0 {
        out.push(pack_face(x, z, Dir::East, false, false));
    }
    if x.abs() <= 1 && z.abs() <= 1 {
        for dir in Dir::ALL {
            out.push(pack_face(x, z, dir, false, true));
        }
    }
}
