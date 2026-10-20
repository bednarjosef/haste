// The quad format's arena upload half (`--terrain-format quad`): rect and
// general records into their storage buffers, and the mesh's spans as draw
// ranges. Split out of arena.rs (file-size rule); see the parent module for
// the arena design.

use crate::mesher::SectionMesh;
use crate::quad_format::{PackedGeneral, PackedQuad, SpanKind};

use super::{arena_vertices, Arena, LayerDraw, MeshArenas, ResidentSection};

impl MeshArenas {
    /// The quad format's upload: rect records into the rect half, general
    /// records into the general half, spans as draw ranges.
    pub(super) fn upload_quad(
        &mut self,
        mesh: &SectionMesh,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Option<ResidentSection> {
        let qcount = mesh.quads.len() as u32;
        let gcount = mesh.generals.len() as u32;
        if qcount == 0 && gcount == 0 {
            return None;
        }
        if qcount > arena_vertices(0) || gcount > arena_vertices(0) / 4 {
            tracing::warn!("section {:?} mesh too large for an arena", mesh.pos);
            return None;
        }
        let mut found = None;
        for (i, a) in self.arenas.iter_mut().enumerate() {
            let Some(q) = a.quads.as_mut() else {
                continue;
            };
            if let Some(qx) = q.rect_alloc.alloc(qcount) {
                match q.general_alloc.alloc(gcount) {
                    Some(gx) => {
                        found = Some((i as u32, qx, gx));
                        break;
                    }
                    None => q.rect_alloc.free(qx, qcount),
                }
            }
        }
        let (arena, base_quad, base_general) = match found {
            Some(f) => f,
            None => {
                // Same growth rule as the vertex format: open a new arena at
                // the current tier, doubling the tier when the last arena of
                // the tier was more than half full (either half).
                let same_tier = self.arenas.last().is_some_and(|a| {
                    a.quads
                        .as_ref()
                        .is_some_and(|q| q.rect_alloc.total() == arena_vertices(self.tier))
                });
                if same_tier {
                    let last = self.arenas.last().unwrap();
                    let (rv, rg) = last
                        .quads
                        .as_ref()
                        .map_or((0, 0), |q| (q.rect_alloc.used(), q.general_alloc.used()));
                    if rv * 2 > arena_vertices(self.tier) || rg * 2 > arena_vertices(self.tier) / 4
                    {
                        self.tier += 1;
                    }
                }
                let i = self.new_arena();
                let a = &mut self.arenas[i as usize];
                let q = a.quads.as_mut().unwrap();
                let qx = q.rect_alloc.alloc(qcount).unwrap();
                let gx = q.general_alloc.alloc(gcount).unwrap();
                (i, qx, gx)
            }
        };
        let a: &Arena = &self.arenas[arena as usize];
        let q = a.quads.as_ref().unwrap();
        if qcount > 0 {
            let mut view = self.belt.write_buffer(
                encoder,
                &q.rect,
                base_quad as u64 * std::mem::size_of::<PackedQuad>() as u64,
                std::num::NonZeroU64::new(qcount as u64 * std::mem::size_of::<PackedQuad>() as u64)
                    .unwrap(),
            );
            view.copy_from_slice(bytemuck::cast_slice(&mesh.quads));
        }
        if gcount > 0 {
            let mut view = self.belt.write_buffer(
                encoder,
                &q.general,
                base_general as u64 * std::mem::size_of::<PackedGeneral>() as u64,
                std::num::NonZeroU64::new(
                    gcount as u64 * std::mem::size_of::<PackedGeneral>() as u64,
                )
                .unwrap(),
            );
            view.copy_from_slice(bytemuck::cast_slice(&mesh.generals));
        }
        let mut layers = Vec::new();
        for l in &mesh.layers {
            for s in &l.spans {
                layers.push(LayerDraw {
                    layer: l.layer,
                    first: match s.kind {
                        SpanKind::Rect => base_quad + s.first,
                        SpanKind::General => base_general + s.first,
                        SpanKind::Indexed => s.first,
                    },
                    count: s.count,
                    dir: s.dir,
                    kind: s.kind,
                });
            }
        }
        Some(self.finish_resident(
            mesh,
            arena,
            0,
            0,
            base_quad,
            qcount,
            base_general,
            gcount,
            0,
            0,
            layers,
        ))
    }
}
