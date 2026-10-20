// Section meshes in a few large shared GPU buffers ("arenas") instead of one vertex and one
// index buffer per section: a frame then binds each arena once and issues plain
// `draw_indexed(first_index, base_vertex)` calls, which keeps wgpu's per-draw recording and
// per-submit resource tracking small (measured: ~700 per-section buffers cost ~8 ms per frame
// in encoding and submit on the Radeon 860M).
//
// In the quad format (`--terrain-format quad`, `quad_format.rs`) an arena is a pair of
// storage buffers — pulled vertex records and 32-byte rect records — bound once per arena
// as group 3, with no index buffer at all: the draw's vertex count comes from the span
// kind and the shader expands corners from `vertex_index`.

use std::sync::Arc;

use haste_assets::RenderLayer;
use haste_core::SectionPos;

mod quad;
mod range_alloc;

use crate::mesher::SectionMesh;
use crate::quad_format::{quad_format_enabled, PackedGeneral, PackedQuad, SpanKind};
use crate::vertex::PackedVertex;
pub(crate) use range_alloc::RangeAlloc;

/// Largest arena: 2^22 vertices (20 bytes each: 80 MiB) and 2x2^22 indices (u16, so
/// 16 MiB), the proven steady-state size at render distance 32. A quad is 4 vertices and
/// 6 indices; the merged rectangles' fans add a few more, so 2 per vertex leaves room.
/// In the quad format the vertex half holds pulled records (no indices) and the quad
/// half 32-byte rect records, so the same ladder fits a far bigger world.
const MAX_ARENA_VERTICES: u32 = 1 << 22;
const MAX_ARENA_INDICES: u32 = 2 << 22;

/// Indices are relative to the section's base vertex, so a section of up to this many
/// vertices fits u16 indices; a bigger one (a section packed with leaves can pass it)
/// goes into an arena with u32 indices.
const U16_VERTICES: u32 = 1 << 16;

/// Size of arena tier `t` (0-based): the pair starts at 16 MiB (vertices + indices)
/// and doubles per tier up to [`MAX_ARENA_VERTICES`], so a small render distance never
/// pays for the full 128 MiB pairs. A tier keeps opening same-sized arenas while its
/// last one was less than half full (a mesh that just missed the tail is no reason to
/// grow); a section mesh never exceeds the tier-0 size (one section is at most ~64k
/// vertices, tier 0 holds 2^19).
fn arena_vertices(t: usize) -> u32 {
    let shift = t.min(3) as u32;
    (MAX_ARENA_VERTICES >> (3 - shift)).max(1 << 19)
}

fn arena_indices(t: usize) -> u32 {
    let shift = t.min(3) as u32;
    (MAX_ARENA_INDICES >> (3 - shift)).max(2 << 19)
}

/// Bytes per index of an arena's index buffer.
fn index_bytes(wide: bool) -> u64 {
    if wide {
        4
    } else {
        2
    }
}

/// The quad-format halves of an arena: the rect-record and the general-record
/// storage buffers (binding 0 and 1 of group 3).
struct QuadHalf {
    rect: wgpu::Buffer,
    rect_alloc: RangeAlloc,
    general: wgpu::Buffer,
    general_alloc: RangeAlloc,
}

pub(crate) struct Arena {
    /// Vertex-format vertex buffer (None in the quad format).
    pub vertex: Option<wgpu::Buffer>,
    /// Vertex-format index buffer (None in the quad format).
    pub index: Option<wgpu::Buffer>,
    /// The index buffer holds u32 indices (sections past [`U16_VERTICES`]); else u16.
    wide: bool,
    vertices: RangeAlloc,
    indices: RangeAlloc,
    /// Quad-format record halves (None in the vertex format).
    quads: Option<QuadHalf>,
    /// Group 3 (rect records at binding 0, general records at 1) for the quad
    /// pipelines; None in the vertex format or without a layout (tests).
    pub quad_bind: Option<wgpu::BindGroup>,
}

/// One draw range of a resident section: a render layer, or one face
/// direction's group within it (see `SectionLayerMesh`). In the quad format
/// `first`/`count` address the span's records (rect or general by `kind`)
/// instead of indices.
#[derive(Clone, Copy, Debug)]
pub struct LayerDraw {
    pub layer: RenderLayer,
    pub first: u32,
    pub count: u32,
    /// Facing group (`Direction::ALL` index) of this range; 6 = faces off the
    /// axis planes (crossed plants, sloped fluids), never direction-skipped.
    pub dir: u8,
    /// Which pool `first` indexes (and the vertex-count rule; `Indexed` is the
    /// vertex format's index buffer).
    pub kind: SpanKind,
}

/// A section's meshes in an arena.
#[derive(Clone, Debug)]
pub struct ResidentSection {
    pub pos: SectionPos,
    pub arena: u32,
    pub base_vertex: u32,
    pub vertex_count: u32,
    /// Quad format: the section's rect records in the arena's quad half
    /// (0 in the vertex format).
    pub base_quad: u32,
    pub quad_count: u32,
    /// Quad format: the section's general records (0 in the vertex format).
    pub base_general: u32,
    pub general_count: u32,
    /// Index range of all layers together (for freeing); 0 in the quad format.
    pub index_start: u32,
    pub index_total: u32,
    pub layers: Vec<LayerDraw>,
    /// World-space bounds.
    pub aabb: ([f32; 3], [f32; 3]),
}

pub struct MeshArenas {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pub(crate) arenas: Vec<Arena>,
    scratch: Vec<u32>,
    scratch16: Vec<u16>,
    /// Staging for mesh uploads: one pool of recycled host-mapped chunks instead of
    /// wgpu's per-`write_buffer` staging. Measured at render distance 32: one
    /// `queue.write_buffer` per mesh (two per section) leaves ~3 GiB of 64 MiB driver
    /// staging blocks resident; the belt recycles the same chunks every frame.
    belt: wgpu::util::StagingBelt,
    /// Current size tier (see [`arena_vertices`]).
    tier: usize,
    /// The quad pipelines' group-3 layout (quad + vertex record storage),
    /// passed by the renderer; None keeps quad arenas without bind groups.
    quad_bind_layout: Option<wgpu::BindGroupLayout>,
}

impl MeshArenas {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        quad_bind_layout: Option<wgpu::BindGroupLayout>,
    ) -> MeshArenas {
        let belt = wgpu::util::StagingBelt::new(wgpu::Device::clone(&*device), 4 * 1024 * 1024);
        MeshArenas {
            device,
            queue,
            arenas: Vec::new(),
            scratch: Vec::new(),
            scratch16: Vec::new(),
            belt,
            tier: 0,
            quad_bind_layout,
        }
    }

    /// Memory stats for diagnostics: arena count, the vertex+index+record
    /// bytes allocated (each arena is a fixed-size GPU buffer set) and the
    /// bytes actually used.
    pub fn memory_stats(&self) -> (usize, u64, u64) {
        let quad_bytes = |a: &Arena| -> u64 {
            a.quads
                .as_ref()
                .map(|q| {
                    q.rect_alloc.total() as u64 * std::mem::size_of::<PackedQuad>() as u64
                        + q.general_alloc.total() as u64
                            * std::mem::size_of::<PackedGeneral>() as u64
                })
                .unwrap_or(0)
        };
        let quad_used = |a: &Arena| -> u64 {
            a.quads
                .as_ref()
                .map(|q| {
                    q.rect_alloc.used() as u64 * std::mem::size_of::<PackedQuad>() as u64
                        + q.general_alloc.used() as u64
                            * std::mem::size_of::<PackedGeneral>() as u64
                })
                .unwrap_or(0)
        };
        let bytes = self
            .arenas
            .iter()
            .map(|a| {
                let v = a.vertices.total() as u64 * std::mem::size_of::<PackedVertex>() as u64;
                let i = a.indices.total() as u64 * index_bytes(a.wide);
                v + i + quad_bytes(a)
            })
            .sum();
        let used = self
            .arenas
            .iter()
            .map(|a| {
                let v = a.vertices.used();
                let i = a.indices.used();
                v as u64 * std::mem::size_of::<PackedVertex>() as u64
                    + i as u64 * index_bytes(a.wide)
                    + quad_used(a)
            })
            .sum();
        (self.arenas.len(), bytes, used)
    }

    fn new_arena(&mut self, wide: bool) -> u32 {
        let vcount = arena_vertices(self.tier);
        let quad_mode = quad_format_enabled();
        // In the quad format the arena is the two record halves (rect quads
        // at the tier's record count, general records at a quarter of it —
        // they are the minority); the vertex and index halves exist only in
        // the vertex format.
        let (vertex, index) = if quad_mode {
            (None, None)
        } else {
            let vertex = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("section-arena-vertices"),
                size: vcount as u64 * std::mem::size_of::<PackedVertex>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let icount = arena_indices(self.tier);
            let index = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("section-arena-indices"),
                size: icount as u64 * index_bytes(wide),
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            (Some(vertex), Some(index))
        };
        let quads = if quad_mode {
            let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
            let rect = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("section-arena-rect-quads"),
                size: vcount as u64 * std::mem::size_of::<PackedQuad>() as u64,
                usage: storage,
                mapped_at_creation: false,
            });
            let general = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("section-arena-general-quads"),
                size: vcount as u64 / 4 * std::mem::size_of::<PackedGeneral>() as u64,
                usage: storage,
                mapped_at_creation: false,
            });
            Some(QuadHalf {
                rect,
                rect_alloc: RangeAlloc::new(vcount),
                general,
                general_alloc: RangeAlloc::new(vcount / 4),
            })
        } else {
            None
        };
        let quad_bind = quads.as_ref().and_then(|q| {
            self.quad_bind_layout.as_ref().map(|layout| {
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("section-arena-quad-bind"),
                    layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: q.rect.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: q.general.as_entire_binding(),
                        },
                    ],
                })
            })
        });
        self.arenas.push(Arena {
            vertex,
            index,
            wide,
            vertices: RangeAlloc::new(if quad_mode { 0 } else { vcount }),
            indices: RangeAlloc::new(if quad_mode {
                0
            } else {
                arena_indices(self.tier)
            }),
            quads,
            quad_bind,
        });
        self.arenas.len() as u32 - 1
    }

    /// Uploads a mesh through the staging belt into `encoder` (copy commands; the caller
    /// submits the encoder and ends the batch with [`MeshArenas::end_batch`]); None for
    /// an empty one (nothing to draw).
    pub fn upload(
        &mut self,
        mesh: &SectionMesh,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Option<ResidentSection> {
        if quad_format_enabled() {
            self.upload_quad(mesh, encoder)
        } else {
            self.upload_vertex(mesh, encoder)
        }
    }

    /// The vertex format's upload: vertices into the vertex half, layer
    /// indices into the index half.
    fn upload_vertex(
        &mut self,
        mesh: &SectionMesh,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Option<ResidentSection> {
        let vcount = mesh.vertices.len() as u32;
        let icount: u32 = mesh.layers.iter().map(|l| l.indices.len() as u32).sum();
        if vcount == 0 || icount == 0 {
            return None;
        }
        let wide = vcount > U16_VERTICES;
        // u16 ranges stay even, so every range starts and ends 4-byte aligned (buffer
        // copies need it); the padding index is never drawn.
        let ialloc = if wide {
            icount
        } else {
            icount.next_multiple_of(2)
        };
        if vcount > arena_vertices(0) || ialloc > arena_indices(0) {
            tracing::warn!("section {:?} mesh too large for an arena", mesh.pos);
            return None;
        }
        let mut found = None;
        for (i, a) in self.arenas.iter_mut().enumerate() {
            if a.wide != wide {
                continue;
            }
            if let Some(v) = a.vertices.alloc(vcount) {
                match a.indices.alloc(ialloc) {
                    Some(ix) => {
                        found = Some((i as u32, v, ix));
                        break;
                    }
                    None => a.vertices.free(v, vcount),
                }
            }
        }
        let (arena, base_vertex, index_start) = match found {
            Some(f) => f,
            None => {
                // No live arena could fit the mesh: open one at the current tier. If
                // the last arena of this tier was more than half full, the tier is
                // too small for this world — double it for the arenas after this
                // one (a big mesh that just missed the tail is not a reason to grow).
                let last = self.arenas.iter().rev().find(|a| a.wide == wide);
                if let Some(last) = last.filter(|a| a.vertices.total() == arena_vertices(self.tier))
                {
                    let v_used = last.vertices.used();
                    let i_used = last.indices.used();
                    if v_used * 2 > last.vertices.total() || i_used * 2 > last.indices.total() {
                        self.tier += 1;
                    }
                }
                let i = self.new_arena(wide);
                tracing::info!(
                    "opened terrain arena {i} ({} indices; live arenas hold {} vertices, {} indices)",
                    if wide { "u32" } else { "u16" },
                    self.arenas.iter().map(|a| a.vertices.used() as u64).sum::<u64>(),
                    self.arenas.iter().map(|a| a.indices.used() as u64).sum::<u64>(),
                );
                let a = &mut self.arenas[i as usize];
                (
                    i,
                    a.vertices.alloc(vcount).unwrap(),
                    a.indices.alloc(ialloc).unwrap(),
                )
            }
        };
        let a = &self.arenas[arena as usize];
        let mut view = self.belt.write_buffer(
            encoder,
            a.vertex.as_ref().unwrap(),
            base_vertex as u64 * std::mem::size_of::<PackedVertex>() as u64,
            std::num::NonZeroU64::new(vcount as u64 * std::mem::size_of::<PackedVertex>() as u64)
                .unwrap(),
        );
        view.copy_from_slice(bytemuck::cast_slice(&mesh.vertices));
        self.scratch.clear();
        let mut layers = Vec::with_capacity(mesh.layers.len() * 7);
        for l in &mesh.layers {
            let base = index_start + self.scratch.len() as u32;
            // In storage order, so adjacent groups stay adjacent in the draw list.
            for d in crate::mesher::GROUP_ORDER {
                let (start, count) = l.dirs[d];
                if count > 0 {
                    layers.push(LayerDraw {
                        layer: l.layer,
                        first: base + start,
                        count,
                        dir: d as u8,
                        kind: SpanKind::Indexed,
                    });
                }
            }
            self.scratch.extend_from_slice(&l.indices);
        }
        let bytes = index_bytes(wide);
        let mut view = self.belt.write_buffer(
            encoder,
            a.index.as_ref().unwrap(),
            index_start as u64 * bytes,
            std::num::NonZeroU64::new(ialloc as u64 * bytes).unwrap(),
        );
        if wide {
            view.copy_from_slice(bytemuck::cast_slice(&self.scratch));
        } else {
            self.scratch16.clear();
            self.scratch16
                .extend(self.scratch.iter().map(|&i| i as u16));
            self.scratch16.resize(ialloc as usize, 0);
            view.copy_from_slice(bytemuck::cast_slice(&self.scratch16));
        }
        drop(view);
        // The index range goes back with the section (`free`).
        let resident = self.finish_resident(
            mesh,
            arena,
            base_vertex,
            vcount,
            0,
            0,
            0,
            0,
            index_start,
            ialloc,
            layers,
        );
        Some(resident)
    }

    /// The shared tail of both uploads: the world-space AABB.
    #[allow(clippy::too_many_arguments)]
    fn finish_resident(
        &self,
        mesh: &SectionMesh,
        arena: u32,
        base_vertex: u32,
        vertex_count: u32,
        base_quad: u32,
        quad_count: u32,
        base_general: u32,
        general_count: u32,
        index_start: u32,
        index_total: u32,
        layers: Vec<LayerDraw>,
    ) -> ResidentSection {
        let origin = mesh.pos.block_min();
        let world = |p: [f32; 3]| {
            [
                origin.x as f32 + p[0],
                origin.y as f32 + p[1],
                origin.z as f32 + p[2],
            ]
        };
        ResidentSection {
            pos: mesh.pos,
            arena,
            base_vertex,
            vertex_count,
            base_quad,
            quad_count,
            base_general,
            general_count,
            index_start,
            index_total,
            layers,
            aabb: (world(mesh.aabb.0), world(mesh.aabb.1)),
        }
    }

    /// Ends an upload batch: the belt's pending chunk copies go into `encoder`, and the
    /// encoder is submitted (a queue submit is FIFO, so the copies land before the frame's
    /// later render pass reads the arenas). Call once per frame after the last `upload`.
    pub fn end_batch(&mut self, encoder: wgpu::CommandEncoder) {
        self.belt.finish();
        self.queue.submit(Some(encoder.finish()));
        self.belt.recall();
    }

    /// The starting arena tier for a view distance, applied while no arena exists yet (on
    /// join): a big render distance would otherwise climb the 16/32/64 MiB ladder first,
    /// and every small arena it fills along the way stays allocated for the session — the
    /// slack is GPU memory (measured at render distance 32: ~240 MiB).
    pub(crate) fn set_start_tier_for(&mut self, view_distance: i32) {
        if !self.arenas.is_empty() {
            return;
        }
        self.tier = match view_distance {
            ..=8 => 0,
            9..=16 => 1,
            17..=24 => 2,
            _ => 3,
        };
    }

    /// The index buffer's format.
    pub(crate) fn index_format(&self, arena: u32) -> wgpu::IndexFormat {
        if self.arenas[arena as usize].wide {
            wgpu::IndexFormat::Uint32
        } else {
            wgpu::IndexFormat::Uint16
        }
    }

    pub fn free(&mut self, s: &ResidentSection) {
        if let Some(a) = self.arenas.get_mut(s.arena as usize) {
            a.vertices.free(s.base_vertex, s.vertex_count);
            a.indices.free(s.index_start, s.index_total);
            if let Some(q) = a.quads.as_mut() {
                q.rect_alloc.free(s.base_quad, s.quad_count);
                q.general_alloc.free(s.base_general, s.general_count);
            }
        }
    }
}

impl SectionMesh {
    /// GPU bytes the vertices will occupy in an arena.
    pub fn vertex_bytes(&self) -> u64 {
        (self.vertices.len() as u64) * std::mem::size_of::<PackedVertex>() as u64
    }

    /// GPU bytes the indices will occupy in an arena.
    pub fn index_bytes(&self) -> u64 {
        self.layers.iter().map(|l| l.indices.len() as u64 * 4).sum()
    }

    /// GPU bytes the quad records (rect + general) occupy in an arena.
    pub fn quad_bytes(&self) -> u64 {
        self.quads.len() as u64 * std::mem::size_of::<PackedQuad>() as u64
            + self.generals.len() as u64 * std::mem::size_of::<PackedGeneral>() as u64
    }
}

#[cfg(test)]
mod tests;
