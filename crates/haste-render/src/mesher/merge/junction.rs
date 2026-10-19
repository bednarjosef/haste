//! Junction marks: the positions where a neighbouring quad's corner lies on
//! a merged rectangle's edge (see `merge.rs` for why such a T-junction must
//! be split). One set per section, collected from the merge rectangles' and
//! the staged direct quads' corners, bucketed once and looked up per edge.
//!
//! A mark is a corner position, filed under the line it lies on: a line
//! along one axis is fixed on the other two. Every line a rectangle edge can
//! lie on has one fixed coordinate at an integer 0..=16 (the edge's tangent
//! cell coordinate) and one at the face plane (a cell plus a model height,
//! usually an integer; fractional for snow layers and slabs). So a line is
//! `(direction, integer axis, integer value, plane)` — a small
//! direct-addressed key, and the marks are counting-sorted into those
//! buckets (a comparison sort measurably dominated meshing: a section
//! carries thousands of marks).
//!
//! One geometric line carries edges of two face families that name it with
//! the integer and the plane swapped (a top rectangle's edge along x at
//! `(z = V, y = π)` and a north side rectangle's edge along x at
//! `(y = π, z = V)` are the same line). Marks are filed under the filing
//! edge's own naming and looked up under both, so every family finds them.
//! Marks are matched by f32 bits: a mark on a rectangle's edge line sits
//! bit-exactly where the neighbour's vertex is.
//!
//! All positions are section-local and non-negative, so f32 bit order is
//! value order — used to sort the few in-span split positions of one edge.

use super::{MergeRect, StagedQuad};

/// The two axes a line along `d` is fixed on, ascending.
fn other_axes(d: usize) -> (usize, usize) {
    match d {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    }
}

/// The plane's bucket slot: integer planes 0..=16 by value, all fractional
/// planes (snow layers, slabs) share one overflow slot and are told apart by
/// their f32 bits at lookup.
fn plane_slot(plane: f32) -> usize {
    let p = plane as i32;
    if p as f32 == plane && (0..=16).contains(&p) {
        p as usize
    } else {
        17
    }
}

/// `((direction * 3 + integer axis) * 17 + integer value) * 18 + plane slot`.
fn bucket_of(d: usize, int_axis: usize, int_val: i32, plane: f32) -> usize {
    ((d * 3 + int_axis) * 17 + int_val as usize) * 18 + plane_slot(plane)
}

const N_BUCKETS: usize = 3 * 3 * 17 * 18;

/// The most in-span split positions one edge can carry (integer marks on a
/// 16-long edge plus fractional ones from partial model quads; beyond this,
/// pathological cases degrade to unsplit — no worse than vanilla).
pub(super) const MAX_SPLITS: usize = 64;

/// One rectangle edge's line: along axis `d`, fixed at the tangent cell
/// coordinate `t_val` on `t_axis` and at the face plane `plane` on `n_axis`.
pub(super) struct EdgeLine {
    pub(super) d: usize,
    pub(super) t_axis: usize,
    pub(super) t_val: i32,
    pub(super) n_axis: usize,
    pub(super) plane: f32,
}

/// The boundary ring of the rectangle being emitted: absolute section-local
/// position plus sprite-local UV per vertex.
pub(super) type Ring = Vec<([f32; 3], [f32; 2])>;

/// A rectangle edge's marks, ready for lookup: the counting-sorted marks and
/// the per-bucket offsets.
pub(super) struct MarksView<'a> {
    sorted: &'a [(u32, u32)],
    offsets: &'a [u32],
}

impl MarksView<'_> {
    /// The split positions on the line `(d, tangent axis t_axis, tangent
    /// value t_val, plane on n_axis)` strictly inside the open span
    /// `(lo, hi)`, ascending. Both namings of the line are read (see the
    /// module comment); the plane's f32 bits must match bit-exactly.
    /// `short` says the span is under two units wide, so no integer position
    /// fits strictly inside — then only a fractional plane's marks are
    /// worth reading (positions are compared as bit patterns: all are
    /// non-negative, where bit order is value order).
    pub(super) fn splits_in(
        &self,
        line: &EdgeLine,
        lo: f32,
        hi: f32,
        short: bool,
        out: &mut [f32; MAX_SPLITS],
    ) -> usize {
        let EdgeLine {
            d,
            t_axis,
            t_val,
            n_axis,
            plane,
        } = *line;
        let mut n = 0;
        let (lo_bits, hi_bits) = (lo.to_bits(), hi.to_bits());
        let mut scan = |bucket: usize, plane_bits: u32, sorted: &[(u32, u32)], offsets: &[u32]| {
            for &(pb, posb) in &sorted[offsets[bucket] as usize..offsets[bucket + 1] as usize] {
                if pb != plane_bits || posb <= lo_bits || posb >= hi_bits {
                    continue;
                }
                if n < MAX_SPLITS {
                    out[n] = f32::from_bits(posb);
                    n += 1;
                }
            }
        };
        let pi = plane as i32;
        let plane_is_int = pi as f32 == plane && (0..=16).contains(&pi);
        if !short || !plane_is_int {
            scan(
                bucket_of(d, t_axis, t_val, plane),
                plane.to_bits(),
                self.sorted,
                self.offsets,
            );
            // The same line as named by the other family: the plane becomes
            // the integer (only when it is one) and the tangent value the
            // plane.
            if plane_is_int {
                scan(
                    bucket_of(d, n_axis, pi, t_val as f32),
                    (t_val as f32).to_bits(),
                    self.sorted,
                    self.offsets,
                );
            }
        }
        out[..n].sort_unstable_by(|a, b| a.total_cmp(b));
        // Two neighbours can mark the same position (a side face and its
        // tinted overlay both carry corners there): one split vertex each.
        // The array is fixed-size, so compact in place over the sorted run.
        let mut m = 0;
        for i in 0..n {
            if m == 0 || out[m - 1] != out[i] {
                out[m] = out[i];
                m += 1;
            }
        }
        m
    }
}

/// Per-section scratch: the pending and counting-sorted marks and the
/// boundary ring of the rectangle being emitted. Everything keeps its
/// capacities between sections (the staged quads, runs and rectangles are
/// scratch too; see `merge.rs`).
pub(crate) struct JunctionScratch {
    /// Marks awaiting the counting sort: (bucket, plane bits, position bits).
    pending: Vec<(usize, u32, u32)>,
    /// The counting-sorted marks as (plane bits, position bits), with
    /// `offsets` slicing them per bucket.
    sorted: Vec<(u32, u32)>,
    offsets: Vec<u32>,
    /// The ring of the rectangle being emitted.
    ring: Ring,
}

impl JunctionScratch {
    pub(crate) fn new() -> JunctionScratch {
        JunctionScratch {
            pending: Vec::new(),
            sorted: Vec::new(),
            offsets: vec![0; N_BUCKETS + 1],
            ring: Vec::new(),
        }
    }

    /// The marks (shared) and the ring (mutable) together: `emit_rect` reads
    /// the marks while it extends the ring.
    pub(super) fn split(&mut self) -> (MarksView<'_>, &mut Ring) {
        (
            MarksView {
                sorted: &self.sorted,
                offsets: &self.offsets,
            },
            &mut self.ring,
        )
    }

    /// Collects this section's marks and counting-sorts them into buckets:
    /// every rectangle edge files its two corner positions (so any edge on
    /// the same line finds them), and every staged direct quad's corners
    /// file under the first naming that fits (any position: a stair's
    /// half-cell corner files at 0.5, in the fractional plane slot).
    pub(super) fn collect(&mut self, rects: &[MergeRect], staged: &[StagedQuad]) {
        self.pending.clear();
        for r in rects {
            let f = &r.key.face;
            let (ua, va) = (f.u_axis as usize, f.v_axis as usize);
            let na = 3 - ua - va;
            let height = f32::from_bits(f.plane).fract();
            let plane = r.base[na] as f32 + height;
            // The two edges along u (at the low and high v) and the two
            // along v, each filing its two corner positions.
            let (u0, u1) = (r.base[ua] as f32, (r.base[ua] + r.len_u) as f32);
            let (v0, v1) = (r.base[va] as f32, (r.base[va] + r.len_v) as f32);
            self.add_edge(ua, va, r.base[va], plane, u0, u1);
            self.add_edge(ua, va, r.base[va] + r.len_v, plane, u0, u1);
            self.add_edge(va, ua, r.base[ua], plane, v0, v1);
            self.add_edge(va, ua, r.base[ua] + r.len_u, plane, v0, v1);
        }
        for q in staged {
            if let StagedQuad::Direct { positions, .. } = q {
                for &p in positions {
                    self.add_direct_corner(p);
                }
            }
        }
        self.sort_marks();
    }

    /// One edge's two corner positions along the edge's direction, on the
    /// line `(d, t_axis, t_val, plane)`.
    fn add_edge(
        &mut self,
        d: usize,
        t_axis: usize,
        t_val: i32,
        plane: f32,
        pos_lo: f32,
        pos_hi: f32,
    ) {
        let b = bucket_of(d, t_axis, t_val, plane);
        let plane_bits = plane.to_bits();
        self.pending.push((b, plane_bits, pos_lo.to_bits()));
        self.pending.push((b, plane_bits, pos_hi.to_bits()));
    }

    /// Files a direct quad's corner under the first line naming that fits:
    /// for each direction, whichever of the line's two fixed coordinates is
    /// the integer (the other is the plane).
    fn add_direct_corner(&mut self, p: [f32; 3]) {
        for d in 0..3 {
            let (a, b) = other_axes(d);
            let (pa, pb) = (p[a], p[b]);
            let ia = pa as i32;
            let ib = pb as i32;
            if ia as f32 == pa && (0..=16).contains(&ia) {
                self.add(d, a, ia, pb, p[d]);
            } else if ib as f32 == pb && (0..=16).contains(&ib) {
                self.add(d, b, ib, pa, p[d]);
            }
        }
    }

    fn add(&mut self, d: usize, int_axis: usize, int_val: i32, plane: f32, pos: f32) {
        self.pending.push((
            bucket_of(d, int_axis, int_val, plane),
            plane.to_bits(),
            pos.to_bits(),
        ));
    }

    /// Counting sort: count per bucket, prefix-sum into offsets, then place
    /// through a cursor array (within-bucket order does not matter).
    fn sort_marks(&mut self) {
        self.offsets.fill(0);
        for &(b, _, _) in &self.pending {
            self.offsets[b + 1] += 1;
        }
        for i in 1..=N_BUCKETS {
            self.offsets[i] += self.offsets[i - 1];
        }
        self.sorted.clear();
        self.sorted.resize(self.pending.len(), (0, 0));
        let mut cursor = [0u32; N_BUCKETS];
        cursor.copy_from_slice(&self.offsets[..N_BUCKETS]);
        for &(b, plane, pos) in &self.pending {
            let at = cursor[b] as usize;
            self.sorted[at] = (plane, pos);
            cursor[b] += 1;
        }
    }
}

/// The sprite-local UV of a ring vertex at tangent-axis offsets (off_u,
/// off_v) from the rectangle's low corner: a component that follows the u
/// signs is the offset (or `len - offset` when flipped), so integer offsets
/// tile the sprite per block — the wrap the fragment shader already does.
pub(super) fn ring_uv(
    f: &super::MergeFace,
    off_u: f32,
    off_v: f32,
    len_u: i32,
    len_v: i32,
) -> [f32; 2] {
    let mut uv = [0f32; 2];
    for (uv_c, [src, flip]) in uv.iter_mut().zip(f.uv_map) {
        let (off, len) = if src == 0 {
            (off_u, len_u as f32)
        } else {
            (off_v, len_v as f32)
        };
        *uv_c = if flip == 0 { off } else { len - off };
    }
    uv
}
