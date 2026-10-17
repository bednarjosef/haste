//! Fluid face culling against neighbouring block shapes, after `FluidRenderer.isFaceOccludedByState`
//! and `Shapes.blockOccludes`: a fluid face is dropped when the neighbour's occlusion shape
//! (its boxes touching the shared face) covers the whole fluid face.
//!
//! Boxes are `[min_x, min_y, min_z, max_x, max_y, max_z]` in block units. The fluid itself is
//! the box `(0, 0, 0)..(1, height, 1)`, as in vanilla.

use haste_core::Direction;

/// A neighbour's face occlusion shape (`BlockState.getFaceOcclusionShape`).
#[derive(Clone, Copy, Debug)]
pub enum Occluder<'a> {
    /// `Shapes.empty()`: never occludes.
    Empty,
    /// `Shapes.block()`: the full cube.
    Full,
    /// Any other shape, as its boxes (slabs, stairs, ...).
    Boxes(&'a [[f32; 6]]),
}

/// Coordinates equal within vanilla's tolerance (`DoubleMath.fuzzyEquals(.., 1.0E-7)`).
fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1.0e-7
}

/// `FluidRenderer.isFaceOccludedByState`: is the fluid face `dir` of a fluid `height` high
/// hidden by a neighbour with occlusion shape `occluder`?
pub fn is_face_occluded(dir: Direction, height: f32, occluder: Occluder<'_>) -> bool {
    match occluder {
        Occluder::Empty => false,
        Occluder::Full => dir != Direction::Up || height == 1.0,
        Occluder::Boxes(boxes) => block_occludes(height, boxes, dir),
    }
}

/// `Shapes.blockOccludes(box(0,0,0,1,height,1), occluder, dir)`: the fluid box must reach
/// the shared face along `dir`'s axis, the occluder must touch it from the other side, and
/// the occluder's boxes on that face must cover the fluid's face completely.
fn block_occludes(height: f32, boxes: &[[f32; 6]], dir: Direction) -> bool {
    if boxes.is_empty() {
        return false;
    }
    let (axis, positive) = axis_of(dir);
    // The fluid box spans 0..1 on x and z and 0..height on y.
    let fluid_max = if axis == 1 { height } else { 1.0 };
    // The face the fluid presents: its extent on the two other axes.
    let (u_axis, v_axis) = others(axis);
    let fluid_face = [0.0, 0.0, extent(height, u_axis), extent(height, v_axis)];
    if positive {
        // Fluid is "first": it must reach 1 on the axis; the occluder must start at 0.
        if !near(fluid_max, 1.0) || !near(min_on(boxes, axis), 0.0) {
            return false;
        }
        covered(fluid_face, boxes, |b| near(b[axis], 0.0), u_axis, v_axis)
    } else {
        // Occluder is "first": it must reach 1 on the axis; the fluid starts at 0 always.
        if !near(max_on(boxes, axis), 1.0) {
            return false;
        }
        covered(
            fluid_face,
            boxes,
            |b| near(b[axis + 3], 1.0),
            u_axis,
            v_axis,
        )
    }
}

fn axis_of(dir: Direction) -> (usize, bool) {
    match dir {
        Direction::West => (0, false),
        Direction::East => (0, true),
        Direction::Down => (1, false),
        Direction::Up => (1, true),
        Direction::North => (2, false),
        Direction::South => (2, true),
    }
}

fn others(axis: usize) -> (usize, usize) {
    match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    }
}

fn extent(height: f32, axis: usize) -> f32 {
    if axis == 1 {
        height
    } else {
        1.0
    }
}

fn min_on(boxes: &[[f32; 6]], axis: usize) -> f32 {
    boxes.iter().map(|b| b[axis]).fold(f32::INFINITY, f32::min)
}

fn max_on(boxes: &[[f32; 6]], axis: usize) -> f32 {
    boxes
        .iter()
        .map(|b| b[axis + 3])
        .fold(f32::NEG_INFINITY, f32::max)
}

/// Largest number of boxes a neighbour shape may have for exact coverage testing; shapes with
/// more boxes never occlude (a stair has at most a handful).
const MAX_BOXES: usize = 16;

/// Whether the rectangles of the boxes selected by `on_face` (projected on the `u`/`v` axes)
/// cover `face` = `[u0, v0, u1, v1]` completely. Exact for axis-aligned rectangles: every cell
/// of the grid spanned by all rectangle edges is tested at its centre. No allocation.
fn covered(
    face: [f32; 4],
    boxes: &[[f32; 6]],
    on_face: impl Fn(&[f32; 6]) -> bool,
    u: usize,
    v: usize,
) -> bool {
    let [u0, v0, u1, v1] = face;
    if u1 <= u0 || v1 <= v0 {
        return true;
    }
    let mut rects = [[0.0f32; 4]; MAX_BOXES];
    let mut n = 0;
    for b in boxes.iter().filter(|b| on_face(b)) {
        if n == MAX_BOXES {
            return false;
        }
        rects[n] = [b[u], b[v], b[u + 3], b[v + 3]];
        n += 1;
    }
    let rects = &rects[..n];
    let mut us = [0.0f32; 2 * MAX_BOXES + 2];
    let mut vs = [0.0f32; 2 * MAX_BOXES + 2];
    let nu = edges(&mut us, rects, 0, u0, u1);
    let nv = edges(&mut vs, rects, 1, v0, v1);
    for i in 0..nu - 1 {
        let cu = 0.5 * (us[i] + us[i + 1]);
        for j in 0..nv - 1 {
            let cv = 0.5 * (vs[j] + vs[j + 1]);
            let inside = rects
                .iter()
                .any(|r| r[0] <= cu && cu <= r[2] && r[1] <= cv && cv <= r[3]);
            if !inside {
                return false;
            }
        }
    }
    true
}

/// Sorted, deduplicated edge coordinates of `rects` on axis `k` (0 = u, 1 = v) clamped to
/// `lo..hi`, with `lo` and `hi` included. Returns the count written.
fn edges(out: &mut [f32], rects: &[[f32; 4]], k: usize, lo: f32, hi: f32) -> usize {
    let mut n = 0;
    out[n] = lo;
    n += 1;
    out[n] = hi;
    n += 1;
    for r in rects {
        for c in [r[k], r[k + 2]] {
            if c > lo && c < hi {
                out[n] = c;
                n += 1;
            }
        }
    }
    let slice = &mut out[..n];
    slice.sort_by(|a, b| a.total_cmp(b));
    // Dedup in place.
    let mut w = 1;
    for r in 1..n {
        if slice[r] != slice[w - 1] {
            slice[w] = slice[r];
            w += 1;
        }
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTTOM_SLAB: [[f32; 6]; 1] = [[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]];
    const TOP_SLAB: [[f32; 6]; 1] = [[0.0, 0.5, 0.0, 1.0, 1.0, 1.0]];
    // Stairs facing east, bottom half: slab plus the raised east half.
    const STAIRS_EAST: [[f32; 6]; 2] = [
        [0.0, 0.0, 0.0, 1.0, 0.5, 1.0],
        [0.5, 0.5, 0.0, 1.0, 1.0, 1.0],
    ];

    #[test]
    fn full_block_hides_sides_and_bottom_but_not_a_low_top() {
        assert!(is_face_occluded(Direction::North, 0.5, Occluder::Full));
        assert!(is_face_occluded(Direction::Down, 8.0 / 9.0, Occluder::Full));
        assert!(!is_face_occluded(Direction::Up, 8.0 / 9.0, Occluder::Full));
        assert!(is_face_occluded(Direction::Up, 1.0, Occluder::Full));
        assert!(!is_face_occluded(Direction::East, 1.0, Occluder::Empty));
    }

    #[test]
    fn bottom_slab_hides_a_fluid_no_higher_than_it() {
        // Water at level 7 (height 1/9) next to a bottom slab: hidden on every side.
        for dir in [
            Direction::North,
            Direction::South,
            Direction::West,
            Direction::East,
        ] {
            assert!(is_face_occluded(
                dir,
                1.0 / 9.0,
                Occluder::Boxes(&BOTTOM_SLAB)
            ));
            assert!(is_face_occluded(dir, 0.5, Occluder::Boxes(&BOTTOM_SLAB)));
            assert!(!is_face_occluded(
                dir,
                8.0 / 9.0,
                Occluder::Boxes(&BOTTOM_SLAB)
            ));
            // A top slab leaves the lower part of the face open.
            assert!(!is_face_occluded(dir, 0.5, Occluder::Boxes(&TOP_SLAB)));
        }
        // Below the fluid a top slab covers the whole bottom face, a bottom slab does not.
        assert!(is_face_occluded(
            Direction::Down,
            8.0 / 9.0,
            Occluder::Boxes(&TOP_SLAB)
        ));
        assert!(!is_face_occluded(
            Direction::Down,
            8.0 / 9.0,
            Occluder::Boxes(&BOTTOM_SLAB)
        ));
        // Above the fluid a partial shape never hides a surface lower than the block.
        assert!(!is_face_occluded(
            Direction::Up,
            8.0 / 9.0,
            Occluder::Boxes(&BOTTOM_SLAB)
        ));
    }

    #[test]
    fn stairs_hide_only_their_closed_side() {
        // The stair's east face is full height; its west face only half.
        assert!(is_face_occluded(
            Direction::West,
            1.0,
            Occluder::Boxes(&STAIRS_EAST)
        ));
        assert!(!is_face_occluded(
            Direction::East,
            1.0,
            Occluder::Boxes(&STAIRS_EAST)
        ));
        assert!(is_face_occluded(
            Direction::East,
            0.5,
            Occluder::Boxes(&STAIRS_EAST)
        ));
        // North/south faces of the stair are L-shaped: two boxes together cover a half-high
        // fluid but not a full one.
        assert!(is_face_occluded(
            Direction::North,
            0.5,
            Occluder::Boxes(&STAIRS_EAST)
        ));
        assert!(!is_face_occluded(
            Direction::North,
            1.0,
            Occluder::Boxes(&STAIRS_EAST)
        ));
    }
}
