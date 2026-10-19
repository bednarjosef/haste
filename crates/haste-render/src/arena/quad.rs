// The quad format's arena upload half (`--terrain-format quad`): pulled
// vertex records into the vertex storage buffer, 32-byte rect records into
// the quad half, and the mesh's spans as draw ranges. Split out of arena.rs
// (file-size rule); see the parent module for the arena design.

use crate::mesher::SectionMesh;
use crate::quad_format::{PackedQuad, SpanKind};
use crate::vertex::PackedVertex;

use super::{arena_vertices, Arena, LayerDraw, MeshArenas, ResidentSection};

impl MeshArenas {
    /// The quad format's upload: pulled vertex records into the vertex half,
    /// rect records into the quad half, spans as draw ranges.
    pub(super) fn upload_quad(
        &mut self,
        mesh: &SectionMesh,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Option<ResidentSection> {
        let vcount = mesh.vertices.len() as u32;
        let qcount = mesh.quads.len() as u32;
        if vcount == 0 && qcount == 0 {
            return None;
        }
        if vcount > arena_vertices(0) || qcount > arena_vertices(0) {
            tracing::warn!("section {:?} mesh too large for an arena", mesh.pos);
            return None;
        }
        let mut found = None;
        for (i, a) in self.arenas.iter_mut().enumerate() {
            let Some(q) = a.quads.as_mut() else {
                continue;
            };
            if let Some(v) = a.vertices.alloc(vcount) {
                match q.alloc.alloc(qcount) {
                    Some(qx) => {
                        found = Some((i as u32, v, qx));
                        break;
                    }
                    None => a.vertices.free(v, vcount),
                }
            }
        }
        let (arena, base_vertex, base_quad) = match found {
            Some(f) => f,
            None => {
                // Same growth rule as the vertex format: open a new arena at
                // the current tier, doubling the tier when the last arena of
                // the tier was more than half full.
                let same_tier = self
                    .arenas
                    .last()
                    .is_some_and(|a| a.vertices.total() == arena_vertices(self.tier));
                if same_tier {
                    let last = self.arenas.last().unwrap();
                    let v_used = last.vertices.used();
                    let q_used = last.quads.as_ref().map(|q| q.alloc.used()).unwrap_or(0);
                    if v_used * 2 > last.vertices.total() || q_used * 2 > last.vertices.total() {
                        self.tier += 1;
                    }
                }
                let i = self.new_arena();
                let a = &mut self.arenas[i as usize];
                let v = a.vertices.alloc(vcount).unwrap();
                let q = a.quads.as_mut().unwrap().alloc.alloc(qcount).unwrap();
                (i, v, q)
            }
        };
        let a: &Arena = &self.arenas[arena as usize];
        if vcount > 0 {
            let mut view = self.belt.write_buffer(
                encoder,
                &a.vertex,
                base_vertex as u64 * std::mem::size_of::<PackedVertex>() as u64,
                std::num::NonZeroU64::new(
                    vcount as u64 * std::mem::size_of::<PackedVertex>() as u64,
                )
                .unwrap(),
            );
            view.copy_from_slice(bytemuck::cast_slice(&mesh.vertices));
        }
        if qcount > 0 {
            let q = a.quads.as_ref().unwrap();
            let mut view = self.belt.write_buffer(
                encoder,
                &q.buf,
                base_quad as u64 * std::mem::size_of::<PackedQuad>() as u64,
                std::num::NonZeroU64::new(qcount as u64 * std::mem::size_of::<PackedQuad>() as u64)
                    .unwrap(),
            );
            view.copy_from_slice(bytemuck::cast_slice(&mesh.quads));
        }
        let mut layers = Vec::new();
        for l in &mesh.layers {
            for s in &l.spans {
                layers.push(LayerDraw {
                    layer: l.layer,
                    first: match s.kind {
                        SpanKind::Rect => base_quad + s.first,
                        _ => base_vertex + s.first,
                    },
                    count: s.count,
                    dir: if l.whole { 6 } else { s.dir },
                    kind: s.kind,
                });
            }
        }
        Some(self.finish_resident(
            mesh,
            arena,
            base_vertex,
            vcount,
            base_quad,
            qcount,
            0,
            0,
            layers,
        ))
    }
}
