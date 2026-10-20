// This file is derived from Pomme (https://github.com/josefbednar/pomme),
// pomme-client/src/renderer/chunk/mesher.rs (MeshSink), reused under GPL-3.0
// and reworked for Haste. See THIRD_PARTY.md.

use haste_assets::{Direction, RenderLayer};

use crate::quad_format::{
    pack_general_quad, pack_general_triangle, pack_rect_quad, quad_format_enabled, PackedGeneral,
    PackedQuad, SpanKind,
};
use crate::vertex::{pack_vertex, PackedVertex};

/// One span of a quad-format layer bucket: `count` records of `kind`
/// (`Rect` or `General`) starting at `first`, in emission order. `first` is
/// relative to the bucket's own records (see `take_layers`). `dir` is the
/// facing group (`Direction::ALL` index; 6 = never direction-skipped).
#[derive(Clone, Copy, Debug)]
pub struct QuadSpan {
    pub kind: SpanKind,
    pub first: u32,
    pub count: u32,
    pub dir: u8,
}

/// One render layer of a section mesh: the layer's indices plus the
/// `(start, count)` range of each facing group, so the draw can skip groups
/// whose faces all point away from the camera (the rasterizer culls them only
/// after vertex shading; skipping saves that vertex work). Groups 0..6 hold the
/// faces lying in an axis plane, by the direction they face (`axis_facing`);
/// group 6 holds the rest (crossed plant planes, sloped fluid surfaces), which
/// are never skipped. Within a group the quads keep their emission order (the
/// translucent blend order between coplanar faces depends on it).
///
/// In the quad format (`--terrain-format quad`) the indices are empty and
/// `spans` carries the draw ranges instead, in the same per-direction split.
pub struct SectionLayerMesh {
    pub layer: RenderLayer,
    pub indices: Vec<u32>,
    /// Direction order `Direction::ALL` (down, up, north, south, west, east),
    /// then the never-skipped group.
    pub dirs: [(u32, u32); 7],
    /// Quad format: the layer's draw spans (empty in the vertex format).
    pub spans: Vec<QuadSpan>,
}

/// Facing groups per layer: the six axis directions plus the never-skipped one.
const GROUPS: usize = 7;
/// Bucket count of both formats: four layers' facing groups.
const BUCKETS: usize = 4 * GROUPS;

/// Per-section meshing accumulator. Vertices are shared; indices split by
/// render layer (Pomme's MeshSink, extended from solid/cutout to four layers)
/// and by facing group. Vertices are packed
/// directly (the intermediate QuadVertex/light/tint/sprite columns of the
/// original are folded into one `PackedVertex` push).
///
/// In the quad format each bucket keeps its own records, one vector per kind
/// (rect quads and general quads): a kind's records are contiguous by
/// construction, so `take_layers` derives one span per kind per bucket
/// instead of tracking spans at emission time. Records keep their emission
/// order within a kind; the kinds reorder only between merged rectangles and
/// direct quads, which never overlap coplanar (a merged rectangle is a run of
/// whole block faces, and the model overlays vanilla draws coplanar are
/// direct quads of one state — same kind).
pub(super) struct MeshSink {
    pub(super) vertices: Vec<PackedVertex>,
    /// Quad format: per-bucket rect-quad records (same index as `dirs`).
    pub(super) quad_quads: [Vec<PackedQuad>; BUCKETS],
    /// Quad format: per-bucket general records (cross planes, fluids, split
    /// merged rectangles' fan triangles).
    pub(super) quad_general: [Vec<PackedGeneral>; BUCKETS],
    /// The four layers' indices, one bucket per facing group
    /// (`layer_index * 7 + group`, see `SectionLayerMesh`): a group whose faces
    /// all point away from the camera is skipped at draw time, saving the
    /// vertex work of quads the rasterizer culls after vertex shading.
    pub(super) dirs: [Vec<u32>; BUCKETS],
    /// Minimum extent seen, for the AABB.
    pub(super) min: [f32; 3],
    pub(super) max: [f32; 3],
}

pub(super) fn layer_index(layer: RenderLayer) -> usize {
    match layer {
        RenderLayer::Solid => 0,
        RenderLayer::Cutout => 1,
        RenderLayer::CutoutMipped => 2,
        RenderLayer::Translucent => 3,
    }
}

pub(super) fn layer_from_index(i: usize) -> RenderLayer {
    match i {
        0 => RenderLayer::Solid,
        1 => RenderLayer::Cutout,
        2 => RenderLayer::CutoutMipped,
        _ => RenderLayer::Translucent,
    }
}

impl MeshSink {
    pub(super) fn new() -> MeshSink {
        MeshSink {
            vertices: Vec::new(),
            quad_quads: Default::default(),
            quad_general: Default::default(),
            dirs: Default::default(),
            min: [f32::MAX; 3],
            max: [f32::MIN; 3],
        }
    }

    /// Clears every bucket, keeping the capacities (thread-local scratch).
    pub(super) fn clear(&mut self) {
        self.vertices.clear();
        for v in &mut self.quad_quads {
            v.clear();
        }
        for v in &mut self.quad_general {
            v.clear();
        }
        for d in &mut self.dirs {
            d.clear();
        }
        self.min = [f32::MAX; 3];
        self.max = [f32::MIN; 3];
    }

    fn note_position(&mut self, pos: [f32; 3]) {
        for (a, &p) in pos.iter().enumerate() {
            self.min[a] = self.min[a].min(p);
            self.max[a] = self.max[a].max(p);
        }
    }

    pub(super) fn push_quad(
        &mut self,
        layer: RenderLayer,
        positions: &[[f32; 3]; 4],
        uvs: &[[f32; 2]; 4],
        lights: [[u8; 3]; 4],
        tint: u32,
        sprite: u16,
    ) {
        // The group comes from the geometry, not from `dir`: a model quad's
        // `dir` is its baked nearest axis (a crossed plant plane's is a side),
        // and a fluid's reversed face keeps the direction it is shaded by.
        let bucket = bucket(layer, axis_facing(positions));
        if quad_format_enabled() {
            self.push_quad_pulled(bucket, positions, uvs, lights, tint, sprite);
            return;
        }
        let base = self.vertices.len() as u32;
        for i in 0..4 {
            self.vertices.push(pack_vertex_raw(
                positions[i],
                uvs[i],
                lights[i],
                tint,
                sprite,
            ));
            self.note_position(positions[i]);
        }
        // Flip the diagonal when the default one cuts a bright/dark AO corner
        // (Pomme's emit_face_into anisotropy fix): if 0+2 sums brighter than
        // 1+3, route the shared edge through 1-3. The AO byte holds the
        // occluder count, so brighter is the smaller sum.
        let ao = |v: usize| (lights[v][1] & 7) as u32;
        let indices = &mut self.dirs[bucket];
        if ao(0) + ao(2) < ao(1) + ao(3) {
            indices.extend_from_slice(&[base + 1, base + 2, base + 3, base + 3, base, base + 1]);
        } else {
            indices.extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 3, base]);
        }
    }

    /// The quad-format path of `push_quad`: a rect-shaped quad becomes one
    /// `PackedQuad` record; anything else (cross planes, tilted fluids) keeps
    /// four vertex records with the AO-flip rotation baked into their order,
    /// so the shader's fixed corner table reproduces the vertex path's
    /// triangles. Both keep the bucket's emission order.
    fn push_quad_pulled(
        &mut self,
        bucket: usize,
        positions: &[[f32; 3]; 4],
        uvs: &[[f32; 2]; 4],
        lights: [[u8; 3]; 4],
        tint: u32,
        sprite: u16,
    ) {
        for p in positions {
            self.note_position(*p);
        }
        if let Some(rec) = pack_rect_quad(positions, uvs, &lights, tint, sprite) {
            self.quad_quads[bucket].push(rec);
        } else {
            self.quad_general[bucket]
                .push(pack_general_quad(positions, uvs, &lights, tint, sprite));
        }
    }

    /// Emits one merged rectangle as a triangle fan over its boundary ring
    /// (see `merge.rs`): the ring carries a vertex wherever the quads next to
    /// an edge have a corner (a T-junction, whose floating-point
    /// rasterization can open single-pixel cracks). All vertices share the
    /// rectangle's uniform light, tint and sprite, so there is no diagonal
    /// choice to make.
    pub(super) fn push_fan(
        &mut self,
        layer: RenderLayer,
        dir: Direction,
        ring: &[([f32; 3], [f32; 2])],
        light: [u8; 3],
        tint: u32,
        sprite: u16,
    ) {
        // A merged rectangle is a run of whole block faces in one axis plane,
        // facing its merge direction.
        let bucket = bucket(layer, Some(dir));
        if quad_format_enabled() {
            self.push_fan_pulled(bucket, ring, light, tint, sprite);
            return;
        }
        let base = self.vertices.len() as u32;
        for (pos, uv) in ring {
            self.vertices
                .push(pack_vertex_raw(*pos, *uv, light, tint, sprite));
            self.note_position(*pos);
        }
        let indices = &mut self.dirs[bucket];
        for i in 1..ring.len() - 1 {
            indices.extend_from_slice(&[base, base + i as u32, base + i as u32 + 1]);
        }
    }

    /// The quad-format path of `push_fan`: an unsplit rectangle (a four-corner
    /// ring) is one record (rect when its shape allows, else general); a
    /// split ring becomes one general record per fan triangle — the exact
    /// (apex, ring[i], ring[i+1]) corners with the apex repeated fourth, so
    /// the T-junction behaviour and the per-triangle fog interpolation are
    /// unchanged, the records merge into the bucket's general span, and the
    /// frame's draw count stays near the vertex format's.
    fn push_fan_pulled(
        &mut self,
        bucket: usize,
        ring: &[([f32; 3], [f32; 2])],
        light: [u8; 3],
        tint: u32,
        sprite: u16,
    ) {
        for (pos, _) in ring {
            self.note_position(*pos);
        }
        if ring.len() == 4 {
            let mut positions = [[0.0; 3]; 4];
            let mut uvs = [[0.0; 2]; 4];
            for i in 0..4 {
                positions[i] = ring[i].0;
                uvs[i] = ring[i].1;
            }
            if let Some(rec) = pack_rect_quad(&positions, &uvs, &[light; 4], tint, sprite) {
                self.quad_quads[bucket].push(rec);
            } else {
                self.quad_general[bucket].push(pack_general_quad(
                    &positions,
                    &uvs,
                    &[light; 4],
                    tint,
                    sprite,
                ));
            }
            return;
        }
        let apex = ring[0];
        for i in 1..ring.len() - 1 {
            self.quad_general[bucket].push(pack_general_triangle(
                apex,
                ring[i],
                ring[i + 1],
                light,
                tint,
                sprite,
            ));
        }
    }

    /// Takes the section's output: the vertices move, the layer buckets are
    /// concatenated into per-layer index vectors (the buckets keep their
    /// capacity in the thread-local scratch, so the next section of a similar
    /// shape does not grow them again). In the quad format the buckets'
    /// records are concatenated into the section's record vectors and each
    /// bucket becomes at most two spans — one per kind, in emission order
    /// within the kind.
    pub(super) fn take_layers(
        &mut self,
    ) -> (
        Vec<PackedVertex>,
        Vec<PackedQuad>,
        Vec<PackedGeneral>,
        Vec<SectionLayerMesh>,
    ) {
        if quad_format_enabled() {
            let mut quads: Vec<PackedQuad> = Vec::new();
            let mut generals: Vec<PackedGeneral> = Vec::new();
            let mut layers: Vec<SectionLayerMesh> = Vec::with_capacity(4);
            for b in 0..BUCKETS {
                let (layer, dir) = (layer_from_index(b / GROUPS), (b % GROUPS) as u8);
                let qbase = quads.len() as u32;
                let gbase = generals.len() as u32;
                let mut spans = Vec::with_capacity(2);
                // Chunk spans at the static pattern buffer's size (a real
                // section never reaches it; a pathological one still draws).
                let push_spans =
                    |kind: SpanKind, first: u32, count: u32, spans: &mut Vec<QuadSpan>| {
                        let mut first = first;
                        let mut left = count;
                        while left > 0 {
                            let n = left.min(crate::quad_format::QUADS_PER_SPAN);
                            spans.push(QuadSpan {
                                kind,
                                first,
                                count: n,
                                dir,
                            });
                            first += n;
                            left -= n;
                        }
                    };
                if !self.quad_quads[b].is_empty() {
                    push_spans(
                        SpanKind::Rect,
                        qbase,
                        self.quad_quads[b].len() as u32,
                        &mut spans,
                    );
                }
                if !self.quad_general[b].is_empty() {
                    push_spans(
                        SpanKind::General,
                        gbase,
                        self.quad_general[b].len() as u32,
                        &mut spans,
                    );
                }
                quads.append(&mut self.quad_quads[b]);
                generals.append(&mut self.quad_general[b]);
                if !spans.is_empty() {
                    layers.push(SectionLayerMesh {
                        layer,
                        indices: Vec::new(),
                        dirs: [(0, 0); GROUPS],
                        spans,
                    });
                }
            }
            return (Vec::new(), quads, generals, layers);
        }
        let vertices = std::mem::take(&mut self.vertices);
        let mut layers: Vec<SectionLayerMesh> = Vec::with_capacity(4);
        for l in 0..4 {
            let buckets = &mut self.dirs[l * GROUPS..(l + 1) * GROUPS];
            let total: usize = buckets.iter().map(|b| b.len()).sum();
            if total == 0 {
                continue;
            }
            let mut indices = Vec::with_capacity(total);
            let mut dirs = [(0u32, 0u32); GROUPS];
            for (d, b) in buckets.iter_mut().enumerate() {
                dirs[d] = (indices.len() as u32, b.len() as u32);
                indices.append(b);
            }
            layers.push(SectionLayerMesh {
                layer: layer_from_index(l),
                indices,
                dirs,
                spans: Vec::new(),
            });
        }
        (vertices, Vec::new(), Vec::new(), layers)
    }
}

/// The bucket of a quad in `layer` facing `facing` (`None`: never skipped).
fn bucket(layer: RenderLayer, facing: Option<Direction>) -> usize {
    layer_index(layer) * GROUPS + facing.map_or(GROUPS - 1, Direction::index)
}

/// The direction a quad faces, when it lies in an axis plane: all four
/// corners share that axis's coordinate (within rounding of a rotated model),
/// and the winding's normal (Newell's sum over the four edges, which survives
/// a quad with two coincident corners; outward for the front face the
/// pipeline keeps) gives the sign. Anything else (a crossed plant plane, a
/// sloped fluid surface) is `None`: no single side of the section hides it.
pub(crate) fn axis_facing(p: &[[f32; 3]; 4]) -> Option<Direction> {
    let axis = (0..3).find(|&k| p.iter().all(|c| (c[k] - p[0][k]).abs() < 1e-4))?;
    // Newell's normal component on `axis`: the signed area of the outline
    // projected onto the other two axes.
    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
    let mut n = 0.0;
    for i in 0..4 {
        let (a, b) = (p[i], p[(i + 1) % 4]);
        n += (a[u] - b[u]) * (a[v] + b[v]);
    }
    Some(match (axis, n > 0.0) {
        (0, false) => Direction::West,
        (0, true) => Direction::East,
        (1, false) => Direction::Down,
        (1, true) => Direction::Up,
        (_, false) => Direction::North,
        (_, true) => Direction::South,
    })
}

/// `vertex::pack_vertex` on plain values (no QuadVertex indirection): identical math.
fn pack_vertex_raw(
    position: [f32; 3],
    uv: [f32; 2],
    light: [u8; 3],
    tint: u32,
    sprite: u16,
) -> PackedVertex {
    pack_vertex(
        &crate::vertex::QuadVertex { position, uv },
        sprite,
        light,
        tint,
    )
}
