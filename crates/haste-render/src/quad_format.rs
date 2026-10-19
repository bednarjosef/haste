// Terrain quad pulling (`--terrain-format quad`): instead of four 20-byte
// `PackedVertex`s plus six u32 indices per quad (104 bytes), most terrain is
// one 32-byte `PackedQuad` record in a storage buffer, and the vertex shader
// (`vs_quad` in chunk.wgsl) expands the corners itself from `vertex_index`.
// Everything the mesher cannot express as a record (cross planes, tilted
// fluids) is drawn as four pulled vertex records (80 bytes, still no index
// buffer); greedy-merged fans keep their exact ring triangulation as pulled
// vertices. The drawn image is unchanged: positions and UVs use the same
// quantization grids as `PackedVertex`, and per-corner light/AO, tints and the
// AO diagonal flip are carried bit-exact.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::vertex::{pack_sprite_uv, quantize_coord};

#[cfg(test)]
mod tests;

/// Which geometry a draw range holds (a layer bucket's spans are emitted in
/// draw order; see `SectionLayerMesh::spans`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpanKind {
    /// The vertex format's index buffer: not a quad-format span, but the
    /// marker that lets one LayerDraw struct serve both formats.
    Indexed,
    /// `PackedQuad` records in the quad arena; six vertices per record.
    Rect,
    /// Four `PackedVertex` records per quad in the vertex arena (the AO-flip
    /// corner rotation is baked into the stored order); six vertices per quad.
    Quad,
    /// A greedy-merge fan ring in the vertex arena: `count` records forming
    /// `count - 2` triangles (apex, ring[i], ring[i+1]); three vertices per
    /// triangle.
    Fan,
}

impl SpanKind {
    /// The draw call's vertex count for a span of `count` units (indices for
    /// `Indexed`, whose draws are `draw_indexed` and never ask).
    pub fn vertex_count(self, count: u32) -> u32 {
        match self {
            SpanKind::Indexed => count,
            SpanKind::Rect | SpanKind::Quad => count * 6,
            SpanKind::Fan => count.saturating_sub(2) * 3,
        }
    }
}

/// The terrain format switch (`--terrain-format`): read by the mesh workers
/// (like the greedy-merge switch) and by the renderer's upload/draw paths.
/// Set once before meshing starts; never switched mid-session.
static QUAD_FORMAT: AtomicBool = AtomicBool::new(false);

pub fn set_terrain_format_quad(on: bool) {
    QUAD_FORMAT.store(on, Ordering::Relaxed);
}

pub fn quad_format_enabled() -> bool {
    QUAD_FORMAT.load(Ordering::Relaxed)
}

/// One axis-aligned terrain quad, 32 bytes. The four corners are implied by
/// the normal-axis plane, the low/high bounds on the two tangent axes and the
/// corner ring (see `header`); the UV follows the corner signs through the
/// per-component driver, so a record needs no per-corner UV. Per-corner light
/// is packed as sky (6 bits, vanilla smooth units divided by 4 — they are
/// always a multiple of 4), block light (6 bits) and AO occluder count
/// (3 bits).
///
/// Corner order is the baked quad's vertex order with the AO diagonal-flip
/// rotation already applied, so the shader's fixed corner table
/// `[0, 1, 2, 2, 3, 0]` reproduces today's triangles exactly.
#[repr(C, align(4))]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable, Debug)]
pub struct PackedQuad {
    /// The quad's coordinate on the normal axis (`header & 3`), on the
    /// `PackedVertex` position grid.
    pub plane: u16,
    /// Low/high bounds on tangent axis a (the lower of the two non-normal
    /// axes), same grid.
    pub u0: u16,
    pub u1: u16,
    /// Low/high bounds on tangent axis b, same grid.
    pub v0: u16,
    pub v1: u16,
    /// UV bounds: the sprite-local u value at driver-sign 0 / 1 (fixed
    /// point, `TERRAIN_UV_FIXED_SCALE` per unit), then v.
    pub uv0: [u16; 2],
    pub uv1: [u16; 2],
    /// Atlas sprite index (`sprite_rects`). Sits between the u16s and the
    /// light word so the record has no padding (32 bytes exactly).
    pub sprite: u16,
    /// Per-corner light, corner `i` at bit `15*i`: sky | block << 6 |
    /// ao << 12; the face shade index (bits 3..5 of the packed ao/shade byte)
    /// at bit 60.
    pub light: [u32; 2],
    /// Tint, sRGB bytes.
    pub tint: [u8; 3],
    /// Bits 0..2: normal axis. Bits 2..4: which corner sign drives uv.u
    /// (0 = tangent a, 1 = tangent b), bit 3..4 the same for uv.v.
    /// Bits 4..6: the corner ring's start (the grid corner of vertex 0) and
    /// bit 6 its step (0 = 1, 1 = 3): vertex k is grid corner
    /// `(start + k*step) & 3`, where grid corner g has tangent signs
    /// (a: g == 1 || g == 2, b: g == 2 || g == 3).
    pub header: u8,
}

const _: () = assert!(std::mem::size_of::<PackedQuad>() == 32);

/// Pack the light word pair: corner `i` gets 15 bits (sky 6, block 6, ao 3),
/// the shade index rides bit 60.
fn pack_quad_light(lights: &[[u8; 3]; 4], order: &[usize; 4]) -> [u32; 2] {
    let mut lo = 0u64;
    for (slot, &corner) in order.iter().enumerate() {
        let [sky16, ao_shade, block16] = lights[corner];
        let bits = ((sky16 >> 2) as u64 & 0x3F)
            | (((block16 >> 2) as u64 & 0x3F) << 6)
            | (((ao_shade & 7) as u64) << 12);
        lo |= bits << (15 * slot);
    }
    let shade = ((lights[order[0]][1] >> 3) & 7) as u64;
    lo |= shade << 60;
    [lo as u32, (lo >> 32) as u32]
}

/// Whether a quad's four corners take exactly the two given values on `axis`.
/// Returns (low, high) with low < high, or None.
fn two_valued(positions: &[[f32; 3]; 4], axis: usize) -> Option<(f32, f32)> {
    let mut low = None;
    let mut high = None;
    for p in positions {
        let v = p[axis];
        match (low, high) {
            (None, _) => low = Some(v),
            (Some(l), _) if v == l => {}
            (Some(_), None) => high = Some(v),
            (Some(_), Some(h)) if v == h => {}
            _ => return None,
        }
    }
    let (low, high) = (low?, high?);
    if low < high {
        Some((low, high))
    } else if low > high {
        Some((high, low))
    } else {
        None
    }
}

/// The uv driver for one component: which corner sign (tangent axis a or b)
/// the values follow, plus the value at sign 0 and at sign 1. All four values
/// equal (a constant uv) also matches, with either driver.
fn uv_driver(
    uvs: &[[f32; 2]; 4],
    comp: usize,
    signs: &[(bool, bool); 4],
) -> Option<(u8, f32, f32)> {
    for driver in 0..2u8 {
        let mut v0 = None;
        let mut v1 = None;
        let mut ok = true;
        for (i, &(sa, sb)) in signs.iter().enumerate() {
            let s = if driver == 0 { sa } else { sb };
            let slot = if s { &mut v1 } else { &mut v0 };
            match (*slot, uvs[i][comp]) {
                (None, v) => *slot = Some(v),
                (Some(a), v) if a == v => {}
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            let (v0, v1) = (v0?, v1?);
            if v0 != v1 {
                return Some((driver, v0, v1));
            }
        }
    }
    // Constant across all corners: any driver, both bounds equal.
    let v = uvs[0][comp];
    if uvs.iter().all(|u| u[comp] == v) {
        return Some((0, v, v));
    }
    None
}

/// Pack one quad as a [`PackedQuad`] if it is an axis-aligned rectangle whose
/// UV components each follow one corner sign, with the AO diagonal-flip
/// rotation baked into the corner order. `lights` is `[sky16, ao|shade,
/// block16]` per corner, in the baked vertex order. None: the quad needs the
/// pulled-vertex path.
pub fn pack_rect_quad(
    positions: &[[f32; 3]; 4],
    uvs: &[[f32; 2]; 4],
    lights: &[[u8; 3]; 4],
    tint: u32,
    sprite: u16,
) -> Option<PackedQuad> {
    // The normal axis: exactly one axis the four corners agree on.
    let mut n_axis = None;
    let mut constant = 0;
    for axis in 0..3 {
        let v = positions[0][axis];
        if positions.iter().all(|p| p[axis] == v) {
            constant += 1;
            n_axis = Some(axis);
        }
    }
    if constant != 1 {
        return None;
    }
    let n_axis = n_axis?;
    let (a, b) = match n_axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let (alo, ahi) = two_valued(positions, a)?;
    let (blo, bhi) = two_valued(positions, b)?;
    // Corner signs in baked order; all four combinations must appear.
    let mut signs = [(false, false); 4];
    for (i, p) in positions.iter().enumerate() {
        signs[i] = (p[a] == ahi, p[b] == bhi);
    }
    for i in 0..4 {
        for j in i + 1..4 {
            if signs[i] == signs[j] {
                return None;
            }
        }
    }
    // UV drivers and bounds.
    let (du, uvu0, uvu1) = uv_driver(uvs, 0, &signs)?;
    let (dv, uvv0, uvv1) = uv_driver(uvs, 1, &signs)?;
    // AO diagonal flip (MeshSink::push_quad's rule): rotate the corner order
    // so the shader's fixed table picks the brighter shared edge.
    let ao = |v: usize| (lights[v][1] & 7) as u32;
    let rotate = ao(0) + ao(2) < ao(1) + ao(3);
    // The corner order (after the flip rotation) must walk the quad's
    // perimeter: grid corner g of (sa, sb) is 0..3 around the ring.
    let grid = |i: usize| -> u8 {
        let (sa, sb) = signs[i];
        match (sa, sb) {
            (false, false) => 0,
            (true, false) => 1,
            (true, true) => 2,
            (false, true) => 3,
        }
    };
    let order: [usize; 4] = if rotate { [1, 2, 3, 0] } else { [0, 1, 2, 3] };
    let ring: Vec<u8> = order.iter().map(|&i| grid(i)).collect();
    let start = ring[0];
    let step = (ring[1] + 4 - ring[0]) & 3;
    if step != 1 && step != 3 {
        return None; // crossed quad, not a perimeter order
    }
    for (k, &g) in ring.iter().enumerate() {
        if g != (start + step * k as u8) & 3 {
            return None;
        }
    }
    let header = (n_axis as u8) | (du << 2) | (dv << 3) | (start << 4) | ((step >> 1) << 6);
    Some(PackedQuad {
        plane: quantize_coord(positions[0][n_axis]),
        u0: quantize_coord(alo),
        u1: quantize_coord(ahi),
        v0: quantize_coord(blo),
        v1: quantize_coord(bhi),
        uv0: [pack_sprite_uv(uvu0), pack_sprite_uv(uvv0)],
        uv1: [pack_sprite_uv(uvu1), pack_sprite_uv(uvv1)],
        light: pack_quad_light(lights, &order),
        tint: [(tint >> 16) as u8, (tint >> 8) as u8, tint as u8],
        sprite,
        header,
    })
}

/// One quad-format draw's instance data (instance-rate vertex buffer): the
/// section origin plus where the span's records start and which kind they are.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct QuadInstance {
    /// Section origin in world blocks.
    pub origin: [i32; 3],
    /// First record (quad-arena index for `Rect`, vertex-arena index
    /// (in records, 4 per quad) for `Quad`/`Fan`).
    pub first: u32,
    /// `SpanKind` as u32: 0 = Rect, 1 = Quad, 2 = Fan.
    pub kind: u32,
}

impl QuadInstance {
    /// Attribute layout: one instance per terrain draw.
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: 20,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Sint32x3,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint32,
                offset: 12,
                shader_location: 1,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint32,
                offset: 16,
                shader_location: 2,
            },
        ],
    };

    pub fn kind_of(kind: SpanKind) -> u32 {
        match kind {
            SpanKind::Indexed | SpanKind::Rect => 0,
            SpanKind::Quad => 1,
            SpanKind::Fan => 2,
        }
    }
}

/// The five-u32 indirect argument record for a non-indexed multi-draw (the
/// spec's DrawIndirect command; wgpu's util type is indexed-only here).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DrawIndirect {
    pub vertex_count: u32,
    pub instance_count: u32,
    pub first_vertex: u32,
    pub first_instance: u32,
}
