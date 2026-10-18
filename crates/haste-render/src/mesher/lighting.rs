// This file is derived from Pomme (https://github.com/josefbednar/pomme),
// pomme-client/src/renderer/chunk/mesher.rs (compute_face_ao), reused under
// GPL-3.0 and reworked for Haste. See THIRD_PARTY.md.

use super::MeshContext;

/// The per-face AO neighbour offsets (see `face_corner_table`), built once.
static FACE_CORNERS: std::sync::OnceLock<[[[[i32; 3]; 3]; 4]; 6]> = std::sync::OnceLock::new();

fn face_corners() -> &'static [[[[i32; 3]; 3]; 4]; 6] {
    FACE_CORNERS.get_or_init(face_corner_table)
}

/// The per-face side-0 cell offsets (see `face_side0_table`), built once.
static FACE_SIDE0: std::sync::OnceLock<[[i32; 3]; 6]> = std::sync::OnceLock::new();

pub(super) fn face_side0() -> &'static [[i32; 3]; 6] {
    FACE_SIDE0.get_or_init(face_side0_table)
}

/// Face shade index for packing: 0..5 = down/up/north/south/west/east,
/// 6 = unshaded (shade: false elements).
pub(super) fn shade_index(dir: Option<haste_assets::Direction>) -> u8 {
    match dir {
        Some(d) => d.index() as u8,
        None => 6,
    }
}

/// Vanilla ambient occlusion brightness as an occluder count (vanilla
/// `BlockModelLighter.prepareQuadAmbientOcclusion`): the corner's brightness
/// is the average of four shade samples — the two sides, the diagonal and the
/// face-adjacent cell — each 1.0 or 0.2 (`getShadeBrightness`), so brightness
/// = 1.0 - 0.2 * count, count 0..4. The diagonal is the caller's to resolve:
/// vanilla reads side 0's sample when both of the vertex's side cells are
/// non-light-permeable.
pub(super) fn vertex_ao_level(side1: bool, side2: bool, diagonal: bool, center: bool) -> u8 {
    side1 as u8 + side2 as u8 + diagonal as u8 + center as u8
}

/// The four (side1, side2, corner) neighbour offsets of each face vertex, in
/// face_positions vertex order (vanilla AdjacencyInfo), precomputed per face
/// direction: `face_corner_offsets` is a pure function of the direction, so the
/// mesh loop reads a table instead of recomputing it per vertex.
fn face_corner_table() -> [[[[i32; 3]; 3]; 4]; 6] {
    let mut table = [[[[0i32; 3]; 3]; 4]; 6];
    for (di, dir) in haste_assets::Direction::ALL.into_iter().enumerate() {
        let (u_axis, v_axis) = face_axes(dir);
        table[di] = face_corner_offsets(dir, u_axis, v_axis);
    }
    table
}

/// The "side 0" cell offset per face (vanilla `BlockModelLighter.AdjacencyInfo`
/// `corners[0]` plus the face normal): vanilla substitutes this cell's shade and
/// light for a vertex's diagonal sample whenever both of that vertex's side
/// cells are non-light-permeable — whichever vertex the diagonal belongs to
/// (so for two of the four vertices it is not one of their own sides; that is
/// vanilla's behaviour, kept as the oracle).
fn face_side0_table() -> [[i32; 3]; 6] {
    use haste_assets::Direction as D;
    // AdjacencyInfo.corners[0] per face, in Haste's Direction::ALL order
    // (down, up, north, south, west, east).
    let side0 = [
        D::West, // down
        D::East, // up
        D::Up,   // north
        D::West, // south
        D::Up,   // west
        D::Down, // east
    ];
    let mut table = [[0i32; 3]; 6];
    for (di, dir) in D::ALL.into_iter().enumerate() {
        let n = dir.offset();
        let s = side0[di].offset();
        table[di] = [n[0] + s[0], n[1] + s[1], n[2] + s[2]];
    }
    table
}

/// One AO/light sample cell: whether it darkens (`getShadeBrightness <= 0.2`)
/// and its packed light.
#[derive(Clone, Copy)]
pub(super) struct AoSample {
    pub(super) occludes: bool,
    pub(super) light: u8,
}

/// The per-vertex corner blend, vanilla `BlockModelLighter`'s cubic path:
/// the AO occluder count over the two sides, the diagonal (the caller has
/// already applied vanilla's side-0 substitution) and the center, plus the
/// light blend of `LightCoordsUtil.smoothBlend` ([`smooth_blend`]). The light comes back in vanilla's smooth units
/// (16 per level, 0..240 — the exact average, like vanilla keeps into the
/// lightmap; `LIGHT16_PER_LEVEL`).
pub(super) fn ao_corner_blend(
    sides: [AoSample; 2],
    diagonal: AoSample,
    center: AoSample,
) -> (u8, [u8; 2]) {
    let ao = vertex_ao_level(
        sides[0].occludes,
        sides[1].occludes,
        diagonal.occludes,
        center.occludes,
    );
    let (sky, block) = smooth_blend(
        [sides[0].light, sides[1].light, diagonal.light],
        center.light,
    );
    // Vanilla keeps the exact average in 16th-of-a-level units (0..240) into
    // the lightmap; the vertex carries it (`LIGHT16_PER_LEVEL`).
    (
        ao,
        [
            (sky * 4).min(crate::vertex::LIGHT16_MAX as u32) as u8,
            (block * 4).min(crate::vertex::LIGHT16_MAX as u32) as u8,
        ],
    )
}

/// Vanilla `LightCoordsUtil.smoothBlend` on packed (sky << 4 | block) cell
/// lights: when the center is lit (sky > 2 or block > 2), a fully dark
/// neighbour (inside an opaque block) takes the center's light and a zero-sky
/// one the center's sky; then the four are summed per channel. Returns the
/// (sky, block) sums in whole levels, 0..60 (the caller scales the average to
/// 16ths).
pub(super) fn smooth_blend(neighbours: [u8; 3], center: u8) -> (u32, u32) {
    let (cs, cb) = (center >> 4, center & 0xF);
    let lit = cs > 2 || cb > 2;
    let mut sky = cs as u32;
    let mut block = cb as u32;
    for n in neighbours {
        let n = if lit && n == 0 {
            center
        } else if lit && n >> 4 == 0 {
            (cs << 4) | (n & 0xF)
        } else {
            n
        };
        sky += (n >> 4) as u32;
        block += (n & 0xF) as u32;
    }
    (sky, block)
}

/// Per-vertex AO occluder count + smooth light for face vertex `vi` (vanilla
/// `BlockModelLighter.prepareQuadAmbientOcclusion`). Coordinates are
/// section-local; the +1 maps the section into the snapshot's border.
/// `cubic` is the quad's faceCubic flag: it picks the center sample (the
/// face-adjacent cell when cubic, else the block's own cell).
pub(super) fn vertex_light_for(
    ctx: &MeshContext,
    lx: i32,
    ly: i32,
    lz: i32,
    dir: haste_assets::Direction,
    cubic: bool,
    vi: usize,
) -> (u8, [u8; 2]) {
    // (side1, side2, corner) offsets per vertex, in face_positions order.
    let corners = face_corners()[dir.index()][vi];
    let [s1, s2, corner] = corners;
    let (x, y, z) = (lx + 1, ly + 1, lz + 1);

    let cell = |d: [i32; 3]| ctx.cell(x + d[0], y + d[1], z + d[2]);
    // The AO occluder test (`BlockBehaviour.getShadeBrightness`): a full
    // collision cube darkens (0.2), not texture opacity — glass and slabs
    // keep their neighbours bright.
    let sample = |d: [i32; 3]| AoSample {
        occludes: ctx.tables.ao.is_solid(cell(d).state),
        light: cell(d).light,
    };
    // Vanilla `BlockState.isLightPermeable` (whether the diagonal sample is
    // reachable), tested — as in `BlockModelLighter` — at the cells ONE STEP
    // OUT from the side cells along the face normal (`side cell + normal`),
    // not at the side cells themselves: a terrain step's side cell sits inside
    // the raised neighbour (non-permeable) while the cell above it is open, so
    // testing the side cell would substitute the diagonal's dark sample where
    // vanilla keeps it. Approximated as solid-render (`baked.is_full_opaque`);
    // the light-dampening half of the vanilla test has no data in the client
    // yet (the server's light values already carry it).
    let n = dir.offset();
    let out = |d: [i32; 3]| [d[0] + n[0], d[1] + n[1], d[2] + n[2]];
    let permeable_out = |d: [i32; 3]| !ctx.baked.is_full_opaque(cell(out(d)).state);

    // The diagonal sample: the corner cell, or vanilla's side-0 cell when
    // both one-out side cells are non-permeable (vanilla reads side 0's shade
    // and light there — not this vertex's own side).
    let diagonal = if !permeable_out(s1) && !permeable_out(s2) {
        sample(face_side0()[dir.index()])
    } else {
        sample(corner)
    };

    // Center sample (vanilla `prepareQuadShape`'s faceCubic split): the shade
    // comes from the face-adjacent cell for a cubic face and from the block's
    // own cell otherwise; the light comes from the face-adjacent cell for a
    // cubic face or one visible through a non-solid-render neighbour, and
    // from the block's own cell otherwise. The corner table's offsets already
    // carry the face normal (checked against vanilla's AdjacencyInfo by
    // `face_corner_offsets_match_vanilla_adjacency`), so sampling one cell
    // further out would make two adjacent coplanar faces read different
    // cells and draw a line along every block edge.
    let own = cell([0, 0, 0]);
    let center = AoSample {
        occludes: if cubic {
            ctx.tables.ao.is_solid(cell(n).state)
        } else {
            ctx.tables.ao.is_solid(own.state)
        },
        light: if cubic || !ctx.baked.is_full_opaque(cell(n).state) {
            cell(n).light
        } else {
            own.light
        },
    };
    ao_corner_blend([sample(s1), sample(s2)], diagonal, center)
}

/// The two in-plane axes of a face (u, v), each a unit offset.
pub(super) fn face_axes(dir: haste_assets::Direction) -> ([i32; 3], [i32; 3]) {
    use haste_assets::Direction as D;
    match dir {
        D::Down | D::Up => ([1, 0, 0], [0, 0, 1]),
        D::North | D::South => ([1, 0, 0], [0, 1, 0]),
        D::West | D::East => ([0, 0, 1], [0, 1, 0]),
    }
}

/// The four (side1, side2, corner) neighbour offsets of a face's vertices, in
/// face_positions vertex order (vanilla AdjacencyInfo).
pub(super) fn face_corner_offsets(
    dir: haste_assets::Direction,
    u: [i32; 3],
    v: [i32; 3],
) -> [[[i32; 3]; 3]; 4] {
    use haste_assets::Direction as D;
    // Vertex order per face_positions: which corner of the u-v plane each
    // vertex sits at.
    let corner_signs: [[i32; 2]; 4] = match dir {
        D::Down => [[0, 1], [0, 0], [1, 0], [1, 1]],
        D::Up => [[0, 0], [0, 1], [1, 1], [1, 0]],
        D::North => [[1, 1], [1, 0], [0, 0], [0, 1]],
        D::South => [[0, 1], [0, 0], [1, 0], [1, 1]],
        D::West => [[0, 1], [0, 0], [1, 0], [1, 1]],
        D::East => [[1, 1], [1, 0], [0, 0], [0, 1]],
    };
    let n = dir.offset();
    corner_signs.map(|[su, sv]| {
        // 0 -> -1, 1 -> +1: the side neighbour toward this vertex along each axis.
        let (fu, fv) = (su * 2 - 1, sv * 2 - 1);
        [
            [n[0] + fu * u[0], n[1] + fu * u[1], n[2] + fu * u[2]],
            [n[0] + fv * v[0], n[1] + fv * v[1], n[2] + fv * v[2]],
            [
                n[0] + fu * u[0] + fv * v[0],
                n[1] + fu * u[1] + fv * v[1],
                n[2] + fu * u[2] + fv * v[2],
            ],
        ]
    })
}
