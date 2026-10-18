// This file is derived from Pomme (https://github.com/josefbednar/pomme),
// pomme-client/src/renderer/chunk/mesher.rs (MeshSink), reused under GPL-3.0
// and reworked for Haste. See THIRD_PARTY.md.

use haste_assets::RenderLayer;

use crate::vertex::{pack_vertex, PackedVertex};

/// Per-section meshing accumulator. Vertices are shared; indices split by
/// render layer (Pomme's MeshSink, extended from solid/cutout to four layers).
/// Vertices are packed directly (the intermediate QuadVertex/light/tint/sprite
/// columns of the original are folded into one `PackedVertex` push).
pub(super) struct MeshSink {
    pub(super) vertices: Vec<PackedVertex>,
    /// Layer -> index list. RenderLayer order gives draw order.
    pub(super) layers: [Vec<u32>; 4],
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
            layers: Default::default(),
            min: [f32::MAX; 3],
            max: [f32::MIN; 3],
        }
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
        let indices = &mut self.layers[layer_index(layer)];
        if ao(0) + ao(2) < ao(1) + ao(3) {
            indices.extend_from_slice(&[base + 1, base + 2, base + 3, base + 3, base, base + 1]);
        } else {
            indices.extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 3, base]);
        }
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
