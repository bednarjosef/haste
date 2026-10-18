//! Face culling by occlusion coverage (vanilla `Block.shouldRenderFace`).
//!
//! Vanilla culls a model face in three steps: the neighbour's face occlusion
//! shape being the full block, the block's own `skipRendering`, and the shape
//! comparison — the face renders only where its own occlusion shape pokes out
//! beyond the neighbour's (`Shapes.joinIsNotEmpty(shape, occluder, ONLY_FIRST)`).
//! Haste's mesher had only the first rule's model equivalent
//! (`BakedBlockModels::is_full_opaque`), so faces between two partial blocks
//! that cover each other exactly were drawn: snow layers against snow layers
//! drew every interior side face, which shows as thin dark lines along the
//! block edges from a distance (user bug "terrain seams").
//!
//! This module carries the shape comparison, read off the baked models: a
//! state's face-plane coverage is the union of its opaque, axis-aligned quads
//! flush with that face. That is visually exact — where the neighbour's opaque
//! geometry covers the shared plane, the face cannot be seen from either side
//! (from one side the covering quads are front-facing and opaque, from the
//! other the culled face itself is back-facing) — and it follows vanilla for
//! every block whose model matches its shape (snow layers, slabs, full cubes).
//! Vanilla's data is the block's occlusion *shape*, not its model; where the
//! two disagree haste's rule decides by what is actually drawn, and the
//! non-flush faces (an arm reaching into the neighbour's cell) stay
//! conservative and render.

use haste_assets::{AtlasRegions, BakedQuad, Direction};

/// The quad's extent on `direction`'s face plane, as `[u0, v0, u1, v1]` in the
/// plane's tangent axes, when the quad is a rectangle axis-aligned on the
/// plane and flush with the cell boundary there (the plane a cullface sits
/// on). Quads that tilt or sit off the plane contribute no coverage: `None`.
pub(super) fn face_box(direction: Direction, positions: &[[f32; 3]; 4]) -> Option<[f32; 4]> {
    let n = normal_axis(direction);
    let (a, b) = tangent_axes(direction);
    let plane = if direction.offset()[n] > 0 { 1.0 } else { 0.0 };
    if positions.iter().any(|p| (p[n] - plane).abs() > 1e-4) {
        return None;
    }
    let (mut u0, mut u1, mut v0, mut v1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for p in positions {
        u0 = u0.min(p[a]);
        u1 = u1.max(p[a]);
        v0 = v0.min(p[b]);
        v1 = v1.max(p[b]);
    }
    // The quad fills its extent rectangle: each corner sits on a distinct
    // (u, v) corner of the extent (a tilted or inset quad does not).
    let near = |t: f32, lo: f32, hi: f32| (t - lo).abs() <= 1e-4 || (t - hi).abs() <= 1e-4;
    let mut corners = [false; 4];
    for p in positions {
        if !near(p[a], u0, u1) || !near(p[b], v0, v1) {
            return None;
        }
        let cu = usize::from((p[a] - u1).abs() <= 1e-4);
        let cv = usize::from((p[b] - v1).abs() <= 1e-4);
        corners[cu * 2 + cv] = true;
    }
    if !corners.iter().all(|&c| c) {
        return None;
    }
    Some([u0, v0, u1, v1])
}

/// A state's face-plane occlusion coverage: one box per opaque, axis-aligned
/// quad flush with its cullface's plane, grouped by direction. Built once per
/// block state with the state's other meshing info.
pub(super) struct FaceCover {
    /// The boxes of all six directions, concatenated; `ranges` slices them.
    boxes: Vec<[f32; 4]>,
    /// Per direction (its `Direction::index`), the box range in `boxes`.
    ranges: [(usize, usize); 6],
}

impl FaceCover {
    /// The coverage of `state`'s baked quads under `atlas` (opaque textures).
    pub(super) fn from_quads(quads: &[BakedQuad], atlas: &dyn AtlasRegions) -> FaceCover {
        let mut boxes = Vec::new();
        let mut ranges = [(0, 0); 6];
        for (dir_i, range) in ranges.iter_mut().enumerate() {
            let start = boxes.len();
            for quad in quads {
                let Some(cull) = quad.cullface else { continue };
                if cull.index() != dir_i || !atlas.region_of(&quad.texture).opaque {
                    continue;
                }
                if let Some(box2) = face_box(cull, &quad.positions) {
                    boxes.push(box2);
                }
            }
            *range = (start, boxes.len());
        }
        FaceCover { boxes, ranges }
    }

    /// Whether the quad `positions` (carrying `cull` as its cullface) is fully
    /// covered by this state's coverage on the shared plane: the neighbour
    /// this state sits at occludes it. A quad that is not a flush
    /// axis-aligned rectangle is never covered (it renders).
    pub(super) fn covers(&self, cull: Direction, positions: &[[f32; 3]; 4]) -> bool {
        let (start, end) = self.ranges[cull.opposite().index()];
        if start == end {
            return false;
        }
        match face_box(cull, positions) {
            Some(region) => union_covers(&self.boxes[start..end], region),
            None => false,
        }
    }
}

/// Whether the union of `boxes` contains the whole rectangle `region`
/// (vanilla `Shapes.joinIsNotEmpty(own, occluder, ONLY_FIRST)` being empty).
/// Exact for axis-aligned boxes: the boxes' edges cut the plane into cells
/// that each lie fully inside or outside every box, so testing cell centres
/// decides every cell. Allocations are avoided on this per-quad path: up to
/// `MAX_BREAKPOINTS` distinct coordinates per axis, more boxes than that
/// simply render.
pub(super) fn union_covers(boxes: &[[f32; 4]], region: [f32; 4]) -> bool {
    const MAX_BREAKPOINTS: usize = 10;
    let [ru0, rv0, ru1, rv1] = region;
    let mut us = [0.0; MAX_BREAKPOINTS];
    let mut vs = [0.0; MAX_BREAKPOINTS];
    us[0] = ru0;
    us[1] = ru1;
    vs[0] = rv0;
    vs[1] = rv1;
    let (mut un, mut vn) = (2, 2);
    for [bu0, bv0, bu1, bv1] in boxes {
        for (axis, coords, n) in [
            (&mut us, [bu0, bu1], &mut un),
            (&mut vs, [bv0, bv1], &mut vn),
        ] {
            for c in coords {
                if axis[..*n].iter().all(|&t| (t - c).abs() > 1e-4) {
                    if *n == MAX_BREAKPOINTS {
                        return false; // too many boxes to decide exactly: render
                    }
                    axis[*n] = *c;
                    *n += 1;
                }
            }
        }
    }
    us[..un].sort_by(|x, y| x.partial_cmp(y).unwrap());
    vs[..vn].sort_by(|x, y| x.partial_cmp(y).unwrap());
    let inside = |u: f32, v: f32, [bu0, bv0, bu1, bv1]: [f32; 4]| {
        u >= bu0 - 1e-4 && u <= bu1 + 1e-4 && v >= bv0 - 1e-4 && v <= bv1 + 1e-4
    };
    for iu in 0..un - 1 {
        for iv in 0..vn - 1 {
            let (u, v) = ((us[iu] + us[iu + 1]) * 0.5, (vs[iv] + vs[iv + 1]) * 0.5);
            if inside(u, v, region) && !boxes.iter().any(|b| inside(u, v, *b)) {
                return false;
            }
        }
    }
    true
}

/// The world axis `direction` points along.
fn normal_axis(direction: Direction) -> usize {
    let o = direction.offset();
    (0..3).find(|&i| o[i] != 0).unwrap()
}

/// The face plane's two tangent axes (world axis indices, ascending): the 2D
/// frame both the culled quad and the neighbour's occluder boxes are measured
/// in — the same world axes for a direction and its opposite.
fn tangent_axes(direction: Direction) -> (usize, usize) {
    match normal_axis(direction) {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    }
}
