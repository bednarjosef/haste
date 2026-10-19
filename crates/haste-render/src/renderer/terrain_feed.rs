// The terrain feed (vanilla's chunk render dispatcher side): submitting dirty
// sections to the mesher, uploading finished meshes into the arenas, and the
// occlusion bonus for newly uploaded sections.
use std::sync::Arc;

use haste_core::view::BlockView;
use haste_core::{ChunkPos, SectionPos};

use crate::arena::ResidentSection;
use crate::mesher::SectionMesh;

use super::Renderer;

/// Cap on finished meshes held back by the visible-first upload ordering (sections the
/// gate no longer wants: pre-meshed ahead of the turn, or turned away from), awaiting
/// the pre-mesh slice. Bounded so the pipeline's RAM stays near [`MESH_BACKLOG_CAP`]'s:
/// past it the meshes upload in place.
const PREMESH_HOLD_CAP: usize = 2048;
/// The pre-mesh (non-visible) uploads' slice of the frame, in ms, added to the work
/// budget: a pre-meshed section is preparation, not pop-in, so it uploads in the frame's
/// slack; a turn's visible burst keeps the work budget to itself.
const PREMESH_UPLOAD_MS: f32 = 1.0;
/// Pre-mesh slack gate: the mesh pipeline's backlog above this many sections counts as
/// saturated (the workers cannot keep up, or the machine is contended — under load the
/// pre-mesh volume is what collapses the frame; a quiet machine drains a turn's burst
/// past it in a few hundred ms).
const PREMESH_SATURATED: usize = 1536;
/// The backlog must drain below this before pre-meshing resumes (hysteresis: a backlog
/// hovering around the saturation mark must not flip the gate every frame).
const PREMESH_DRAINED: usize = 512;
/// How long the backlog must stand above [`PREMESH_SATURATED`] before pre-meshing stops
/// (a normal turn burst must not trip the gate).
const PREMESH_SUSTAIN: std::time::Duration = std::time::Duration::from_millis(400);

impl Renderer {
    /// Whether pre-meshing may add work this frame: the mesh pipeline's backlog must not
    /// have stood saturated ([`PREMESH_SATURATED`] for [`PREMESH_SUSTAIN`]). The feed's
    /// pre-mesh scan checks this every frame, so it backs off the moment the pipeline
    /// stops draining and resumes once it does.
    pub fn premesh_slack(&self) -> bool {
        !self.premesh_gated
    }

    /// One feed frame of the slack gate ([`Self::premesh_slack`]).
    fn update_premesh_slack(&mut self) {
        let pending = self.mesher.pending_len();
        if pending > PREMESH_SATURATED {
            match self.premesh_saturated_since {
                None => self.premesh_saturated_since = Some(std::time::Instant::now()),
                Some(t) => {
                    if t.elapsed() >= PREMESH_SUSTAIN {
                        self.premesh_gated = true;
                    }
                }
            }
        } else {
            self.premesh_saturated_since = None;
            if pending < PREMESH_DRAINED {
                self.premesh_gated = false;
            }
        }
    }
}

/// What the client knows about a section's blocks, reported as dirty sections are
/// examined (vanilla: `ClientChunkCache` reports its empty sections to
/// `SectionOcclusionGraph.updateEmptySections`, and a missing chunk drops the
/// section's render data).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SectionLoadState {
    /// The section's chunk is not loaded.
    Unloaded,
    /// Loaded and holds no blocks: see-through to the occlusion walk, never meshed.
    Empty,
    /// Loaded with blocks.
    NonEmpty,
}

/// Sets or clears a column's empty-section bit, dropping the column when its last bit
/// goes; returns whether the map changed.
pub(super) fn set_empty_bit(
    empty: &mut crate::occlusion::EmptyColumns,
    column: ChunkPos,
    si: i32,
    is_empty: bool,
) -> bool {
    let bit = 1u32 << si;
    let bits = *empty.get(&column).unwrap_or(&0);
    let want = if is_empty { bits | bit } else { bits & !bit };
    if want == bits {
        return false;
    }
    if want == 0 {
        empty.remove(&column);
    } else {
        empty.insert(column, want);
    }
    true
}

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

    /// The per-state shape tables from haste-data's shape data: the AO occluder table
    /// (vanilla `getShadeBrightness <= 0.2`), the fluid face-occlusion shapes, the
    /// terrain face-occlusion shapes (`BlockState.getOcclusionShape`, vanilla
    /// `Block.shouldRenderFace`) and the face-skip rules from the class dump
    /// (`BlockBehaviour.skipRendering`, see `mesher/skip.rs`). Applies to sections meshed
    /// afterwards: call it before the first chunks.
    pub fn set_shapes(
        &mut self,
        states: &dyn haste_core::view::BlockStateTable,
        shapes: &haste_data::BlockShapes,
    ) {
        let mut tables = self.mesher.tables();
        tables.ao = Arc::new(crate::mesher::AoTable::from_shapes(states, shapes));
        tables.fluid_occluders = Arc::new(crate::fluid::FluidOccluders::from_shapes(
            states,
            shapes,
            &self.baked,
        ));
        tables.face_occluders = Arc::new(crate::mesher::FaceOccluders::from_shapes(states, shapes));
        tables.skip = Arc::new(crate::mesher::SkipTable::load(states));
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
        self.update_premesh_slack();
    }

    /// Uploads finished meshes; returns (sections uploaded, bytes written). The budget
    /// comes from the work budget (`--work-budget`, `renderer/work_budget.rs`): `fixed`
    /// is 2 ms of the frame, extended to 8 ms while a backlog stands (a heavy load's
    /// frames are warm-up — the alternative is the backlog's mesh bytes staying in RAM
    /// until the 2 ms slices drain it); `adaptive` predicts the next frame's cost and
    /// uploads the sections the gate wants first, holding the rest (pre-meshed or
    /// turned away from) for the budget's remainder. The staging belt's copy commands
    /// go into one encoder that is submitted at the end of the batch (before this
    /// frame's render pass reads the arenas).
    fn upload_finished(&mut self) -> (u32, u64) {
        let started = std::time::Instant::now();
        let budget_ms = self.work_budget.budget_ms(self.mesher.pending_len());
        let elapsed_ms = || started.elapsed().as_secs_f32() * 1000.0;
        let mut uploads = 0;
        let mut bytes = 0;
        let mut batch = std::mem::take(&mut self.drain_buf);
        let mut encoder: Option<wgpu::CommandEncoder> = None;
        while elapsed_ms() < budget_ms {
            self.mesher.drain(16, &mut batch);
            if batch.is_empty() {
                break;
            }
            for mesh in batch.drain(..) {
                // Visible sections first: a finished mesh the gate no longer wants
                // (pre-meshed ahead of the turn, or the camera turned away since it
                // was submitted) waits in the hold for the pre-mesh slice below.
                // Removals always run (they free arenas); a full hold drains in place
                // so the RAM stays bounded.
                if !mesh.is_removal()
                    && self.premesh_hold.len() < PREMESH_HOLD_CAP
                    && !self.wants_section(mesh.pos)
                {
                    self.premesh_hold.push_back(mesh);
                    continue;
                }
                bytes += mesh.vertex_bytes() + mesh.index_bytes() + mesh.quad_bytes();
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
        // The held-back uploads. A held mesh the gate wants again (the turn arrived
        // where the cone pointed, or the camera turned back) is visible and takes the
        // work budget like any visible section; the rest — pre-mesh preparation, not
        // pop-in — take their own small slice, so a cone pre-mesh burst uploads in the
        // frame's slack instead of taking the visible sections' budget every frame it
        // arrives in. Non-wanted meshes rotate to the back, keeping their order, so
        // nothing starves behind newer guesses.
        let mut premesh_spent_ms = 0.0;
        let mut examined = 0;
        while examined < self.premesh_hold.len() {
            let Some(mesh) = self.premesh_hold.pop_front() else {
                break;
            };
            examined += 1;
            let wanted = self.wants_section(mesh.pos);
            if !wanted && premesh_spent_ms >= PREMESH_UPLOAD_MS {
                self.premesh_hold.push_back(mesh);
                continue;
            }
            if elapsed_ms() >= budget_ms + PREMESH_UPLOAD_MS {
                // Out of budget for either class: back to the front, and the next
                // frame resumes exactly here.
                self.premesh_hold.push_front(mesh);
                break;
            }
            let before = elapsed_ms();
            bytes += mesh.vertex_bytes() + mesh.index_bytes() + mesh.quad_bytes();
            let encoder = encoder.get_or_insert_with(|| {
                self.gpu
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("mesh-upload"),
                    })
            });
            self.upload_mesh(mesh, encoder);
            uploads += 1;
            if !wanted {
                premesh_spent_ms += elapsed_ms() - before;
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
        // The section's mesh is done even when it produced no vertices (no resident
        // entry): pop-in tracking must not count it as unmeshed forever.
        self.premesh.note_mesh_done(pos);
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
        self.known_empty.lock().clear();
        self.visible_mask = Default::default();
        self.visible_for = None;
        self.scheduled_for = None;
        self.walk_in_flight = false;
        self.clear_bonus();
        self.vis_dirty = false;
        self.want_version += 1;
        // The tracked positions belong to the old world; the run's counters stay.
        self.premesh.world_cleared();
    }

    /// A slab record's arena identity (`MeshArenas::free` input): the packed
    /// layer list is flat in the slab, so rebuild the record's shape.
    fn slab_identity(&self, s: &crate::section_slab::SlabSection) -> ResidentSection {
        ResidentSection {
            pos: s.pos,
            arena: s.arena,
            base_vertex: s.base_vertex,
            vertex_count: s.vertex_count,
            base_quad: s.base_quad,
            quad_count: s.quad_count,
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

    /// A chunk column the walk has not seen just loaded (the feed's first encounter):
    /// the walk treated the column as see-through-unloaded, so its sightline data
    /// changed and the next walk must re-run — otherwise sections of the new chunk
    /// stay out of the visible mask (and unmeshed) until the camera crosses a section
    /// boundary and a walk happens anyway. Vanilla's `SectionOcclusionGraph` learns
    /// of loaded sections the same way (its empty-section update runs per load).
    pub fn note_chunk_loaded(&mut self) {
        self.vis_dirty = true;
    }

    /// Records what the client knows about a section (`SectionLoadState`). The empty
    /// bits are the walk's see-through data for sections that have no mesh, so
    /// unmeshed air never blocks it (vanilla swaps `UNCOMPILED` for `EMPTY` without
    /// compiling); a bit appearing or disappearing marks the sightline data dirty so
    /// the walk re-runs, like vanilla's propagation from newly loaded sections.
    pub fn note_section_state(&mut self, pos: SectionPos, state: SectionLoadState) {
        self.premesh
            .note_column(pos.chunk(), state != SectionLoadState::Unloaded);
        let si = pos.y - self.min_section;
        if !(0..32).contains(&si) {
            return;
        }
        if set_empty_bit(
            &mut self.known_empty.lock(),
            ChunkPos::new(pos.x, pos.z),
            si,
            state == SectionLoadState::Empty,
        ) {
            self.vis_dirty = true;
        }
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
