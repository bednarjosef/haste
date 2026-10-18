// This file is derived from Pomme (https://github.com/josefbednar/pomme),
// pomme-client/src/renderer/chunk/mesher.rs (MeshSink), reused under GPL-3.0
// and reworked for Haste. See THIRD_PARTY.md.

use haste_assets::{Direction, RenderLayer};

use crate::vertex::{pack_vertex, PackedVertex};

/// One render layer of a section mesh: the layer's indices plus, for the
/// depth-tested layers, the `(start, count)` range of each face direction's
/// bucket so the draw can skip direction groups whose faces all point away
/// from the camera (the rasterizer culls them only after vertex shading;
/// skipping saves that vertex work). Translucent keeps one range covering
/// the whole vec — its quads must stay in emission order for blending.
pub struct SectionLayerMesh {
    pub layer: RenderLayer,
    pub indices: Vec<u32>,
    /// Direction order `Direction::ALL` (down, up, north, south, west, east).
    pub dirs: [(u32, u32); 6],
    /// True when `dirs` is not split (translucent): one range, never skipped.
    pub whole: bool,
}

/// Per-section meshing accumulator. Vertices are shared; indices split by
/// render layer (Pomme's MeshSink, extended from solid/cutout to four layers)
/// and, for the depth-tested layers, by face direction. Vertices are packed
/// directly (the intermediate QuadVertex/light/tint/sprite columns of the
/// original are folded into one `PackedVertex` push).
pub(super) struct MeshSink {
    pub(super) vertices: Vec<PackedVertex>,
    /// Translucent indices, in emission order: blending between coplanar
    /// faces of one section (water under ice) depends on it, so this layer
    /// keeps a single bucket.
    pub(super) translucent: Vec<u32>,
    /// The three depth-tested layers' indices, one bucket per face direction
    /// (`layer_index * 6 + direction index`): a direction group whose faces
    /// all point away from the camera is skipped at draw time, saving the
    /// vertex work of quads the rasterizer culls after vertex shading.
    pub(super) dirs: [Vec<u32>; 18],
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
            translucent: Vec::new(),
            dirs: Default::default(),
            min: [f32::MAX; 3],
            max: [f32::MIN; 3],
        }
    }

    /// Clears every bucket, keeping the capacities (thread-local scratch).
    pub(super) fn clear(&mut self) {
        self.vertices.clear();
        self.translucent.clear();
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

    #[allow(clippy::too_many_arguments)]
    pub(super) fn push_quad(
        &mut self,
        layer: RenderLayer,
        dir: Direction,
        positions: &[[f32; 3]; 4],
        uvs: &[[f32; 2]; 4],
        lights: [[u8; 3]; 4],
        tint: u32,
        sprite: u16,
    ) {
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
        let indices: &mut Vec<u32> = if layer == RenderLayer::Translucent {
            &mut self.translucent
        } else {
            &mut self.dirs[layer_index(layer) * 6 + dir.index()]
        };
        if ao(0) + ao(2) < ao(1) + ao(3) {
            indices.extend_from_slice(&[base + 1, base + 2, base + 3, base + 3, base, base + 1]);
        } else {
            indices.extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 3, base]);
        }
    }

    /// Emits one merged rectangle as a triangle fan over its boundary ring
    /// (see `merge.rs`): the ring carries a vertex at every unit boundary the
    /// rectangle's edges cross, so an edge never runs past a corner of the
    /// quads next to it (a T-junction, whose floating-point rasterization can
    /// open single-pixel cracks). All vertices share the rectangle's uniform
    /// light, tint and sprite, so there is no diagonal choice to make.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn push_fan(
        &mut self,
        layer: RenderLayer,
        dir: Direction,
        positions: &[[f32; 3]],
        uvs: &[[f32; 2]],
        light: [u8; 3],
        tint: u32,
        sprite: u16,
    ) {
        let base = self.vertices.len() as u32;
        for i in 0..positions.len() {
            self.vertices
                .push(pack_vertex_raw(positions[i], uvs[i], light, tint, sprite));
            self.note_position(positions[i]);
        }
        let indices: &mut Vec<u32> = if layer == RenderLayer::Translucent {
            &mut self.translucent
        } else {
            &mut self.dirs[layer_index(layer) * 6 + dir.index()]
        };
        for i in 1..positions.len() - 1 {
            indices.extend_from_slice(&[base, base + i as u32, base + i as u32 + 1]);
        }
    }

    /// Takes the section's output: the vertices move, the layer buckets are
    /// concatenated into per-layer index vectors (the buckets keep their
    /// capacity in the thread-local scratch, so the next section of a similar
    /// shape does not grow them again).
    pub(super) fn take_layers(&mut self) -> (Vec<PackedVertex>, Vec<SectionLayerMesh>) {
        let vertices = std::mem::take(&mut self.vertices);
        let mut layers: Vec<SectionLayerMesh> = Vec::with_capacity(4);
        let t_len = self.translucent.len();
        if t_len > 0 {
            layers.push(SectionLayerMesh {
                layer: RenderLayer::Translucent,
                indices: std::mem::take(&mut self.translucent),
                dirs: [(0, t_len as u32), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
                whole: true,
            });
        }
        self.translucent = Vec::with_capacity(t_len);
        for l in 0..3 {
            let buckets = &mut self.dirs[l * 6..l * 6 + 6];
            let total: usize = buckets.iter().map(|b| b.len()).sum();
            if total == 0 {
                continue;
            }
            let mut indices = Vec::with_capacity(total);
            let mut dirs = [(0u32, 0u32); 6];
            for (d, b) in buckets.iter_mut().enumerate() {
                dirs[d] = (indices.len() as u32, b.len() as u32);
                indices.append(b);
            }
            layers.push(SectionLayerMesh {
                layer: layer_from_index(l),
                indices,
                dirs,
                whole: false,
            });
        }
        (vertices, layers)
    }
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
