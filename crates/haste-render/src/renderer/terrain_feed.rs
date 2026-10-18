// The terrain feed (vanilla's chunk render dispatcher side): submitting dirty
// sections to the mesher, uploading finished meshes into the arenas, and the
// occlusion bonus for newly uploaded sections.
use std::sync::Arc;

use haste_core::view::BlockView;
use haste_core::{ChunkPos, SectionPos};

use crate::arena::ResidentSection;
use crate::mesher::SectionMesh;

use super::Renderer;

/// Cap on sections meshed but not yet uploaded (the mesher's pending snapshots, queued
/// results and in-flight jobs). At render distance 32 one section's mesh averages ~50 KiB,
/// so this bounds the pipeline's RAM to ~150 MiB instead of the whole view distance's
/// meshes (~1.4 GiB) piling up behind the per-frame upload budget.
const MESH_BACKLOG_CAP: usize = 3072;

impl Renderer {
    /// The biome table of the joined server (synced `minecraft:worldgen/biome` registry, index =
    /// biome id). Applies to sections meshed afterwards: call it before the first chunks.
    pub fn set_biomes(&mut self, biomes: &[crate::tint::BiomeClimate]) {
        let mut tables = self.mesher.tables();
        let radius = tables.colors.radius();
        tables.colors =
            Arc::new(crate::tint::BiomeColors::new(biomes, &self.colormaps).with_radius(radius));
        self.mesher.set_tables(tables);
    }

    /// Biome blend radius (vanilla "Biome Blend": 0 = off, 2 = default 5x5).
    pub fn set_biome_blend(&mut self, radius: i32) {
        let mut tables = self.mesher.tables();
        tables.colors = Arc::new((*tables.colors).clone().with_radius(radius));
        self.mesher.set_tables(tables);
    }

    /// The AO occluder table from haste-data's shape data (vanilla
    /// `getShadeBrightness <= 0.2`). Applies to sections meshed afterwards:
    /// call it before the first chunks.
    pub fn set_ao_shapes(
        &mut self,
        states: &dyn haste_core::view::BlockStateTable,
        shapes: &haste_data::BlockShapes,
    ) {
        let mut tables = self.mesher.tables();
        tables.ao = Arc::new(crate::mesher::AoTable::from_shapes(states, shapes));
        self.mesher.set_tables(tables);
    }

    /// Submit dirty sections to the mesher (snapshots taken from `world`) and
    /// upload finished meshes (per-frame budget). Without biomes every block tints as plains.
    pub fn update_world(
        &mut self,
        world: &dyn BlockView,
        dirty: &[SectionPos],
        camera_chunk: ChunkPos,
        view_distance: i32,
    ) {
        self.update_world_with_biomes(world, None, dirty, camera_chunk, view_distance);
    }

    /// `update_world` reading biomes (for grass/foliage/water tints) from `biomes`.
    pub fn update_world_with_biomes(
        &mut self,
        world: &dyn BlockView,
        biomes: Option<&dyn crate::tint::BiomeSource>,
        dirty: &[SectionPos],
        _camera_chunk: ChunkPos,
        view_distance: i32,
    ) {
        if view_distance != self.view_distance {
            self.view_distance = view_distance;
            self.visible_for = None;
        }
        self.arenas.set_start_tier_for(view_distance);
        for pos in dirty {
            self.mesher.submit_with_biomes(world, biomes, *pos);
        }
        self.last_uploads = self.upload_finished();
    }

    /// Like `update_world`, but the mesh workers capture the snapshots themselves from
    /// `source` (the render thread neither reads the world nor holds its lock).
    ///
    /// Back-pressure: a mesh result holds its vertex and index bytes, and the workers mesh
    /// faster than the per-frame upload budget drains, so without a cap a big load (render
    /// distance 32 is ~28k sections) piles up GiB of RAM before the uploads catch up. Over
    /// [`MESH_BACKLOG_CAP`] the dirty sections wait in `mesh_carry` — positions, not
    /// bytes — and are submitted as the uploads drain.
    pub fn update_world_from(
        &mut self,
        source: &Arc<dyn crate::mesher::SnapshotSource>,
        dirty: &[SectionPos],
        _camera_chunk: ChunkPos,
        view_distance: i32,
    ) {
        if view_distance != self.view_distance {
            self.view_distance = view_distance;
            self.visible_for = None;
        }
        self.arenas.set_start_tier_for(view_distance);
        let room = MESH_BACKLOG_CAP.saturating_sub(self.mesher.pending_len());
        if room > 0 {
            let mut jobs = std::mem::take(&mut self.mesh_carry);
            if jobs.len() > room {
                // The rest stays carried, in queue order (the feed queued nearest first).
                self.mesh_carry = jobs.split_off(room);
            }
            jobs.extend_from_slice(dirty);
            self.mesher.submit_from(source, &jobs);
        } else if !dirty.is_empty() {
            self.mesh_carry.extend_from_slice(dirty);
        }
        self.last_uploads = self.upload_finished();
    }

    /// Uploads finished meshes; returns (sections uploaded, bytes written). The budget is
    /// 2 ms of the frame, extended to 8 ms while a backlog stands (a heavy load's frames
    /// are warm-up — the alternative is the backlog's mesh bytes staying in RAM until the
    /// 2 ms slices drain it). The staging belt's copy commands go into one encoder that is
    /// submitted at the end of the batch (before this frame's render pass reads the arenas).
    fn upload_finished(&mut self) -> (u32, u64) {
        let started = std::time::Instant::now();
        let budget = if self.mesher.pending_len() > 1024 {
            std::time::Duration::from_millis(8)
        } else {
            std::time::Duration::from_millis(2)
        };
        let mut uploads = 0;
        let mut bytes = 0;
        let mut batch = std::mem::take(&mut self.drain_buf);
        let mut encoder: Option<wgpu::CommandEncoder> = None;
        while started.elapsed() < budget {
            self.mesher.drain(16, &mut batch);
            if batch.is_empty() {
                break;
            }
            for mesh in batch.drain(..) {
                bytes += mesh.vertex_bytes() + mesh.index_bytes();
                let encoder = encoder.get_or_insert_with(|| {
                    self.gpu
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("mesh-upload"),
                        })
                });
                self.upload_mesh(mesh, encoder);
                uploads += 1;
            }
        }
        self.drain_buf = batch;
        if let Some(encoder) = encoder {
            self.arenas.end_batch(encoder);
        }
        (uploads, bytes)
    }

    fn upload_mesh(&mut self, mesh: SectionMesh, encoder: &mut wgpu::CommandEncoder) {
        let pos = mesh.pos;
        let si = pos.y - self.min_section;
        if mesh.is_removal() {
            // The chunk is gone: drop the resident mesh and its visibility data
            // (`ClientChunkCache.drop` releases the render data with the chunk). A
            // section mined to air is an ordinary empty mesh and takes the path below.
            let mut vis = self.section_vis.lock();
            let had_vis = vis.remove(&(ChunkPos::new(pos.x, pos.z), si)).is_some();
            drop(vis);
            // The bonus bit lives on the section record and goes with it (a stale
            // `bonus_list` entry is skipped when the list is cleared).
            if let Some(old) = self.sections.remove(&pos) {
                self.arenas.free(&old);
            }
            if had_vis {
                self.vis_dirty = true;
            }
            return;
        }
        let mut vis = self.section_vis.lock();
        let prev = vis.insert((ChunkPos::new(pos.x, pos.z), si), mesh.visibility);
        drop(vis);
        if prev != Some(mesh.visibility) {
            self.vis_dirty = true;
            self.add_to_bonus(pos, si);
        }
        if let Some(old) = self.sections.remove(&pos) {
            self.arenas.free(&old);
        }
        if let Some(resident) = self.arenas.upload(&mesh, encoder) {
            self.sections.insert(resident);
        }
    }

    /// Drops every resident section mesh back to the empty state of a fresh renderer, the world
    /// the menus draw over after leaving a server (vanilla `Minecraft.disconnectClient` clears
    /// the level and its render chunks). The per-frame lists (entities, particles, weather,
    /// overlays, the hand) are rebuilt from each `FrameInput` and need no reset. The mesh
    /// arenas keep their GPU buffers: `sections` is emptied, not freed, so a rejoin uploads into
    /// the same arenas without reallocating them.
    pub fn clear_world(&mut self) {
        for s in self.sections.iter() {
            self.arenas.free(&self.slab_identity(s));
        }
        self.sections.clear();
        self.section_vis.lock().clear();
        self.visible_mask = Default::default();
        self.visible_for = None;
        self.scheduled_for = None;
        self.walk_in_flight = false;
        self.clear_bonus();
        self.vis_dirty = false;
    }

    /// A slab record's arena identity (`MeshArenas::free` input): the packed
    /// layer list is flat in the slab, so rebuild the record's shape.
    fn slab_identity(&self, s: &crate::section_slab::SlabSection) -> ResidentSection {
        ResidentSection {
            pos: s.pos,
            arena: s.arena,
            base_vertex: s.base_vertex,
            vertex_count: s.vertex_count,
            index_start: s.index_start,
            index_total: s.index_total,
            layers: Vec::new(),
            aabb: s.aabb,
        }
    }

    /// Clear the bonus bits (a new mask arrived or the world went away); the
    /// dense list names the sections, so it stays proportional to the bonus.
    pub(super) fn clear_bonus(&mut self) {
        for &pos in &self.bonus_list {
            if let Some(s) = self.sections.get_mut(pos) {
                s.bonus = false;
            }
        }
        self.bonus_list.clear();
        self.bonus_dirty = false;
    }

    /// A newly uploaded section is drawn until the async walk catches up when a
    /// face-neighbour is in the current mask: vanilla seeds newly loaded
    /// sections into the propagation queue, so they appear before the next
    /// full update. The bonus is cleared when the new mask arrives.
    fn add_to_bonus(&mut self, pos: SectionPos, si: i32) {
        if !(0..32).contains(&si) {
            return;
        }
        for face in crate::occlusion::Face::ALL {
            let (dx, dy, dz) = face.offset();
            let (nx, nsy, nz) = (pos.x + dx, pos.y + dy, pos.z + dz);
            let nsi = nsy - self.min_section;
            if !(0..32).contains(&nsi) {
                continue;
            }
            let column = self.visible_mask.column(ChunkPos::new(nx, nz));
            if column & (1u32 << nsi) != 0 {
                if let Some(s) = self.sections.get_mut(pos) {
                    if !s.bonus {
                        s.bonus = true;
                        self.bonus_list.push(pos);
                    }
                    self.bonus_dirty = true;
                }
                return;
            }
        }
    }
}
