// This file is derived from Pomme (https://github.com/josefbednar/pomme),
// pomme-client/src/renderer/chunk/occlusion_graph.rs, reused under GPL-3.0
// (vanilla-style section visibility for cave culling: VisGraph flood fill +
// SectionOcclusionGraph walk). See THIRD_PARTY.md.

use std::collections::VecDeque;
use std::sync::Arc;
use std::thread::JoinHandle;

use ahash::AHashMap;
use crossbeam_channel::{unbounded, Receiver, Sender};
use glam::DVec3;
use haste_core::{ChunkPos, SectionPos};
use parking_lot::Mutex;

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

/// The sections the walk reached, per column, over the square of columns around the camera.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VisibleMask {
    cx: i32,
    cz: i32,
    r: i32,
    columns: Vec<u32>,
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
/// section. Sections without visibility data are see-through, so the result
/// only ever under-culls. Frustum culling is applied separately. Flat arrays
/// instead of hash maps: a full walk at render distance 12 visits ~15k nodes.
#[allow(clippy::too_many_arguments)]
pub fn compute_visible_mask(
    section_vis: impl Fn(ChunkPos, i32) -> Option<VisibilitySet>,
    cam_col: ChunkPos,
    cam_si: i32,
    eye: DVec3,
    min_y: i32,
    section_count: i32,
    render_distance: i32,
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
        let source = grid.source[i];
        let cone = grid.cone[i] & !VISITED;
        let vis = section_vis(ChunkPos::new(x, z), si).unwrap_or(VisibilitySet::all());
        let distant = (x - cam_col.x).abs() > ADV_CULL_SECTION_DIST
            || (si - cam_si).abs() > ADV_CULL_SECTION_DIST
            || (z - cam_col.z).abs() > ADV_CULL_SECTION_DIST;
        // The ray march depends only on this section, not on the face; compute it
        // once per pop. Visits only accumulate during the pop, so a cached
        // "blocked at section q" stays blocked while q is unvisited, and a
        // cached "clear" stays clear; if q was reached in between, re-march.
        let mut ray: Option<(bool, (i32, i32, i32))> = None;
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
            // Distant sections also get a coarse ray-march back to the camera.
            if distant {
                let blocked = match ray {
                    None => {
                        let (b, q) = ray_occluded(
                            &grid,
                            eye,
                            cam_center,
                            (x, si, z),
                            min_y_f,
                            world_min_y,
                            world_max_y,
                        );
                        ray = Some((b, q.unwrap_or((0, 0, 0))));
                        b
                    }
                    Some((false, _)) => false,
                    Some((true, q)) => {
                        if grid.visited(q.0, q.1, q.2) {
                            let (b, q) = ray_occluded(
                                &grid,
                                eye,
                                cam_center,
                                (x, si, z),
                                min_y_f,
                                world_min_y,
                                world_max_y,
                            );
                            ray = Some((b, q.unwrap_or((0, 0, 0))));
                            b
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
        columns,
    }
}

/// Step from the distant section's corner facing the camera toward the camera
/// by one section diagonal at a time; if a step lands in a section the walk
/// hasn't reached, the line of sight is blocked. Returns whether the sightline
/// is blocked and the first unvisited section along it (`None` when clear).
fn ray_occluded(
    grid: &Grid,
    eye: DVec3,
    cam_center: DVec3,
    (sx, ssi, sz): (i32, i32, i32),
    min_y_f: f64,
    world_min_y: f64,
    world_max_y: f64,
) -> (bool, Option<(i32, i32, i32)>) {
    let ox = (sx * 16) as f64;
    let oy = min_y_f + (ssi * 16) as f64;
    let oz = (sz * 16) as f64;
    let corner = |c: f64, o: f64| o + if c > o { 16.0 } else { 0.0 };
    let mut check = DVec3::new(
        corner(cam_center.x, ox),
        corner(cam_center.y, oy),
        corner(cam_center.z, oz),
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

/// Snapshot the shared section visibility sets into a dense grid for the walk:
/// the walk reads each visited section's set once, and a slice index beats a
/// hash probe per node. Sections without an entry are see-through (`all`).
pub fn dense_visibility_grid(
    vis: &AHashMap<(ChunkPos, i32), VisibilitySet>,
    cam: ChunkPos,
    r: i32,
    h: i32,
) -> Vec<VisibilitySet> {
    let w = 2 * r + 1;
    let mut dense = vec![VisibilitySet::all(); (w * w * h) as usize];
    for ((c, si), v) in vis.iter() {
        let dx = c.x - cam.x + r;
        let dz = c.z - cam.z + r;
        if dx < 0 || dz < 0 || dx >= w || dz >= w || *si < 0 || *si >= h {
            continue;
        }
        dense[((dx * w + dz) * h + *si) as usize] = *v;
    }
    dense
}

/// One walk request: the camera section and render distance the mask is
/// computed for, plus the world vertical extent and the eye position. Runs on
/// the worker: snapshot the visibility sets under the shared lock, then walk.
#[derive(Clone, Copy)]
struct WalkJob {
    cam_section: SectionPos,
    view_distance: i32,
    section_count: i32,
    min_section: i32,
    eye: DVec3,
}

impl WalkJob {
    fn run(&self, vis: &Mutex<AHashMap<(ChunkPos, i32), VisibilitySet>>) -> VisibleMask {
        let r = self.view_distance + 1;
        let h = self.section_count;
        let cam = ChunkPos::new(self.cam_section.x, self.cam_section.z);
        let dense = dense_visibility_grid(&vis.lock(), cam, r, h);
        let w = 2 * r + 1;
        compute_visible_mask(
            |c, si| {
                let dx = c.x - cam.x + r;
                let dz = c.z - cam.z + r;
                if dx < 0 || dz < 0 || dx >= w || dz >= w || si < 0 || si >= h {
                    return None;
                }
                Some(dense[((dx * w + dz) * h + si) as usize])
            },
            cam,
            self.cam_section.y - self.min_section,
            self.eye,
            self.min_section * 16,
            h,
            r,
        )
    }
}

/// The section visibility walk, off the render thread. Vanilla runs the full
/// `SectionOcclusionGraph` update on a background executor and keeps drawing
/// the previous graph until the new one is swapped in; this does the same: the
/// render thread only schedules a walk (`schedule`) and collects the finished
/// mask (`poll`), never walking itself. One dedicated worker thread runs the
/// walks serially, so at most one is in flight.
pub struct AsyncOcclusionWalker {
    done_rx: Receiver<VisibleMask>,
    job_tx: Option<Sender<WalkJob>>,
    worker: Option<JoinHandle<()>>,
}

impl AsyncOcclusionWalker {
    /// Shares `vis` with the worker: the render thread inserts on upload, the
    /// worker snapshots under the lock when a walk starts.
    pub fn new(vis: Arc<Mutex<AHashMap<(ChunkPos, i32), VisibilitySet>>>) -> std::io::Result<Self> {
        let (job_tx, job_rx) = unbounded::<WalkJob>();
        let (done_tx, done_rx) = unbounded::<VisibleMask>();
        let vis_worker = vis;
        let worker = std::thread::Builder::new()
            .name("haste-occlusion".into())
            .spawn(move || {
                while let Ok(job) = job_rx.recv() {
                    let mask = job.run(&vis_worker);
                    if done_tx.send(mask).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            done_rx,
            job_tx: Some(job_tx),
            worker: Some(worker),
        })
    }

    /// Ask the worker to walk for `cam_section` / `view_distance`. Cheap: a
    /// channel send; the render thread never walks.
    pub fn schedule(
        &self,
        cam_section: SectionPos,
        view_distance: i32,
        section_count: i32,
        min_section: i32,
        eye: DVec3,
    ) {
        let job = WalkJob {
            cam_section,
            view_distance,
            section_count,
            min_section,
            eye,
        };
        if let Some(tx) = &self.job_tx {
            let _ = tx.send(job);
        }
    }

    /// Collect the newest finished mask, if any. Non-blocking: a walk that is
    /// not done yet simply keeps the previous mask.
    pub fn poll(&self) -> Option<VisibleMask> {
        let mut latest = None;
        while let Ok(mask) = self.done_rx.try_recv() {
            latest = Some(mask);
        }
        latest
    }
}

impl Drop for AsyncOcclusionWalker {
    fn drop(&mut self) {
        // Drop the request channel first so the worker exits; join it.
        self.job_tx.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

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
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn empty_section_connects_all_faces() {
        let v = compute_visibility(|_, _, _| false);
        for a in Face::ALL {
            for b in Face::ALL {
                assert!(v.visible_between(a, b));
            }
        }
    }

    #[test]
    fn solid_section_connects_nothing() {
        let v = compute_visibility(|_, _, _| true);
        for a in Face::ALL {
            for b in Face::ALL {
                assert!(!v.visible_between(a, b));
            }
        }
    }

    #[test]
    fn wall_splits_west_from_east() {
        // Full YZ plane at x=8 (256 cells, so the flood path runs).
        let v = compute_visibility(|x, _, _| x == 8);
        assert!(!v.visible_between(Face::West, Face::East));
        assert!(v.visible_between(Face::West, Face::Up));
        assert!(v.visible_between(Face::East, Face::Up));
    }

    #[test]
    fn open_grid_reaches_every_section() {
        // No visibility data => every section is see-through; the walk must
        // reach every section within render distance.
        let section_vis: HashMap<(ChunkPos, i32), VisibilitySet> = HashMap::new();
        let sc = 8;
        let rd = 6;
        let eye = DVec3::new(8.0, 72.0, 8.0);
        let mask = compute_visible_mask(
            |c, si| section_vis.get(&(c, si)).copied(),
            ChunkPos::new(0, 0),
            4,
            eye,
            0,
            sc,
            rd,
        );
        let full = (1u32 << sc) - 1;
        for x in -rd..=rd {
            for z in -rd..=rd {
                assert_eq!(
                    mask.column(ChunkPos::new(x, z)),
                    full,
                    "column ({x}, {z}) not fully reached"
                );
            }
        }
    }

    #[test]
    fn solid_wall_hides_sections_behind_it() {
        let mut section_vis = HashMap::new();
        section_vis.insert((ChunkPos::new(0, -1), 4), VisibilitySet::none());
        let eye = DVec3::new(8.0, 72.0, 8.0);
        let mask = compute_visible_mask(
            |c, si| section_vis.get(&(c, si)).copied(),
            ChunkPos::new(0, 0),
            4,
            eye,
            0,
            8,
            4,
        );
        // The wall section itself is reached (visible face)...
        assert!(mask.column(ChunkPos::new(0, -1)) & (1 << 4) != 0);
        // ...but the section directly behind it on the same row is not.
        assert!(mask.column(ChunkPos::new(0, -2)) & (1 << 4) == 0);
    }

    /// The walk before the culling optimizations: per-face ray march, the
    /// 6-iteration `any` sightline check and `/16.0` section indexing. The
    /// optimized walk must produce the identical mask; this reference is the
    /// oracle the differential test compares against.
    fn reference_visible_mask(
        section_vis: &HashMap<(ChunkPos, i32), VisibilitySet>,
        cam_col: ChunkPos,
        cam_si: i32,
        eye: DVec3,
        min_y: i32,
        section_count: i32,
        render_distance: i32,
    ) -> VisibleMask {
        let r = render_distance.max(0);
        let w = 2 * r + 1;
        let h = section_count.max(1);
        let n = (w * w * h) as usize;
        let mut source = vec![0u8; n];
        let mut cone = vec![0u8; n];
        let mut columns = vec![0u32; (w * w) as usize];
        let mut queue: VecDeque<(i32, i32, i32)> = VecDeque::new();
        let idx = |x: i32, si: i32, z: i32| -> Option<usize> {
            let (dx, dz) = (x - cam_col.x + r, z - cam_col.z + r);
            if dx < 0 || dz < 0 || dx >= w || dz >= w || si < 0 || si >= h {
                return None;
            }
            Some(((dx * w + dz) * h + si) as usize)
        };
        let cam_center = DVec3::new(
            (cam_col.x * 16 + 8) as f64,
            (min_y + cam_si * 16 + 8) as f64,
            (cam_col.z * 16 + 8) as f64,
        );
        let start = (cam_col.x, cam_si.clamp(0, h - 1), cam_col.z);
        cone[idx(start.0, start.1, start.2).unwrap()] = VISITED;
        columns[(r * w + r) as usize] |= 1u32 << start.1;
        queue.push_back(start);
        while let Some((x, si, z)) = queue.pop_front() {
            let i = idx(x, si, z).unwrap();
            let src = source[i];
            let c = cone[i] & !VISITED;
            let vis = section_vis
                .get(&(ChunkPos::new(x, z), si))
                .copied()
                .unwrap_or(VisibilitySet::all());
            let distant = (x - cam_col.x).abs() > ADV_CULL_SECTION_DIST
                || (si - cam_si).abs() > ADV_CULL_SECTION_DIST
                || (z - cam_col.z).abs() > ADV_CULL_SECTION_DIST;
            for face in Face::ALL {
                if c & face.opposite().bit() != 0 {
                    continue;
                }
                if src != 0
                    && !Face::ALL
                        .into_iter()
                        .any(|sf| src & sf.bit() != 0 && vis.visible_between(sf.opposite(), face))
                {
                    continue;
                }
                let (dx, dy, dz) = face.offset();
                let (nx, nsi, nz) = (x + dx, si + dy, z + dz);
                if distant
                    && reference_ray_occluded(
                        &cone,
                        &idx,
                        eye,
                        cam_center,
                        (x, si, z),
                        min_y,
                        section_count,
                    )
                {
                    continue;
                }
                let Some(ni) = idx(nx, nsi, nz) else {
                    continue;
                };
                if cone[ni] & VISITED != 0 {
                    source[ni] |= face.bit();
                    continue;
                }
                source[ni] = face.bit();
                cone[ni] = VISITED | c | face.bit();
                let col = ((nx - cam_col.x + r) * w + (nz - cam_col.z + r)) as usize;
                columns[col] |= 1u32 << nsi;
                queue.push_back((nx, nsi, nz));
            }
        }
        VisibleMask {
            cx: cam_col.x,
            cz: cam_col.z,
            r,
            columns,
        }
    }

    fn reference_ray_occluded(
        cone: &[u8],
        idx: &impl Fn(i32, i32, i32) -> Option<usize>,
        eye: DVec3,
        cam_center: DVec3,
        (sx, ssi, sz): (i32, i32, i32),
        min_y: i32,
        section_count: i32,
    ) -> bool {
        let ox = (sx * 16) as f64;
        let oy = (min_y + ssi * 16) as f64;
        let oz = (sz * 16) as f64;
        let corner = |c: f64, o: f64| o + if c > o { 16.0 } else { 0.0 };
        let mut check = DVec3::new(
            corner(cam_center.x, ox),
            corner(cam_center.y, oy),
            corner(cam_center.z, oz),
        );
        let step = (eye - check).normalize() * CEILED_SECTION_DIAGONAL;
        let world_min_y = min_y as f64;
        let world_max_y = (min_y + section_count * 16) as f64;
        while check.distance_squared(eye) > ADV_CULL_MIN_DIST_SQ {
            check += step;
            if check.y > world_max_y || check.y < world_min_y {
                return false;
            }
            let cx = (check.x / 16.0).floor() as i32;
            let csi = ((check.y - min_y as f64) / 16.0).floor() as i32;
            let cz = (check.z / 16.0).floor() as i32;
            let blocked = idx(cx, csi, cz).is_none_or(|i| cone[i] & VISITED == 0);
            if blocked {
                return true;
            }
        }
        false
    }

    /// Deterministic LCG so the differential test needs no dependencies.
    fn lcg(state: &mut u64) -> u64 {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *state >> 33
    }

    #[test]
    fn optimized_walk_matches_reference() {
        for seed in 1..=40u64 {
            let mut rng = seed;
            let sc = 4 + (lcg(&mut rng) % 5) as i32; // 4..=8 sections tall
            let rd = 2 + (lcg(&mut rng) % 4) as i32; // render distance 2..=5
            let w = 2 * rd + 1;
            let mut section_vis = HashMap::new();
            for x in -rd..=rd {
                for z in -rd..=rd {
                    for si in 0..sc {
                        let roll = lcg(&mut rng) % 10;
                        let vis = match roll {
                            0 => VisibilitySet::none(),
                            1 => VisibilitySet::all(),
                            _ => {
                                // Symmetric like every set `add` builds.
                                let bits = lcg(&mut rng) & ((1u64 << 36) - 1);
                                let mut mirrored = 0u64;
                                for a in 0..6u32 {
                                    for b in 0..6u32 {
                                        if bits >> (a * 6 + b) & 1 != 0 {
                                            mirrored |= 1u64 << (b * 6 + a);
                                        }
                                    }
                                }
                                VisibilitySet(bits | mirrored)
                            }
                        };
                        section_vis.insert((ChunkPos::new(x, z), si), vis);
                    }
                }
            }
            let cam_x = lcg(&mut rng) as i32 % w - rd;
            let cam_z = lcg(&mut rng) as i32 % w - rd;
            let cam_si = lcg(&mut rng) as i32 % sc;
            let eye = DVec3::new(
                (lcg(&mut rng) % 400) as f64 - 100.0,
                (lcg(&mut rng) % 200) as f64,
                (lcg(&mut rng) % 400) as f64 - 100.0,
            );
            let min_y = -8 + (lcg(&mut rng) % 16) as i32;
            let fast = compute_visible_mask(
                |c, si| section_vis.get(&(c, si)).copied(),
                ChunkPos::new(cam_x, cam_z),
                cam_si,
                eye,
                min_y,
                sc,
                rd,
            );
            let slow = reference_visible_mask(
                &section_vis,
                ChunkPos::new(cam_x, cam_z),
                cam_si,
                eye,
                min_y,
                sc,
                rd,
            );
            for x in -rd..=rd {
                for z in -rd..=rd {
                    assert_eq!(
                        fast.column(ChunkPos::new(x, z)),
                        slow.column(ChunkPos::new(x, z)),
                        "seed {seed}: column ({x}, {z}) differs"
                    );
                }
            }
        }
    }

    /// The worker's mask must equal the synchronous walk over the same data:
    /// the async path changes only where the walk runs, not what it computes.
    #[test]
    fn async_walk_matches_sync() {
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        let mut map = AHashMap::new();
        let mut state = 0x1234_5678_9abc_def0u64;
        let sc = 8i32;
        let vd = 6i32;
        for x in -vd..=vd {
            for z in -vd..=vd {
                for si in 0..sc {
                    let v = match lcg(&mut state) % 4 {
                        0 => VisibilitySet::none(),
                        1 => VisibilitySet::all(),
                        _ => VisibilitySet(lcg(&mut state) | (lcg(&mut state) << 32)),
                    };
                    map.insert((ChunkPos::new(x, z), si), v);
                }
            }
        }
        let vis = Arc::new(Mutex::new(map));
        let walker = AsyncOcclusionWalker::new(vis.clone()).unwrap();
        let cam = ChunkPos::new(1, -1);
        let eye = DVec3::new(20.0, 70.0, -10.0);
        walker.schedule(SectionPos::new(cam.x, 4, cam.z), vd, sc, 0, eye);
        let deadline = Instant::now() + Duration::from_secs(10);
        let mask = loop {
            if let Some(m) = walker.poll() {
                break m;
            }
            assert!(Instant::now() < deadline, "worker never finished the walk");
            std::thread::sleep(Duration::from_millis(1));
        };
        let sync = compute_visible_mask(
            |c, si| vis.lock().get(&(c, si)).copied(),
            cam,
            4,
            eye,
            0,
            sc,
            vd + 1,
        );
        assert_eq!(mask, sync);
    }

    /// The render thread must never run the walk: `schedule` (a channel send)
    /// and `poll` (a channel drain) cost microseconds even while a large walk is
    /// in flight on the worker. The walk itself is ~ms; if either call blocked
    /// on it, the per-call cost would be ~ms too.
    #[test]
    fn render_thread_never_walks() {
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        let mut map = AHashMap::new();
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let sc = 24i32;
        let vd = 32i32;
        for x in -vd..=vd {
            for z in -vd..=vd {
                for si in 0..sc {
                    let v = match lcg(&mut state) % 3 {
                        0 => VisibilitySet::none(),
                        1 => VisibilitySet::all(),
                        _ => VisibilitySet(lcg(&mut state) | (lcg(&mut state) << 32)),
                    };
                    map.insert((ChunkPos::new(x, z), si), v);
                }
            }
        }
        let vis = Arc::new(Mutex::new(map));
        let walker = AsyncOcclusionWalker::new(vis).unwrap();
        let eye = DVec3::new(8.0, 72.0, 8.0);

        let started = Instant::now();
        walker.schedule(SectionPos::new(0, 4, 0), vd, sc, -4, eye);
        let schedule_us = started.elapsed().as_secs_f64() * 1e6;
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut poll_calls = 0u32;
        let mut poll_total = Duration::ZERO;
        let mask = loop {
            let p = Instant::now();
            let m = walker.poll();
            poll_total += p.elapsed();
            poll_calls += 1;
            if let Some(m) = m {
                break m;
            }
            assert!(Instant::now() < deadline, "worker never finished");
            std::thread::sleep(Duration::from_micros(200));
        };
        let poll_us = poll_total.as_secs_f64() * 1e6 / poll_calls as f64;
        eprintln!("schedule: {schedule_us:.2} us, poll: {poll_us:.2} us/call ({poll_calls} calls)");
        assert!(schedule_us < 500.0, "schedule blocked the render thread");
        assert!(poll_us < 500.0, "poll blocked the render thread");
        assert!(
            mask.column(ChunkPos::new(0, 0)) != 0,
            "worker walked an empty mask"
        );
    }
}
