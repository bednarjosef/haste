//! Terrain face culling from the oracle dump's occlusion shapes, vanilla
//! `Block.shouldRenderFace` — the per-direction gate `ModelBlockRenderer` runs before
//! emitting any of a face's quads. A face is hidden when the neighbour's face occlusion
//! shape is the full block, shown when either side's face shape is empty, and otherwise
//! shown exactly where the state's own face shape pokes out beyond the neighbour's
//! (`Shapes.joinIsNotEmpty(shape, occluder, BooleanOp.ONLY_FIRST)`).
//!
//! The face shapes are the occlusion shape's boxes touching the shared plane
//! (`VoxelShape.getFaceShape`'s slice, whose normal-axis coordinates re-normalize to
//! 0..1, so the join is the 2D comparison on the plane), classified per state like
//! vanilla `BlockStateBase.initCache`'s cached `occlusionShapesByFace`. Sets are interned
//! by occlusion shape: most states share the full cube or nothing, so the per-state cost
//! is one index.
//!
//! `Block.shouldRenderFace`'s second gate, `BlockBehaviour.skipRendering` (same-block
//! faces: glass against glass, leaves, iron bars), is not modelled here: it needs
//! per-block class knowledge the dump does not carry. The model rule the mesher falls
//! back to (a full-opaque neighbour, then the neighbour's opaque quads covering the
//! quad's extent, `mesher/culling.rs`) already hides most of those.
//!
//! Dumps from before the 2026-10-18 `occlusion` field answer `None` from
//! [`FaceOccluders::should_render_face`] for those states, and the mesher keeps the model
//! rule. That rule can diverge both ways (a fence's model arms cover a neighbouring
//! fence's arm face while vanilla's smaller fence occlusion shape shows it; a partial
//! model whose occlusion shape is full hides nothing in vanilla); the occlusion-shape
//! rule matches vanilla exactly for every state the dump covers.

use std::collections::HashMap;

use haste_assets::Direction;
use haste_core::view::BlockStateTable;
use haste_core::{Aabb, BlockStateId, DVec3};

use super::culling::union_covers;

/// Per-state face-occlusion shapes for terrain culling (vanilla
/// `Block.shouldRenderFace`'s shape comparison). Built once per connection from
/// haste-data's occlusion shapes; no allocation per block or quad.
#[derive(Default)]
pub struct FaceOccluders {
    /// Per state: index into `sets`, or [`NO_SET`] — the dump predates the occlusion
    /// field, or the id is past the table (a modded server can send those; they fall
    /// back to the model rule instead of panicking).
    states: Vec<u32>,
    sets: Vec<FaceSet>,
    /// All `Rects` rectangles, back to back; the sets index into this.
    rects: Vec<[f32; 4]>,
}

const NO_SET: u32 = u32::MAX;

/// One state's six face classifications (by `Direction::index`).
#[derive(Clone, Copy)]
struct FaceSet {
    faces: [FaceClass; 6],
}

/// A face's occlusion shape on its plane (vanilla `getFaceOcclusionShape`).
#[derive(Clone, Copy)]
enum FaceClass {
    /// `Shapes.empty()`: no box touches the plane.
    Empty,
    /// `Shapes.block()`: the merged shape's coordinates on both tangent axes are exactly
    /// 0..1 (the slice is cube-like), so the touching boxes cover the whole face.
    Full,
    /// The touching boxes as rectangles on the plane.
    Rects { start: u32, len: u32 },
}

/// Dump coordinates are rounded to 1e-6, so plane tests need that much slack (vanilla
/// compares at 1e-7 on unrounded doubles).
fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6
}

fn axis(v: DVec3, i: usize) -> f64 {
    match i {
        0 => v.x,
        1 => v.y,
        _ => v.z,
    }
}

impl FaceOccluders {
    /// From haste-data's shape data (oracle-backed when the dump exists). States without
    /// the occlusion field keep [`NO_SET`] and answer `None` below.
    pub fn from_shapes(
        states: &dyn BlockStateTable,
        shapes: &haste_data::BlockShapes,
    ) -> FaceOccluders {
        let mut out = FaceOccluders {
            states: vec![NO_SET; states.len()],
            ..Default::default()
        };
        // Intern by the shape's bit pattern: identical shapes (the vast majority —
        // full cubes, empty) share one FaceSet.
        let mut intern: HashMap<Vec<[u64; 6]>, u32> = HashMap::with_capacity(128);
        for id in 0..states.len() as u32 {
            let Some(boxes) = shapes.occlusion_boxes(BlockStateId(id)) else {
                continue;
            };
            let key: Vec<[u64; 6]> = boxes
                .iter()
                .map(|b| {
                    [
                        axis(b.min, 0).to_bits(),
                        axis(b.min, 1).to_bits(),
                        axis(b.min, 2).to_bits(),
                        axis(b.max, 0).to_bits(),
                        axis(b.max, 1).to_bits(),
                        axis(b.max, 2).to_bits(),
                    ]
                })
                .collect();
            let set = match intern.get(&key) {
                Some(&i) => i,
                None => {
                    let i = out.sets.len() as u32;
                    out.sets.push(face_set(boxes, &mut out.rects));
                    intern.insert(key, i);
                    i
                }
            };
            out.states[id as usize] = set;
        }
        out
    }

    /// No occlusion data at all (tests, before the game data is installed): every lookup
    /// answers `None` and the mesher's model rule applies.
    pub fn absent() -> FaceOccluders {
        FaceOccluders::default()
    }

    fn face(&self, state: BlockStateId, dir: Direction) -> Option<FaceClass> {
        let set = self.states.get(state.0 as usize).copied()?;
        if set == NO_SET {
            return None;
        }
        Some(self.sets[set as usize].faces[dir.index()])
    }

    fn rect_slice(&self, class: FaceClass) -> &[[f32; 4]] {
        match class {
            FaceClass::Rects { start, len } => &self.rects[start as usize..(start + len) as usize],
            _ => &[],
        }
    }

    /// Vanilla `Block.shouldRenderFace(state, neighbor, direction)`: whether the face
    /// renders against `neighbor`. `None` when either state lacks occlusion data — the
    /// caller falls back to its model rule.
    pub fn should_render_face(
        &self,
        state: BlockStateId,
        neighbor: BlockStateId,
        dir: Direction,
    ) -> Option<bool> {
        let own = self.face(state, dir)?;
        let occluder = self.face(neighbor, dir.opposite())?;
        Some(match occluder {
            // occluder == Shapes.block(): hidden whatever the own shape is.
            FaceClass::Full => false,
            // occluder == Shapes.empty(): nothing to hide behind.
            FaceClass::Empty => true,
            FaceClass::Rects { .. } => match own {
                // The own face shape is empty (the shape does not reach the plane).
                FaceClass::Empty => true,
                FaceClass::Full => !union_covers(self.rect_slice(occluder), [0.0, 0.0, 1.0, 1.0]),
                FaceClass::Rects { .. } => {
                    !covers_union(self.rect_slice(occluder), self.rect_slice(own))
                }
            },
        })
    }
}

/// One state's six face classifications from its occlusion boxes (vanilla
/// `BlockStateBase.initCache`: `occlusionShapesByFace`).
fn face_set(boxes: &[Aabb], rects: &mut Vec<[f32; 4]>) -> FaceSet {
    // A full-cube occlusion shape is solid render: every face is the full block.
    if is_full_cube(boxes) {
        return FaceSet {
            faces: [FaceClass::Full; 6],
        };
    }
    let mut faces = [FaceClass::Empty; 6];
    for dir in [
        Direction::Down,
        Direction::Up,
        Direction::North,
        Direction::South,
        Direction::West,
        Direction::East,
    ] {
        let n = super::culling::normal_axis(dir);
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
        let (u, v) = super::culling::tangent_axes(dir);
        // The slice is cube-like (vanilla returns `Shapes.block()`) when the merged
        // shape's coordinates on both tangent axes are exactly 0..1: every box spans the
        // full face, so the touching ones cover it.
        let full_face = boxes.iter().all(|b| {
            near(axis(b.min, u), 0.0)
                && near(axis(b.max, u), 1.0)
                && near(axis(b.min, v), 0.0)
                && near(axis(b.max, v), 1.0)
        });
        faces[dir.index()] = if full_face {
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
    FaceSet { faces }
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

/// Whether the union of `cover` contains the union of `covered` (vanilla
/// `Shapes.joinIsNotEmpty(own, occluder, ONLY_FIRST)` being empty: the neighbour's face
/// shape covers the state's own everywhere). Exact for axis-aligned rectangles with the
/// cell-grid test `union_covers` uses; too many boxes to decide exactly means show.
fn covers_union(cover: &[[f32; 4]], covered: &[[f32; 4]]) -> bool {
    const MAX_BREAKPOINTS: usize = 12;
    let mut us = [0.0f32; MAX_BREAKPOINTS];
    let mut vs = [0.0f32; MAX_BREAKPOINTS];
    let (mut un, mut vn) = (0, 0);
    for boxes in [cover, covered] {
        for [bu0, bv0, bu1, bv1] in boxes {
            for (axis_coords, coords, n) in [
                (&mut us, [bu0, bu1], &mut un),
                (&mut vs, [bv0, bv1], &mut vn),
            ] {
                for c in coords {
                    if axis_coords[..*n].iter().all(|&t| (t - c).abs() > 1e-4) {
                        if *n == MAX_BREAKPOINTS {
                            return false;
                        }
                        axis_coords[*n] = *c;
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
            if covered.iter().any(|b| inside(u, v, *b)) && !cover.iter().any(|b| inside(u, v, *b)) {
                return false;
            }
        }
    }
    true
}
