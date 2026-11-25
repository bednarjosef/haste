// This file is derived from Pomme (https://github.com/josefbednar/pomme),
// pomme-client/src/renderer/chunk/mesher.rs, reused under GPL-3.0 and reworked
// for Haste (the greedy merge itself is new). See docs/THIRD_PARTY.md.

use super::lighting::face_axes;
use super::sink::{layer_from_index, MeshSink};
use std::sync::atomic::{AtomicBool, Ordering};

mod junction;

pub(super) use junction::JunctionScratch;
use junction::{ring_uv, EdgeLine, MAX_SPLITS};

/// Debug switch for the merged-quad crack tests: greedy merging off, so every
/// face meshes as vanilla does (one quad per block face) and the two meshes
/// can be rendered and compared pixel by pixel. Read per quad at staging time
/// (a relaxed atomic load; the quad loop does far more work per quad).
static GREEDY_MERGE: AtomicBool = AtomicBool::new(true);

/// See [`GREEDY_MERGE`]. Set before the scene's sections are (re)meshed.
pub fn set_greedy_merge_enabled(on: bool) {
    GREEDY_MERGE.store(on, Ordering::Relaxed);
}

pub(super) fn greedy_merge_enabled() -> bool {
    GREEDY_MERGE.load(Ordering::Relaxed)
}

/// The greedy-merge signature of a baked quad, if it has one: the positions
/// must be exactly the full unit face of `quad.direction` (the 0/1 corners on
/// the two tangent axes, the face plane on the normal axis) and the
/// sprite-local UVs exactly 0/1 at the corners. Such quads tile: extending
/// one along a tangent axis with the UV repeating keeps every fragment's
/// texel identical (the shader wraps sprite-local UVs with `fract`), so long
/// as the per-vertex attributes are constant across the merge (checked at
/// emit time: uniform light, one tint per quad).
pub(super) fn merge_face(quad: &haste_assets::BakedQuad, uvs: &[[f32; 2]; 4]) -> Option<MergeFace> {
    merge_face_geom(quad.direction, &quad.positions, uvs)
}

/// [`merge_face`] from the geometry alone: a quad on `dir`'s axis with block-local
/// `positions` (a fluid surface; the winding may face either way along the axis).
pub(super) fn merge_face_geom(
    dir: haste_assets::Direction,
    positions: &[[f32; 3]; 4],
    uvs: &[[f32; 2]; 4],
) -> Option<MergeFace> {
    let (u, v) = face_axes(dir);
    let n = dir.offset();
    let n_axis = n.iter().position(|&c| c != 0)?;
    // Per-vertex corner signs on (u, v) and the raw 0/1 UV corners.
    let mut corners = [[0u8, 0u8]; 4];
    let mut uv_corners = [[0u8, 0u8]; 4];
    // The face's plane on the normal axis (any height: a snow layer's top at
    // 2/16 merges with the same layer's neighbours). All four corners must
    // agree — a tilted quad is not a face.
    let mut plane: Option<f32> = None;
    for (i, (&pos, &uv)) in positions.iter().zip(uvs).enumerate() {
        match plane {
            None => plane = Some(pos[n_axis]),
            Some(p) if p != pos[n_axis] => return None,
            Some(_) => {}
        }
        let su = corner_sign(pos, u)?;
        let sv = corner_sign(pos, v)?;
        corners[i] = [su, sv];
        // The UV must sit exactly on a sprite corner (0/1), or extending the
        // quad would shift the texture.
        uv_corners[i] = [uv_flag(uv[0])?, uv_flag(uv[1])?];
    }
    // A degenerate pattern (two corners equal) is not a face.
    let degenerate = (0..4).any(|i| (0..4).any(|j| i != j && corners[i] == corners[j]));
    if degenerate {
        return None;
    }
    // Each UV component follows one corner sign (possibly flipped): find
    // which, so the merged quad's UVs extend along the same axis.
    let mut uv_map = [[0u8, 2]; 2];
    for c in 0..2 {
        let map = (0..2).find_map(|from| {
            let mut by_sign = [None; 2]; // sign 0, sign 1 -> uv value
            for (vi, &[su, sv]) in corners.iter().enumerate() {
                let s = [su, sv][from];
                let val = uv_corners[vi][c];
                if by_sign[s as usize].is_none() {
                    by_sign[s as usize] = Some(val);
                } else if by_sign[s as usize] != Some(val) {
                    return None;
                }
            }
            match (by_sign[0], by_sign[1]) {
                (Some(0), Some(1)) => Some([from as u8, 0u8]),
                (Some(1), Some(0)) => Some([from as u8, 1u8]),
                _ => None,
            }
        });
        uv_map[c] = map?;
    }
    Some(MergeFace {
        u_axis: axis_index(u)?,
        v_axis: axis_index(v)?,
        corners,
        uv_map,
        // The plane's model-space height, as f32 bits (grouping only).
        plane: plane?.to_bits(),
    })
}

/// Section-continuous UVs for a full unit face in block cell `cell` (section-local
/// 0..16): each component counts whole blocks along the axis it follows, so the face's
/// corners carry the same UVs as the touching corners of its coplanar neighbours.
/// Under the shader's `fract` wrap the texels are those of the 0/1 corner UVs (the
/// two differ by whole numbers, like a merged rectangle's repeats), and identical
/// corners let the sink share the vertex between the faces.
pub(super) fn section_uvs(face: &MergeFace, cell: [i32; 3]) -> [[f32; 2]; 4] {
    let mut uvs = [[0f32; 2]; 4];
    for (uv, corner) in uvs.iter_mut().zip(face.corners) {
        for (c, [src, flip]) in face.uv_map.into_iter().enumerate() {
            let axis = if src == 0 { face.u_axis } else { face.v_axis };
            let t = (cell[axis as usize] + i32::from(corner[src as usize])) as f32;
            uv[c] = if flip == 0 { t } else { 16.0 - t };
        }
    }
    uvs
}

/// A UV component as an exact 0/1 flag.
fn uv_flag(t: f32) -> Option<u8> {
    match t {
        0.0 => Some(0),
        1.0 => Some(1),
        _ => None,
    }
}

/// The 0/1 sign of `pos` on axis `axis` (position component exactly 0.0 or 1.0).
fn corner_sign(pos: [f32; 3], axis: [i32; 3]) -> Option<u8> {
    let i = axis_index(axis)? as usize;
    match pos[i] {
        0.0 => Some(0),
        1.0 => Some(1),
        _ => None,
    }
}

/// The nonzero axis of `axis` as 0/1/2.
fn axis_index(axis: [i32; 3]) -> Option<u8> {
    match axis {
        [1, 0, 0] => Some(0),
        [0, 1, 0] => Some(1),
        [0, 0, 1] => Some(2),
        _ => None,
    }
}

/// The greedy-merge geometry of a full unit face: the face's tangent axes,
/// each vertex's corner sign on them, and how each sprite-local UV component
/// follows the corner signs (which axis it tracks, flipped or not — vanilla's
/// per-face `FaceInfo` UV cycles are rotated/flipped differently per face).
/// A quad carrying one may be extended along either axis (UVs repeat: the
/// fragment shader wraps sprite-local UVs, `fract` in chunk.wgsl), with
/// per-vertex attributes that are constant across the merge (uniform light,
/// one tint) — the pixels stay identical.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct MergeFace {
    /// The face's tangent axes ([u, v]; see `face_axes`), as axis indices 0/1/2.
    pub(super) u_axis: u8,
    pub(super) v_axis: u8,
    /// Per-vertex corner signs (0 or 1) on (u, v), in the quad's vertex order.
    pub(super) corners: [[u8; 2]; 4],
    /// Per UV component: which corner sign drives it (0 = the u sign, 1 = the
    /// v sign) and whether it is flipped (uv = 1 - sign).
    pub(super) uv_map: [[u8; 2]; 2],
    /// The face plane's model-space coordinate on the normal axis, f32 bits.
    pub(super) plane: u32,
}

/// One quad between emission and the sink. Everything a section emits is staged
/// first because the greedy merge reorders quads within a merge group; the
/// flush emits direct quads at their original index and each merged rectangle
/// at its first member's index, so the relative order of everything that can
/// overlap coplanar stays as emitted (vanilla relies on model order for its
/// coplanar overlay quads).
#[derive(Clone)]
pub(super) enum StagedQuad {
    /// A quad that cannot merge (any shape, or per-vertex light differences).
    Direct {
        layer: haste_assets::RenderLayer,
        positions: [[f32; 3]; 4],
        uvs: [[f32; 2]; 4],
        lights: [[u8; 3]; 4],
        tint: u32,
        sprite: u16,
    },
    /// A full unit face with uniform light, waiting for the greedy pass.
    Merge(MergeQuad),
}

/// The merge group identity: two quads merge only when every pixel-affecting
/// attribute matches (layer, sprite, tint, uniform light, face geometry and
/// UV pattern — `face` covers direction, axes and per-vertex corner signs).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct MergeKey {
    pub(super) layer: u8,
    pub(super) dir: u8,
    pub(super) sprite: u16,
    pub(super) tint: u32,
    /// The quad's uniform light: [sky16, ao|shade, block16].
    pub(super) light: [u8; 3],
    pub(super) face: MergeFace,
}

#[derive(Clone)]
pub(super) struct MergeQuad {
    pub(super) key: MergeKey,
    /// Section-local block cell (0..16 on each axis).
    pub(super) cell: [u8; 3],
}

/// A merged rectangle of unit faces: `len_u` by `len_v` cells starting at the
/// block-cell corner (u0, v0) on the face's tangent axes, on the fixed normal
/// plane `n`. Emitted as one quad with repeating sprite-local UVs.
struct MergeRect {
    key: MergeKey,
    /// The staged index that emits this rectangle (its first member).
    order: u32,
    /// The rectangle's low corner in section-local block cells, per axis.
    base: [i32; 3],
    /// Extent in cells along the face's u and v axes.
    len_u: i32,
    len_v: i32,
}

/// The greedy pass: merge the staged unit faces into rectangles and emit
/// everything into `sink` in the original emission order.
pub(super) fn flush_staged(
    staged: &mut [StagedQuad],
    sink: &mut MeshSink,
    junction: &mut JunctionScratch,
) {
    // Fast path: nothing mergeable (a section of shapes, or all-AO corners).
    if !staged.iter().any(|q| matches!(q, StagedQuad::Merge(_))) {
        for q in staged.iter() {
            if let StagedQuad::Direct {
                layer,
                positions,
                uvs,
                lights,
                tint,
                sprite,
            } = q
            {
                sink.push_quad(*layer, positions, uvs, *lights, *tint, *sprite);
            }
        }
        return;
    }
    // Pass 1: 1D runs along u. The sort key is (group, fixed plane, v, u), so
    // the members of one candidate run are contiguous and ordered along u.
    // The plane's cell row on the normal axis (the model-space height lives in
    // the key's MergeFace.plane, so same-key members share the exact plane).
    let sort_key = |q: &MergeQuad| {
        let f = &q.key.face;
        let (ua, va) = (f.u_axis as usize, f.v_axis as usize);
        let n_axis = 3 - ua - va;
        let mut plane = q.cell.map(i32::from);
        // The face plane's section coordinate: cell + model-space height,
        // rounded to the cell it falls in (the key's plane bits separate
        // heights within one cell).
        let h = f32::from_bits(f.plane);
        plane[n_axis] += h.floor() as i32;
        let u = i32::from(q.cell[ua]);
        let v = i32::from(q.cell[va]);
        plane[ua] = 0;
        plane[va] = 0;
        (q.key, plane, v, u)
    };
    /// A mergeable quad's placement: the merge group, its plane's cell row,
    /// and its (v, u) cell coordinates.
    type Placed = ((MergeKey, [i32; 3], i32, i32), u32);
    let mut pairs: Vec<Placed> = Vec::with_capacity(staged.len());
    for (i, q) in staged.iter().enumerate() {
        if let StagedQuad::Merge(m) = q {
            pairs.push((sort_key(m), i as u32));
        }
    }
    pairs.sort_by_key(|p| p.0);
    // Scan the sorted list for runs: same key + plane + v, u consecutive.
    let mut runs: Vec<(MergeKey, [i32; 3], i32, i32, i32, u32)> = Vec::new(); // key, plane, v, u0, len, order
    let mut i = 0;
    while i < pairs.len() {
        let (key, plane, v, u0) = pairs[i].0;
        let first_order = pairs[i].1;
        let mut len: i32 = 1;
        while i + (len as usize) < pairs.len() {
            let (k2, p2, v2, u2) = pairs[i + len as usize].0;
            if k2 == key && p2 == plane && v2 == v && u2 == u0 + len {
                len += 1;
            } else {
                break;
            }
        }
        runs.push((key, plane, v, u0, len, first_order));
        i += len as usize;
    }
    // Pass 2: stack runs with the same key, plane, u0 and u-length along v.
    runs.sort_by_key(|r| (r.0, r.1, r.3, r.4, r.2));
    let mut rects: Vec<MergeRect> = Vec::new();
    let mut i = 0;
    while i < runs.len() {
        let (key, plane, v0, u0, len_u, order) = runs[i];
        let mut len_v: i32 = 1;
        while i + (len_v as usize) < runs.len() {
            let (k2, p2, v2, u02, len_u2, _) = runs[i + len_v as usize];
            if k2 == key && p2 == plane && u02 == u0 && len_u2 == len_u && v2 == v0 + len_v {
                len_v += 1;
            } else {
                break;
            }
        }
        let mut base = plane;
        let f = &key.face;
        // `plane` carries the cell row (floor of the height); the fraction
        // comes back at emit time from the key's plane bits.
        let _ = f;
        base[key.face.u_axis as usize] = u0;
        base[key.face.v_axis as usize] = v0;
        rects.push(MergeRect {
            key,
            order,
            base,
            len_u,
            len_v,
        });
        i += len_v as usize;
    }
    // Where the neighbours' corners are: every rectangle edge looks up its
    // line's marks (junction.rs) to place its split vertices.
    junction.collect(&rects, staged);
    // Emit: each rectangle at its first member's staged index, direct quads at
    // their own. `rect_at[i]` says which rectangle staged index i emits.
    let mut rect_at: Vec<Option<usize>> = vec![None; staged.len()];
    for (ri, r) in rects.iter().enumerate() {
        rect_at[r.order as usize] = Some(ri);
    }
    let mut emitted_rects = vec![false; rects.len()];
    for (i, q) in staged.iter().enumerate() {
        match q {
            StagedQuad::Direct {
                layer,
                positions,
                uvs,
                lights,
                tint,
                sprite,
            } => sink.push_quad(*layer, positions, uvs, *lights, *tint, *sprite),
            StagedQuad::Merge(_) => {
                if let Some(ri) = rect_at[i] {
                    if emitted_rects[ri] {
                        continue;
                    }
                    emitted_rects[ri] = true;
                    let r = &rects[ri];
                    emit_rect(r, junction, sink);
                }
            }
        }
    }
}

/// Emits one merged rectangle as a triangle fan over its boundary ring. The
/// ring's corners are the rectangle's quad corners; between them, an edge
/// gets a vertex only where the quads sharing its line have a corner (a
/// T-junction there is what cracks: the two sides rasterize the shared line
/// from different vertex pairs, and the floating-point difference opens
/// single-pixel gaps showing the background, which vanilla — one quad per
/// block face, every edge one unit at most — never draws). Interior edges
/// look up their line's junction marks (every staged quad's and rectangle's
/// corners, bit-exact); an edge lying on a section boundary instead carries
/// a vertex at every unit boundary, which the section across the line also
/// does deterministically, so those edges stay watertight without knowing
/// the neighbour's layout. A rectangle whose neighbours merge the same way
/// keeps a plain four-vertex quad.
///
/// A model quad with fractional corners (stairs, snow layers' sides) files
/// its marks at those exact positions, so they split the edge there too; a
/// fractional corner across a section boundary can still land inside one
/// unit segment — the same T-junction vanilla itself has between such a quad
/// and a unit face, at the same one-unit scale, so nothing new is visible.
///
/// The vertices' UVs repeat the sprite (the fragment shader wraps
/// sprite-local UVs with `fract`, so each block of the rectangle samples
/// the same texels as the unit face did): a UV component that follows the u
/// signs is the offset along u (or `len_u - offset` when flipped).
fn emit_rect(r: &MergeRect, junction: &mut JunctionScratch, sink: &mut MeshSink) {
    let f = &r.key.face;
    let (ua, va) = (f.u_axis as usize, f.v_axis as usize);
    let n_axis = 3 - ua - va;
    // The face plane's model-space height (fraction over the cell row that
    // `base` carries): a full block's positive faces sit at 1.0, a snow
    // layer's top at 2/16, and so on.
    let height = f32::from_bits(f.plane).fract();
    let (marks, ring) = junction.split();
    ring.clear();
    // A quad corner in absolute section-local coordinates, with its
    // tangent-axis offsets (the UVs follow the offsets).
    let corner = |su: i32, sv: i32| {
        let mut pos = [0f32; 3];
        pos[ua] = (r.base[ua] + su * r.len_u) as f32;
        pos[va] = (r.base[va] + sv * r.len_v) as f32;
        pos[n_axis] = r.base[n_axis] as f32 + height;
        (
            pos,
            [su as f32 * r.len_u as f32, sv as f32 * r.len_v as f32],
        )
    };
    // The UVs count from the section's corner, not the rectangle's (`section_uvs`), so
    // ring vertices match the touching corners of the faces around the rectangle.
    let push = |ring: &mut Vec<([f32; 3], [f32; 2])>, pos: [f32; 3], off: [f32; 2]| {
        let (u, v) = (r.base[ua] as f32 + off[0], r.base[va] as f32 + off[1]);
        ring.push((pos, ring_uv(f, u, v, 16, 16)));
    };
    for ci in 0..4 {
        let (su, sv) = (f.corners[ci][0] as i32, f.corners[ci][1] as i32);
        let (cpos, coff) = corner(su, sv);
        push(ring, cpos, coff);
        let (tu, tv) = (
            f.corners[(ci + 1) % 4][0] as i32,
            f.corners[(ci + 1) % 4][1] as i32,
        );
        // Consecutive corners of a quad differ on one tangent axis (both
        // differing would be a crossed quad, not a face; `merge_face` only
        // sees real face quads). The edge between them runs along that axis
        // at the fixed tangent coordinate of the other; splits go strictly
        // between the corners, in walk order.
        if su != tu && sv == tv {
            // Along u at fixed v. On a section boundary (v = 0 or 16) the
            // neighbour is another section: split at every unit. Interior:
            // split at the marks on the edge's line, in this edge only.
            let fixed_v = (r.base[va] + sv * r.len_v) as f32;
            let lo = (r.base[ua] + su.min(tu) * r.len_u) as f32;
            let hi = (r.base[ua] + su.max(tu) * r.len_u) as f32;
            let split = |ring: &mut _, p: f32| {
                let mut pos = cpos;
                pos[ua] = p;
                push(ring, pos, [p - r.base[ua] as f32, coff[1]]);
            };
            if fixed_v == 0.0 || fixed_v == 16.0 {
                let step = if tu > su { 1 } else { -1 };
                let mut k = su * r.len_u + step;
                while k != tu * r.len_u {
                    split(ring, (r.base[ua] + k) as f32);
                    k += step;
                }
            } else {
                let mut splits = [0f32; MAX_SPLITS];
                let line = EdgeLine {
                    d: ua,
                    t_axis: va,
                    t_val: fixed_v as i32,
                    n_axis,
                    plane: cpos[n_axis],
                };
                let n = marks.splits_in(&line, lo, hi, r.len_u < 2, &mut splits);
                if tu > su {
                    for &p in &splits[..n] {
                        split(ring, p);
                    }
                } else {
                    for &p in splits[..n].iter().rev() {
                        split(ring, p);
                    }
                }
            }
        } else if sv != tv && su == tu {
            // Along v at fixed u, mirrored.
            let fixed_u = (r.base[ua] + su * r.len_u) as f32;
            let lo = (r.base[va] + sv.min(tv) * r.len_v) as f32;
            let hi = (r.base[va] + sv.max(tv) * r.len_v) as f32;
            let split = |ring: &mut _, p: f32| {
                let mut pos = cpos;
                pos[va] = p;
                push(ring, pos, [coff[0], p - r.base[va] as f32]);
            };
            if fixed_u == 0.0 || fixed_u == 16.0 {
                let step = if tv > sv { 1 } else { -1 };
                let mut k = sv * r.len_v + step;
                while k != tv * r.len_v {
                    split(ring, (r.base[va] + k) as f32);
                    k += step;
                }
            } else {
                let mut splits = [0f32; MAX_SPLITS];
                let line = EdgeLine {
                    d: va,
                    t_axis: ua,
                    t_val: fixed_u as i32,
                    n_axis,
                    plane: cpos[n_axis],
                };
                let n = marks.splits_in(&line, lo, hi, r.len_v < 2, &mut splits);
                if tv > sv {
                    for &p in &splits[..n] {
                        split(ring, p);
                    }
                } else {
                    for &p in splits[..n].iter().rev() {
                        split(ring, p);
                    }
                }
            }
        }
    }
    let light = [r.key.light[0], r.key.light[1], r.key.light[2]];
    sink.push_fan(
        layer_from_index(r.key.layer as usize),
        haste_assets::Direction::from_index(r.key.dir as usize),
        ring,
        light,
        r.key.tint,
        r.key.sprite,
    );
}
