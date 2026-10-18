//! Shape-based light occlusion, vanilla `LightEngine.shapeOccludes` /
//! `Shapes.faceShapeOccludes` over `BlockStateBase`'s cached `occlusionShapesByFace`.
//!
//! A state's occlusion shape gates light per face when both `can_occlude` and
//! `use_shape_for_light_occlusion` hold (vanilla `LightEngine.isEmptyShape`): a bottom
//! slab blocks the light crossing its bottom face but not its open top, stairs block on
//! their solid sides. Both flags come per state from the oracle dump; dumps from before
//! they were recorded answer `false` everywhere, which is the behaviour the light engine
//! had before shapes (only light dampening decides).
//!
//! The face shapes are the occlusion shape's boxes touching the shared plane
//! (`VoxelShape.getFaceShape`'s slice, whose normal-axis coordinates re-normalize to
//! 0..1, so vanilla's 3D union-covers-cube test is exactly the 2D comparison of the two
//! faces' rectangles on the plane), classified per state like the terrain culling in
//! `haste-render`'s `mesher/face_occlusion.rs` (which this module can later replace).
//! Everything is precomputed at load — one class per (state, direction) and one flat
//! rectangle arena — so a light step is two table lookups plus a small fixed-size
//! rectangle test, never an allocation.

use std::collections::HashMap;

use haste_core::{Aabb, DVec3, Direction};

use super::OracleState;

/// Per-state occlusion shapes (vanilla `BlockState.getOcclusionShape`, what face culling
/// and light occlusion compare against: empty for the `noOcclusion` family, half boxes for
/// slabs, the smaller shapes of fences, fence gates, lecterns and sculk shriekers). The
/// boxes of all states are interned into one arena (most states share the full cube or
/// nothing), so no state carries its own `Vec`.
#[derive(Default)]
pub(super) struct OcclusionTable {
    pub(super) boxes: Vec<Aabb>,
    /// Per state: its slice of `boxes`, or [`OCCLUSION_ABSENT`].
    pub(super) ranges: Vec<std::ops::Range<u32>>,
}

/// `ranges` entry for a state the dump has no occlusion shape for (an empty range is a
/// real shape: glass occludes nothing).
pub(super) const OCCLUSION_ABSENT: std::ops::Range<u32> = u32::MAX..u32::MAX;

impl OcclusionTable {
    /// The state's occlusion boxes, or `None` when the dump predates the field.
    pub(super) fn boxes_of(&self, row: usize) -> Option<&[Aabb]> {
        let range = self.ranges.get(row)?;
        if range == &OCCLUSION_ABSENT {
            return None;
        }
        Some(&self.boxes[range.start as usize..range.end as usize])
    }

    /// Interns one state's boxes into the arena and points its range at them. Identical
    /// shapes (bit-exact after the dump's 1e-6 rounding) share one arena entry; `intern`
    /// is the caller-owned map from the box bit patterns to arena starts.
    pub(super) fn intern(
        &mut self,
        id: u32,
        list: &[[f64; 6]],
        intern: &mut HashMap<Vec<[u64; 6]>, u32>,
    ) {
        if id as usize >= self.ranges.len() {
            // out of range: parse_oracle's main loop rejects the dump for this
            return;
        }
        // Floats are not hashable: intern on the bit patterns (exact).
        let key: Vec<[u64; 6]> = list.iter().map(|b| b.map(f64::to_bits)).collect();
        let start = match intern.get(&key) {
            Some(&i) => i,
            None => {
                let i = self.boxes.len() as u32;
                self.boxes.extend(list.iter().map(|b| Aabb {
                    min: DVec3::new(b[0], b[1], b[2]),
                    max: DVec3::new(b[3], b[4], b[5]),
                }));
                intern.insert(key, i);
                i
            }
        };
        self.ranges[id as usize] = start..start + list.len() as u32;
    }
}

/// The light engines' shape test (vanilla `LightEngine.isEmptyShape` / `shapeOccludes`):
/// per (state, direction) the face's occlusion shape class, precomputed at load. The
/// `can_occlude` / `use_shape_for_light_occlusion` flags gate it exactly like vanilla
/// (`getOcclusionShape` answers empty for a state failing either).
pub(super) struct FaceOcclusion {
    /// Per state and direction (`state * 6 + the direction's ordinal`), the face's class.
    classes: Vec<FaceClass>,
    /// All `Rects` rectangles, back to back; the classes index into this.
    rects: Vec<[f32; 4]>,
    /// `can_occlude` per state: 0 false, 1 true, [`FLAG_ABSENT`] without the field.
    can_occlude: Vec<u8>,
    /// `use_shape_for_light_occlusion` per state, same encoding.
    use_shape: Vec<u8>,
}

/// `can_occlude` / `use_shape_for_light_occlusion` value for a state the dump's flags are
/// absent for (a dump from before they were recorded).
const FLAG_ABSENT: u8 = 2;

/// A face's occlusion shape on its plane (vanilla `getFaceOcclusionShape`).
#[derive(Clone, Copy, PartialEq)]
enum FaceClass {
    /// The dump predates the flags (or the `occlusion` field): no shape test, light
    /// passes like it did before shapes.
    Unknown,
    /// `Shapes.empty()`: no box touches the plane.
    Empty,
    /// `Shapes.block()`: the touching boxes cover the whole face.
    Full,
    /// The touching boxes as rectangles on the plane.
    Rects { start: u32, len: u32 },
}

impl FaceOcclusion {
    /// No shape data at all (the fallback table, tests without the cache): every lookup
    /// answers "empty shape" and `shape_occludes` is false.
    pub(super) fn absent() -> FaceOcclusion {
        FaceOcclusion {
            classes: Vec::new(),
            rects: Vec::new(),
            can_occlude: Vec::new(),
            use_shape: Vec::new(),
        }
    }

    /// Classifies every state that carries both flags and an occlusion shape. Identical
    /// occlusion shapes classify once (most states share the full cube or nothing).
    pub(super) fn build(
        occlusion: &OcclusionTable,
        states: &[OracleState],
        state_count: usize,
    ) -> FaceOcclusion {
        let mut out = FaceOcclusion {
            classes: vec![FaceClass::Unknown; state_count * 6],
            rects: Vec::new(),
            can_occlude: vec![FLAG_ABSENT; state_count],
            use_shape: vec![FLAG_ABSENT; state_count],
        };
        let mut intern: HashMap<Vec<[u64; 6]>, [FaceClass; 6]> = HashMap::with_capacity(128);
        for s in states {
            let (Some(can), Some(use_shape)) = (s.can_occlude, s.use_shape_for_light_occlusion)
            else {
                continue;
            };
            let idx = s.id as usize;
            out.can_occlude[idx] = u8::from(can);
            out.use_shape[idx] = u8::from(use_shape);
            // vanilla `isEmptyShape`: a state failing either flag occludes nothing by
            // shape, whatever its occlusion shape is.
            if !can || !use_shape {
                out.classes[idx * 6..idx * 6 + 6].fill(FaceClass::Empty);
                continue;
            }
            let Some(boxes) = occlusion.boxes_of(idx) else {
                continue;
            };
            let key: Vec<[u64; 6]> = boxes
                .iter()
                .map(|b| {
                    [
                        b.min.x.to_bits(),
                        b.min.y.to_bits(),
                        b.min.z.to_bits(),
                        b.max.x.to_bits(),
                        b.max.y.to_bits(),
                        b.max.z.to_bits(),
                    ]
                })
                .collect();
            let faces = match intern.get(&key) {
                Some(f) => *f,
                None => {
                    let f = face_set(boxes, &mut out.rects);
                    intern.insert(key, f);
                    f
                }
            };
            out.classes[idx * 6..idx * 6 + 6].copy_from_slice(&faces);
        }
        out
    }

    /// Vanilla `LightEngine.shapeOccludes(fromState, toState, direction)`: whether the
    /// two states' face occlusion shapes block light travelling `from -> to` along
    /// `direction` (`Shapes.faceShapeOccludes` between `from`'s face in `direction` and
    /// `to`'s face in the opposite). Rows come from `BlockShapes::row` (unknown ids read
    /// the placeholder's entry, never a panic).
    pub(super) fn occludes(&self, from: usize, to: usize, direction: Direction) -> bool {
        let from = self.class(from, direction);
        let to = self.class(to, direction.opposite());
        match (from, to) {
            // A dump without the flags: no shape test, exactly the pre-shape behaviour.
            (FaceClass::Unknown, _) | (_, FaceClass::Unknown) => false,
            // Vanilla's `shape == block() || occluder == block()` shortcut.
            (FaceClass::Full, _) | (_, FaceClass::Full) => true,
            (FaceClass::Empty, FaceClass::Empty) => false,
            (a, b) => self.union_covers_plane(a, b),
        }
    }

    /// Vanilla `LightEngine.isEmptyShape`: `!canOcclude || !useShapeForLightOcclusion`.
    /// Without the flags (an old dump) every state counts as empty, which is what the
    /// pre-shape engine assumed.
    pub(super) fn is_empty_shape(&self, row: usize) -> bool {
        let can = matches!(self.can_occlude.get(row), Some(&1));
        let use_shape = matches!(self.use_shape.get(row), Some(&1));
        !can || !use_shape
    }

    /// Vanilla `BlockState.useShapeForLightOcclusion`, `None` when the dump predates the
    /// flags (`LightEngine.hasDifferentLightProperties` keeps its approximation then).
    pub(super) fn use_shape(&self, row: usize) -> Option<bool> {
        match self.use_shape.get(row) {
            Some(&0) => Some(false),
            Some(&1) => Some(true),
            _ => None,
        }
    }

    #[inline]
    fn class(&self, row: usize, dir: Direction) -> FaceClass {
        // A row past the table (the fallback table, or an unknown state without a
        // placeholder hit): Unknown, never a panic.
        self.classes
            .get(row * 6 + dir as usize)
            .copied()
            .unwrap_or(FaceClass::Unknown)
    }

    fn rect_slice(&self, class: FaceClass) -> &[[f32; 4]] {
        match class {
            FaceClass::Rects { start, len } => &self.rects[start as usize..(start + len) as usize],
            _ => &[],
        }
    }

    /// Whether the two faces' rectangles together cover the whole plane (vanilla
    /// `Shapes.faceShapeOccludes`: the union of both shapes covers the block). Exact for
    /// axis-aligned rectangles with the cell-grid test; fixed-size buffers, no allocation.
    fn union_covers_plane(&self, a: FaceClass, b: FaceClass) -> bool {
        let (a, b) = (self.rect_slice(a), self.rect_slice(b));
        const MAX_BREAKPOINTS: usize = 24;
        let mut us = [0.0f32; MAX_BREAKPOINTS];
        let mut vs = [0.0f32; MAX_BREAKPOINTS];
        // 0 and 1 always present, so the whole plane is cut into cells.
        us[1] = 1.0;
        vs[1] = 1.0;
        let (mut un, mut vn) = (2, 2);
        for rects in [a, b] {
            for &[bu0, bv0, bu1, bv1] in rects {
                for (axis_coords, coords, n) in [
                    (&mut us, [bu0, bu1], &mut un),
                    (&mut vs, [bv0, bv1], &mut vn),
                ] {
                    for c in coords {
                        if axis_coords[..*n].iter().all(|&t| (t - c).abs() > 1e-4) {
                            if *n == MAX_BREAKPOINTS {
                                // More breakpoints than real occlusion shapes have (a
                                // slab is one box, stairs three): let the light through
                                // rather than guess.
                                return false;
                            }
                            axis_coords[*n] = c;
                            *n += 1;
                        }
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
                if !a.iter().any(|r| inside(u, v, *r)) && !b.iter().any(|r| inside(u, v, *r)) {
                    return false;
                }
            }
        }
        true
    }
}

/// One state's six face classifications from its occlusion boxes (vanilla
/// `BlockStateBase.initCache`: `occlusionShapesByFace`), by direction ordinal.
fn face_set(boxes: &[Aabb], rects: &mut Vec<[f32; 4]>) -> [FaceClass; 6] {
    // A full-cube occlusion shape is solid render: every face is the full block.
    if is_full_cube(boxes) {
        return [FaceClass::Full; 6];
    }
    let mut faces = [FaceClass::Empty; 6];
    for dir in Direction::ALL {
        let n = normal_axis(dir);
        let positive = dir.offset()[n] > 0;
        // A box touches the shared plane when it reaches the face's side of the cell.
        let reaches = |b: &Aabb| {
            near(
                if positive {
                    axis(b.max, n)
                } else {
                    axis(b.min, n)
                },
                if positive { 1.0 } else { 0.0 },
            )
        };
        if !boxes.iter().any(reaches) {
            continue;
        }
        let (u, v) = tangent_axes(dir);
        // The slice is cube-like (vanilla returns `Shapes.block()`) when the merged
        // shape's coordinates on both tangent axes are exactly 0..1: every box spans the
        // full face, so the touching ones cover it.
        let full_face = boxes.iter().all(|b| {
            near(axis(b.min, u), 0.0)
                && near(axis(b.max, u), 1.0)
                && near(axis(b.min, v), 0.0)
                && near(axis(b.max, v), 1.0)
        });
        faces[dir as usize] = if full_face {
            FaceClass::Full
        } else {
            let start = rects.len() as u32;
            for b in boxes {
                if reaches(b) {
                    rects.push([
                        axis(b.min, u) as f32,
                        axis(b.min, v) as f32,
                        axis(b.max, u) as f32,
                        axis(b.max, v) as f32,
                    ]);
                }
            }
            FaceClass::Rects {
                start,
                len: rects.len() as u32 - start,
            }
        };
    }
    faces
}

/// The world axis `direction` points along.
fn normal_axis(direction: Direction) -> usize {
    let o = direction.offset();
    (0..3).find(|&i| o[i] != 0).unwrap()
}

/// The face plane's two tangent axes (world axis indices, ascending): the 2D frame both
/// compared faces are measured in — the same world axes for a direction and its opposite.
fn tangent_axes(direction: Direction) -> (usize, usize) {
    match normal_axis(direction) {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    }
}

fn axis(v: DVec3, i: usize) -> f64 {
    match i {
        0 => v.x,
        1 => v.y,
        _ => v.z,
    }
}

/// Dump coordinates are rounded to 1e-6, so plane tests need that much slack (vanilla
/// compares at 1e-7 on unrounded doubles).
fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6
}

/// `Block.isShapeFullBlock` on an occlusion shape: the boxes fill the whole unit cube
/// (a full cube is overwhelmingly the single-box case, checked first).
fn is_full_cube(boxes: &[Aabb]) -> bool {
    if boxes.len() == 1
        && near(axis(boxes[0].min, 0), 0.0)
        && near(axis(boxes[0].min, 1), 0.0)
        && near(axis(boxes[0].min, 2), 0.0)
        && near(axis(boxes[0].max, 0), 1.0)
        && near(axis(boxes[0].max, 1), 1.0)
        && near(axis(boxes[0].max, 2), 1.0)
    {
        return true;
    }
    let mut volume = 0.0;
    for b in boxes {
        for i in 0..3 {
            if axis(b.min, i) < -1e-9 || axis(b.max, i) > 1.0 + 1e-9 {
                return false;
            }
        }
        let d = b.max - b.min;
        volume += d.x * d.y * d.z;
    }
    (volume - 1.0).abs() < 1e-6
}

#[cfg(test)]
mod tests;
