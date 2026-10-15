// Section meshes in a few large shared GPU buffers ("arenas") instead of one vertex and one
// index buffer per section: a frame then binds each arena once and issues plain
// `draw_indexed(first_index, base_vertex)` calls, which keeps wgpu's per-draw recording and
// per-submit resource tracking small (measured: ~700 per-section buffers cost ~8 ms per frame
// in encoding and submit on the Radeon 860M).

use std::sync::Arc;

use haste_assets::RenderLayer;
use haste_core::SectionPos;

use crate::mesher::SectionMesh;
use crate::vertex::PackedVertex;

/// Largest arena: 2^22 vertices (20 bytes each: 80 MiB) and 3x2^22 indices (4 bytes
/// each: 48 MiB), the proven steady-state size at render distance 32.
const MAX_ARENA_VERTICES: u32 = 1 << 22;
const MAX_ARENA_INDICES: u32 = 3 << 22;

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
    (MAX_ARENA_INDICES >> (3 - shift)).max(3 << 19)
}

/// First-fit range allocator with coalescing frees.
#[derive(Debug, Clone)]
pub(crate) struct RangeAlloc {
    /// Free ranges (start, len), sorted by start, non-adjacent.
    free: Vec<(u32, u32)>,
    /// Total capacity in entries.
    capacity: u32,
}

impl RangeAlloc {
    pub(crate) fn new(len: u32) -> RangeAlloc {
        RangeAlloc {
            free: vec![(0, len)],
            capacity: len,
        }
    }

    pub(crate) fn alloc(&mut self, len: u32) -> Option<u32> {
        if len == 0 {
            return Some(0);
        }
        let i = self.free.iter().position(|&(_, l)| l >= len)?;
        let (start, l) = self.free[i];
        if l == len {
            self.free.remove(i);
        } else {
            self.free[i] = (start + len, l - len);
        }
        Some(start)
    }

    pub(crate) fn free(&mut self, start: u32, len: u32) {
        if len == 0 {
            return;
        }
        let i = self.free.partition_point(|&(s, _)| s < start);
        self.free.insert(i, (start, len));
        // Merge with the next, then with the previous.
        if i + 1 < self.free.len() && self.free[i].0 + self.free[i].1 == self.free[i + 1].0 {
            self.free[i].1 += self.free[i + 1].1;
            self.free.remove(i + 1);
        }
        if i > 0 && self.free[i - 1].0 + self.free[i - 1].1 == self.free[i].0 {
            self.free[i - 1].1 += self.free[i].1;
            self.free.remove(i);
        }
    }

    /// Entries currently allocated (capacity minus the free list).
    pub(crate) fn used(&self) -> u32 {
        let free: u32 = self.free.iter().map(|&(_, l)| l).sum();
        self.capacity - free
    }

    /// Total capacity in entries.
    pub(crate) fn total(&self) -> u32 {
        self.capacity
    }
}

pub(crate) struct Arena {
    pub vertex: wgpu::Buffer,
    pub index: wgpu::Buffer,
    vertices: RangeAlloc,
    indices: RangeAlloc,
}

/// One render layer of a resident section.
#[derive(Clone, Copy, Debug)]
pub struct LayerDraw {
    pub layer: RenderLayer,
    pub first_index: u32,
    pub index_count: u32,
}

/// A section's meshes in an arena.
#[derive(Clone, Debug)]
pub struct ResidentSection {
    pub pos: SectionPos,
    pub arena: u32,
    pub base_vertex: u32,
    pub vertex_count: u32,
    /// Index range of all layers together (for freeing).
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
    /// Current size tier (see [`arena_vertices`]).
    tier: usize,
}

impl MeshArenas {
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> MeshArenas {
        MeshArenas {
            device,
            queue,
            arenas: Vec::new(),
            scratch: Vec::new(),
            tier: 0,
        }
    }

    /// Memory stats for diagnostics: arena count, the vertex+index bytes allocated
    /// (each arena is a fixed-size GPU buffer pair) and the bytes actually used.
    pub fn memory_stats(&self) -> (usize, u64, u64) {
        let bytes = self
            .arenas
            .iter()
            .map(|a| {
                let v = a.vertices.total() as u64 * std::mem::size_of::<PackedVertex>() as u64;
                let i = a.indices.total() as u64 * 4;
                v + i
            })
            .sum();
        let used = self
            .arenas
            .iter()
            .map(|a| {
                let v = a.vertices.used();
                let i = a.indices.used();
                v as u64 * std::mem::size_of::<PackedVertex>() as u64 + i as u64 * 4
            })
            .sum();
        (self.arenas.len(), bytes, used)
    }

    fn new_arena(&mut self) -> u32 {
        let vcount = arena_vertices(self.tier);
        let icount = arena_indices(self.tier);
        let vertex = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("section-arena-vertices"),
            size: vcount as u64 * std::mem::size_of::<PackedVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let index = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("section-arena-indices"),
            size: icount as u64 * 4,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.arenas.push(Arena {
            vertex,
            index,
            vertices: RangeAlloc::new(vcount),
            indices: RangeAlloc::new(icount),
        });
        self.arenas.len() as u32 - 1
    }

    /// Uploads a mesh; None for an empty one (nothing to draw).
    pub fn upload(&mut self, mesh: &SectionMesh) -> Option<ResidentSection> {
        let vcount = mesh.vertices.len() as u32;
        let icount: u32 = mesh.layers.iter().map(|(_, i)| i.len() as u32).sum();
        if vcount == 0 || icount == 0 {
            return None;
        }
        if vcount > arena_vertices(0) || icount > arena_indices(0) {
            tracing::warn!("section {:?} mesh too large for an arena", mesh.pos);
            return None;
        }
        let mut found = None;
        for (i, a) in self.arenas.iter_mut().enumerate() {
            if let Some(v) = a.vertices.alloc(vcount) {
                match a.indices.alloc(icount) {
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
                let same_tier = self
                    .arenas
                    .last()
                    .is_some_and(|a| a.vertices.total() == arena_vertices(self.tier));
                if same_tier {
                    let last = self.arenas.last().unwrap();
                    let v_used = last.vertices.used();
                    let i_used = last.indices.used();
                    if v_used * 2 > last.vertices.total() || i_used * 2 > last.indices.total() {
                        self.tier += 1;
                    }
                }
                let i = self.new_arena();
                let a = &mut self.arenas[i as usize];
                (
                    i,
                    a.vertices.alloc(vcount).unwrap(),
                    a.indices.alloc(icount).unwrap(),
                )
            }
        };
        let a = &self.arenas[arena as usize];
        self.queue.write_buffer(
            &a.vertex,
            base_vertex as u64 * std::mem::size_of::<PackedVertex>() as u64,
            bytemuck::cast_slice(&mesh.vertices),
        );
        self.scratch.clear();
        let mut layers = Vec::with_capacity(mesh.layers.len());
        for (layer, indices) in &mesh.layers {
            layers.push(LayerDraw {
                layer: *layer,
                first_index: index_start + self.scratch.len() as u32,
                index_count: indices.len() as u32,
            });
            self.scratch.extend_from_slice(indices);
        }
        self.queue.write_buffer(
            &a.index,
            index_start as u64 * 4,
            bytemuck::cast_slice(&self.scratch),
        );
        let origin = mesh.pos.block_min();
        let world = |p: [f32; 3]| {
            [
                origin.x as f32 + p[0],
                origin.y as f32 + p[1],
                origin.z as f32 + p[2],
            ]
        };
        Some(ResidentSection {
            pos: mesh.pos,
            arena,
            base_vertex,
            vertex_count: vcount,
            index_start,
            index_total: icount,
            layers,
            aabb: (world(mesh.aabb.0), world(mesh.aabb.1)),
        })
    }

    pub fn free(&mut self, s: &ResidentSection) {
        if let Some(a) = self.arenas.get_mut(s.arena as usize) {
            a.vertices.free(s.base_vertex, s.vertex_count);
            a.indices.free(s.index_start, s.index_total);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_alloc_first_fit_and_coalesce() {
        let mut r = RangeAlloc::new(100);
        let a = r.alloc(10).unwrap();
        let b = r.alloc(20).unwrap();
        let c = r.alloc(30).unwrap();
        assert_eq!((a, b, c), (0, 10, 30));
        r.free(b, 20);
        assert_eq!(r.alloc(5), Some(10), "first fit reuses the hole");
        r.free(10, 5);
        r.free(a, 10);
        r.free(c, 30);
        assert_eq!(r.free, vec![(0, 100)], "all ranges merge back");
        assert_eq!(r.alloc(101), None);
        assert_eq!(r.alloc(100), Some(0));
    }
}
