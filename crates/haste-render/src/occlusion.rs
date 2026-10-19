// This file is derived from Pomme (https://github.com/josefbednar/pomme),
// pomme-client/src/renderer/chunk/occlusion_graph.rs, reused under GPL-3.0
// (vanilla-style section visibility for cave culling: VisGraph flood fill +
// SectionOcclusionGraph walk). See THIRD_PARTY.md.

use std::collections::VecDeque;

use glam::DVec3;
use haste_core::{ChunkPos, SectionPos};

/// The six section faces, ordinals matching vanilla Direction.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Face {
    Down = 0,
    Up = 1,
    North = 2,
    South = 3,
    West = 4,
    East = 5,
}

impl Face {
    pub const ALL: [Face; 6] = [
        Face::Down,
        Face::Up,
        Face::North,
        Face::South,
        Face::West,
        Face::East,
    ];

    /// Unit step toward this face in section space.
    pub fn offset(self) -> (i32, i32, i32) {
        match self {
            Face::Down => (0, -1, 0),
            Face::Up => (0, 1, 0),
            Face::North => (0, 0, -1),
            Face::South => (0, 0, 1),
            Face::West => (-1, 0, 0),
            Face::East => (1, 0, 0),
        }
    }

    /// The face's axis: 0 x (west/east), 1 y (down/up), 2 z (north/south).
    pub fn axis(self) -> usize {
        match self {
            Face::Down | Face::Up => 1,
            Face::North | Face::South => 2,
            Face::West | Face::East => 0,
        }
    }

    pub fn opposite(self) -> Face {
        match self {
            Face::Down => Face::Up,
            Face::Up => Face::Down,
            Face::North => Face::South,
            Face::South => Face::North,
            Face::West => Face::East,
            Face::East => Face::West,
        }
    }

    fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// Symmetric 6x6 face-to-face connectivity (36 bits): bit `a*6+b` set means a
/// sightline crosses the section between faces `a` and `b`. A fully-empty
/// section connects all faces; a fully-solid one connects none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VisibilitySet(u64);

impl VisibilitySet {
    pub const fn none() -> Self {
        VisibilitySet(0)
    }

    pub const fn all() -> Self {
        VisibilitySet((1u64 << 36) - 1)
    }

    /// Whether a sightline crosses the section between faces `a` and `b`.
    pub fn visible_between(self, a: Face, b: Face) -> bool {
        self.0 >> (a as u32 * 6 + b as u32) & 1 != 0
    }

    /// 6-bit mask with bit `a` set when a sightline crosses from face `a` to
    /// `face`: the column of the matrix belonging to `face`. The matrix is
    /// symmetric (only `VisibilitySet::add` builds it, setting both `a*6+b`
    /// and `b*6+a`), so the column equals the row and one shift extracts it.
    pub fn reach_to(self, face: Face) -> u64 {
        (self.0 >> (face as u64 * 6)) & 0x3F
    }

    /// Mark every pair of faces in `faces` (a 6-bit mask) mutually visible.
    fn add(&mut self, faces: u8) {
        for a in 0..6u8 {
            if faces & (1 << a) == 0 {
                continue;
            }
            for b in 0..6u8 {
                if faces & (1 << b) != 0 {
                    self.0 |= 1u64 << (a * 6 + b);
                }
            }
        }
    }
}

const fn idx(x: usize, y: usize, z: usize) -> usize {
    x + y * 16 + z * 256
}

/// Swap the three bit-pairs of a 6-bit face mask: Down<->Up, North<->South,
/// West<->East. Turns "entered from face f" into "may exit towards f.opposite".
fn reverse_face_pairs(m: u8) -> u64 {
    (((m & 0b010101) as u64) << 1) | (((m & 0b101010) as u64) >> 1)
}

/// Faces of the section that cell `(x, y, z)` lies on.
fn edge_faces(x: usize, y: usize, z: usize) -> u8 {
    let mut f = 0u8;
    if x == 0 {
        f |= Face::West.bit();
    } else if x == 15 {
        f |= Face::East.bit();
    }
    if y == 0 {
        f |= Face::Down.bit();
    } else if y == 15 {
        f |= Face::Up.bit();
    }
    if z == 0 {
        f |= Face::North.bit();
    } else if z == 15 {
        f |= Face::South.bit();
    }
    f
}

// Reusable flood-fill stack for `compute_visibility` (per thread; the fill
// is not reentrant).
thread_local! {
    static VISIT_STACK: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Compute a section's `VisibilitySet` from its 16³ opacity grid via vanilla's
/// `VisGraph`: flood each connected non-opaque region seeded from the boundary
/// and connect every section face that region reaches.
pub fn compute_visibility(opaque: impl Fn(usize, usize, usize) -> bool) -> VisibilitySet {
    let mut blocked = [false; 4096];
    let mut opaque_count = 0usize;
    for z in 0..16usize {
        for y in 0..16usize {
            for x in 0..16usize {
                if opaque(x, y, z) {
                    blocked[idx(x, y, z)] = true;
                    opaque_count += 1;
                }
            }
        }
    }
    // VisGraph.resolve() short-circuits: sparse fill => every face connected,
    // fully solid => none.
    if opaque_count < 256 {
        return VisibilitySet::all();
    }
    if opaque_count == 4096 {
        return VisibilitySet::none();
    }

    VISIT_STACK.with(|cell| {
        let mut stack = cell.borrow_mut();
        stack.clear();
        let mut vis = VisibilitySet::none();
        for z in 0..16usize {
            for y in 0..16usize {
                for x in 0..16usize {
                    if x != 0 && x != 15 && y != 0 && y != 15 && z != 0 && z != 15 {
                        continue;
                    }
                    let seed = idx(x, y, z);
                    if blocked[seed] {
                        continue;
                    }
                    blocked[seed] = true;
                    stack.push(seed);
                    let mut faces = 0u8;
                    while let Some(i) = stack.pop() {
                        let (cx, cy, cz) = (i & 15, i >> 4 & 15, i >> 8 & 15);
                        faces |= edge_faces(cx, cy, cz);
                        for face in Face::ALL {
                            let (dx, dy, dz) = face.offset();
                            let (nx, ny, nz) = (cx as i32 + dx, cy as i32 + dy, cz as i32 + dz);
                            if !(0..16).contains(&nx)
                                || !(0..16).contains(&ny)
                                || !(0..16).contains(&nz)
                            {
                                continue;
                            }
                            let ni = idx(nx as usize, ny as usize, nz as usize);
                            if blocked[ni] {
                                continue;
                            }
                            blocked[ni] = true;
                            stack.push(ni);
                        }
                    }
                    vis.add(faces);
                }
            }
        }
        vis
    })
}

/// Vanilla advanced-culling thresholds (Pomme's constants).
const ADV_CULL_SECTION_DIST: i32 = 3;
const CEILED_SECTION_DIAGONAL: f64 = 28.0;
const ADV_CULL_MIN_DIST_SQ: f64 = 3600.0;

/// The sections the walk reached, per column, over the square of columns around
/// the camera. `order` lists them in BFS pop order — vanilla draws its sections
/// in its occlusion graph's node order (`SectionOcclusionGraph.addSectionsInFrustum`
/// appends them as the tree is visited), so opaque terrain needs no sort.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VisibleMask {
    cx: i32,
    cz: i32,
    r: i32,
    /// Vertical section count of the walk's grid (`section_count`).
    h: i32,
    columns: Vec<u32>,
    /// Node index per reached section, in BFS pop order: for a grid of
    /// `(2r+1)^2` columns x `h` sections, `((dx * w + dz) * h + si)`. The
    /// background walk drops the known-empty sections (`retain_order`).
    order: Vec<u32>,
}

impl VisibleMask {
    /// Bit `si` set when section index `si` of the column was reached.
    pub fn column(&self, c: ChunkPos) -> u32 {
        let (dx, dz) = (c.x - self.cx + self.r, c.z - self.cz + self.r);
        let w = 2 * self.r + 1;
        if self.columns.is_empty() || dx < 0 || dz < 0 || dx >= w || dz >= w {
            return 0;
        }
        self.columns[(dx * w + dz) as usize]
    }

    /// The reached sections in BFS pop order (vanilla's draw order for opaque
    /// terrain). Yields `(column, section index)`; empty until the first walk.
    pub fn ordered(&self) -> impl Iterator<Item = (ChunkPos, i32)> + '_ {
        let w = 2 * self.r + 1;
        let h = self.h;
        let cx = self.cx;
        let cz = self.cz;
        let r = self.r;
        self.order.iter().map(move |&n| {
            let (col, si) = ((n / h as u32) as i32, (n % h as u32) as i32);
            (ChunkPos::new(cx + col / w - r, cz + col % w - r), si)
        })
    }

    /// Keeps only the nodes of `order` that `keep` accepts (by node index); the
    /// column bits stay, so `column` still answers "was it reached".
    pub(crate) fn retain_order(&mut self, keep: impl FnMut(u32) -> bool) {
        let mut keep = keep;
        self.order.retain(|&n| keep(n));
    }

    /// HashMap-style lookup (tests).
    pub fn get(&self, c: &ChunkPos) -> Option<u32> {
        Some(self.column(*c)).filter(|m| *m != 0)
    }
}

/// Walk state over a flat grid of (2r+1)^2 columns x `section_count` sections.
struct Grid {
    cx: i32,
    cz: i32,
    r: i32,
    h: i32,
    /// Per node: entered-from faces, cone, visited flag (bit 7 of `cone`).
    source: Vec<u8>,
    cone: Vec<u8>,
}

const VISITED: u8 = 0x80;

impl Grid {
    fn index(&self, x: i32, si: i32, z: i32) -> Option<usize> {
        let (dx, dz) = (x - self.cx + self.r, z - self.cz + self.r);
        let w = 2 * self.r + 1;
        if dx < 0 || dz < 0 || dx >= w || dz >= w || si < 0 || si >= self.h {
            return None;
        }
        Some(((dx * w + dz) * self.h + si) as usize)
    }

    fn visited(&self, x: i32, si: i32, z: i32) -> bool {
        self.index(x, si, z)
            .is_some_and(|i| self.cone[i] & VISITED != 0)
    }
}

/// Vanilla's `SectionOcclusionGraph` walk (Pomme compute_visible_mask): flood
/// the section grid outward from the camera section, stepping into a neighbour
/// only when the path isn't a backtrack and a sightline crosses the current
/// section. Each section's `VisibilitySet` comes from the caller (the grid
/// builder in `walk`); sections the caller reports as opaque block the walk
/// like vanilla's `CompiledSectionMesh.UNCOMPILED` does. Frustum culling is
/// applied separately. Flat arrays instead of hash maps: a full walk at render
/// distance 12 visits ~15k nodes. `render_distance` is the grid's radius in
/// columns; `view_distance` bounds the walk like vanilla's
/// `SectionOcclusionGraph.getRelativeFrom`: a neighbour is entered only inside
/// the view circle (`ChunkTrackingView.isInViewDistance`) and within
/// `view_distance` sections of the camera vertically.
#[allow(clippy::too_many_arguments)]
pub fn compute_visible_mask(
    section_vis: impl Fn(ChunkPos, i32) -> VisibilitySet,
    cam_col: ChunkPos,
    cam_si: i32,
    eye: DVec3,
    min_y: i32,
    section_count: i32,
    render_distance: i32,
    view_distance: i32,
) -> VisibleMask {
    let r = render_distance.max(0);
    let w = 2 * r + 1;
    let h = section_count.max(1);
    let n = (w * w * h) as usize;
    let mut grid = Grid {
        cx: cam_col.x,
        cz: cam_col.z,
        r,
        h,
        source: vec![0; n],
        cone: vec![0; n],
    };
    let mut columns = vec![0u32; (w * w) as usize];
    let mut queue: VecDeque<(i32, i32, i32)> = VecDeque::with_capacity(1024);
    // Sections in BFS pop order (vanilla keeps its graph's node order and draws
    // the sections in it; the mask below is derived from the same list).
    let mut order: Vec<u32> = Vec::new();

    let cam_center = DVec3::new(
        (cam_col.x * 16 + 8) as f64,
        (min_y + cam_si * 16 + 8) as f64,
        (cam_col.z * 16 + 8) as f64,
    );
    let world_min_y = min_y as f64;
    let world_max_y = (min_y + section_count * 16) as f64;
    let min_y_f = min_y as f64;

    let start = (cam_col.x, cam_si.clamp(0, h - 1), cam_col.z);
    let si0 = grid.index(start.0, start.1, start.2).unwrap();
    grid.cone[si0] = VISITED;
    columns[(r * w + r) as usize] |= 1u32 << start.1;
    queue.push_back(start);

    while let Some((x, si, z)) = queue.pop_front() {
        let i = grid.index(x, si, z).unwrap();
        order.push(((x - cam_col.x + r) * w + (z - cam_col.z + r)) as u32 * h as u32 + si as u32);
        let source = grid.source[i];
        let cone = grid.cone[i] & !VISITED;
        let vis = section_vis(ChunkPos::new(x, z), si);
        let distant = (x - cam_col.x).abs() > ADV_CULL_SECTION_DIST
            || (si - cam_si).abs() > ADV_CULL_SECTION_DIST
            || (z - cam_col.z).abs() > ADV_CULL_SECTION_DIST;
        // The ray march depends on this section and the face's axis (vanilla picks
        // the start corner per direction: near the camera along the face's axis,
        // away from it along the other two), so at most three marches per pop. Visits
        // only accumulate during the pop, so a cached "blocked at section q" stays
        // blocked while q is unvisited, and a cached "clear" stays clear; if q was
        // reached in between, re-march.
        let mut ray: [Option<(bool, (i32, i32, i32))>; 3] = [None; 3];
        for face in Face::ALL {
            // Don't walk back the way we came (limits the search to a cone).
            if cone & face.opposite().bit() != 0 {
                continue;
            }
            // A sightline must cross this section from an entered face to `face`.
            // The start node (no source) may exit any direction.
            if source != 0 && (reverse_face_pairs(source) & vis.reach_to(face)) == 0 {
                continue;
            }
            let (dx, dy, dz) = face.offset();
            let (nx, nsi, nz) = (x + dx, si + dy, z + dz);
            let Some(ni) = grid.index(nx, nsi, nz) else {
                continue;
            };
            if !in_view_distance(cam_col, nx, nz, view_distance)
                || (nsi - cam_si).abs() > view_distance
            {
                continue;
            }
            // Distant sections also get a coarse ray-march back to the camera.
            if distant {
                let axis = face.axis();
                let march = || {
                    let (b, q) = ray_occluded(
                        &grid,
                        eye,
                        cam_center,
                        (x, si, z),
                        axis,
                        min_y_f,
                        world_min_y,
                        world_max_y,
                    );
                    (b, q.unwrap_or((0, 0, 0)))
                };
                let blocked = match ray[axis] {
                    None => {
                        let r = march();
                        ray[axis] = Some(r);
                        r.0
                    }
                    Some((false, _)) => false,
                    Some((true, q)) => {
                        if grid.visited(q.0, q.1, q.2) {
                            let r = march();
                            ray[axis] = Some(r);
                            r.0
                        } else {
                            true
                        }
                    }
                };
                if blocked {
                    continue;
                }
            }
            if grid.cone[ni] & VISITED != 0 {
                grid.source[ni] |= face.bit();
                continue;
            }
            grid.source[ni] = face.bit();
            grid.cone[ni] = VISITED | cone | face.bit();
            let col = ((nx - cam_col.x + r) * w + (nz - cam_col.z + r)) as usize;
            columns[col] |= 1u32 << nsi;
            queue.push_back((nx, nsi, nz));
        }
    }
    VisibleMask {
        cx: cam_col.x,
        cz: cam_col.z,
        r,
        h,
        columns,
        order,
    }
}

/// One coordinate of the ray's start corner (`SectionOcclusionGraph.runUpdates`'s
/// `maxX`/`maxY`/`maxZ`): along the face's axis the high side when the camera
/// section's centre is past the origin, along the other axes when it is before it.
fn start_corner(cam: f64, origin: f64, face_axis: bool) -> f64 {
    let high = if face_axis { cam > origin } else { cam < origin };
    origin + if high { 16.0 } else { 0.0 }
}

/// Vanilla `ChunkTrackingView.isInViewDistance` (`isWithinDistance` without the
/// neighbour buffer): the column's distance from the camera column, less one on
/// each axis, inside the view distance circle.
pub(crate) fn in_view_distance(cam: ChunkPos, x: i32, z: i32, view_distance: i32) -> bool {
    let dx = ((x - cam.x).abs() - 1).max(0) as i64;
    let dz = ((z - cam.z).abs() - 1).max(0) as i64;
    dx * dx + dz * dz < view_distance as i64 * view_distance as i64
}

/// Step from the distant section's start corner toward the camera by one
/// section diagonal at a time; if a step lands in a section the walk hasn't
/// reached, the line of sight is blocked. The corner is vanilla's for a face on
/// `axis` (0 x, 1 y, 2 z): the high side where the camera section's centre is
/// past the origin along that axis, and where it is before the origin along the
/// other two. Returns whether the sightline is blocked and the first unvisited
/// section along it (`None` when clear).
#[allow(clippy::too_many_arguments)]
fn ray_occluded(
    grid: &Grid,
    eye: DVec3,
    cam_center: DVec3,
    (sx, ssi, sz): (i32, i32, i32),
    axis: usize,
    min_y_f: f64,
    world_min_y: f64,
    world_max_y: f64,
) -> (bool, Option<(i32, i32, i32)>) {
    let ox = (sx * 16) as f64;
    let oy = min_y_f + (ssi * 16) as f64;
    let oz = (sz * 16) as f64;
    let mut check = DVec3::new(
        start_corner(cam_center.x, ox, axis == 0),
        start_corner(cam_center.y, oy, axis == 1),
        start_corner(cam_center.z, oz, axis == 2),
    );
    let step = (eye - check).normalize() * CEILED_SECTION_DIAGONAL;
    while check.distance_squared(eye) > ADV_CULL_MIN_DIST_SQ {
        check += step;
        if check.y > world_max_y || check.y < world_min_y {
            return (false, None);
        }
        // Multiplying by 2^-4 is bit-identical to dividing by 16.0.
        let cx = (check.x * 0.0625).floor() as i32;
        let csi = ((check.y - min_y_f) * 0.0625).floor() as i32;
        let cz = (check.z * 0.0625).floor() as i32;
        if !grid.visited(cx, csi, cz) {
            return (true, Some((cx, csi, cz)));
        }
    }
    (false, None)
}

mod walk;

pub use walk::{AsyncOcclusionWalker, EmptyColumns};

/// Per-section visibility from a meshed snapshot's opacity grid, for the walk
/// above. `snapshot` reads the section's own 16³ blocks.
pub fn section_visibility(
    snapshot: &crate::mesher::MeshSnapshot,
    baked: &haste_assets::BakedBlockModels,
    pos: SectionPos,
) -> VisibilitySet {
    let origin = pos.block_min();
    compute_visibility(|x, y, z| {
        let state = snapshot.block_state(origin.add(x as i32, y as i32, z as i32));
        baked.is_full_opaque(state)
    })
}

#[cfg(test)]
mod tests;
