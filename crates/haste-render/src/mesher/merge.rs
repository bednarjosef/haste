// This file is derived from Pomme (https://github.com/josefbednar/pomme),
// pomme-client/src/renderer/chunk/mesher.rs, reused under GPL-3.0 and reworked
// for Haste (the greedy merge itself is new). See THIRD_PARTY.md.

use super::lighting::face_axes;
use super::sink::{layer_from_index, MeshSink};

/// The greedy-merge signature of a baked quad, if it has one: the positions
/// must be exactly the full unit face of `quad.direction` (the 0/1 corners on
/// the two tangent axes, the face plane on the normal axis) and the
/// sprite-local UVs exactly 0/1 at the corners. Such quads tile: extending
/// one along a tangent axis with the UV repeating keeps every fragment's
/// texel identical (the shader wraps sprite-local UVs with `fract`), so long
/// as the per-vertex attributes are constant across the merge (checked at
/// emit time: uniform light, one tint per quad).
pub(super) fn merge_face(quad: &haste_assets::BakedQuad, uvs: &[[f32; 2]; 4]) -> Option<MergeFace> {
    let dir = quad.direction;
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
    for (i, (&pos, &uv)) in quad.positions.iter().zip(uvs).enumerate() {
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
pub(super) fn flush_staged(staged: &mut [StagedQuad], sink: &mut MeshSink) {
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
                    emit_rect(r, sink);
                }
            }
        }
    }
}

/// Emits one merged rectangle: the four corners of its member unit faces with
/// the UVs stretched to repeat the sprite (the fragment shader wraps
/// sprite-local UVs with `fract`, so each block of the rectangle samples the
/// same texels as the unit face did).
fn emit_rect(r: &MergeRect, sink: &mut MeshSink) {
    let f = &r.key.face;
    let (ua, va) = (f.u_axis as usize, f.v_axis as usize);
    let n_axis = 3 - ua - va;
    // The face plane's model-space height (fraction over the cell row that
    // `base` carries): a full block's positive faces sit at 1.0, a snow
    // layer's top at 2/16, and so on.
    let height = f32::from_bits(f.plane).fract();
    let mut positions = [[0f32; 3]; 4];
    let mut uvs = [[0f32; 2]; 4];
    for vi in 0..4 {
        let (su, sv) = (f.corners[vi][0] as i32, f.corners[vi][1] as i32);
        // The rectangle's corner in section-local block units: the low corner
        // plus the per-vertex extent on each tangent axis; the normal axis
        // keeps the fixed plane.
        let mut pos = r.base;
        pos[ua] += su * r.len_u;
        pos[va] += sv * r.len_v;
        positions[vi] = pos.map(|c| c as f32);
        positions[vi][n_axis] += height;
        // The UV corner follows its own mapping: uv_c = (source corner sign
        // XOR flip) scaled by the source axis's extent.
        for (uv_c, [src, flip]) in uvs[vi].iter_mut().zip(f.uv_map) {
            let (s, len) = if src == 0 {
                (su, r.len_u)
            } else {
                (sv, r.len_v)
            };
            let flag = (s as u8) ^ flip;
            *uv_c = f32::from(flag) * len as f32;
        }
    }
    let light = [r.key.light[0], r.key.light[1], r.key.light[2]];
    let lights = [light; 4];
    sink.push_quad(
        layer_from_index(r.key.layer as usize),
        &positions,
        &uvs,
        lights,
        r.key.tint,
        r.key.sprite,
    );
}
