// This file is derived from Pomme (https://github.com/josefbednar/pomme),
// pomme-client/src/renderer/chunk/mesher.rs (MeshDispatcher), reused under
// GPL-3.0 and reworked for Haste (on rayon instead of raw threads). See
// THIRD_PARTY.md.

use std::sync::Arc;

use haste_core::view::BlockView;
use haste_core::SectionPos;
use parking_lot::Mutex;
use rayon::prelude::*;

use super::mesh_section_with;
use super::snapshot::MeshSnapshot;
use super::tables::MeshTables;
use super::SectionMesh;

/// The worker pool: meshes dirty sections off the render thread. Results come
/// back through a bounded channel (back-pressure), drained per frame with a
/// budget (Pomme MeshDispatcher + drain_results, on rayon instead of raw
/// threads).
/// Something mesh workers can capture section snapshots from on their own threads (the
/// client's shared, lock-protected world). `None` when the section's chunk is gone.
pub trait SnapshotSource: Send + Sync {
    /// Captures `pos` with the biome cells within `blend_radius` blocks (for tints).
    fn capture(&self, pos: SectionPos, blend_radius: i32) -> Option<MeshSnapshot>;
}

pub struct Mesher {
    baked: Arc<haste_assets::BakedBlockModels>,
    atlas: Arc<dyn haste_assets::AtlasRegions + Send + Sync>,
    tables: Mutex<MeshTables>,
    pending: Mutex<Vec<(SectionPos, Arc<MeshSnapshot>)>>,
    results: crossbeam_channel::Receiver<SectionMesh>,
    result_tx: crossbeam_channel::Sender<SectionMesh>,
    /// Sections handed to workers whose mesh has not been sent yet.
    in_flight: Arc<std::sync::atomic::AtomicUsize>,
}

impl Mesher {
    pub fn new(
        baked: Arc<haste_assets::BakedBlockModels>,
        atlas: Arc<dyn haste_assets::AtlasRegions + Send + Sync>,
    ) -> Mesher {
        // Unbounded: workers never block a rayon thread waiting for the render thread.
        let (tx, rx) = crossbeam_channel::unbounded();
        Mesher {
            baked,
            atlas,
            tables: Mutex::new(MeshTables::default()),
            pending: Mutex::new(Vec::new()),
            results: rx,
            result_tx: tx,
            in_flight: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// Submit a section for meshing. The snapshot is captured on the caller's
    /// thread (world data is not Send across threads); deduplicates pending
    /// submissions for the same section.
    pub fn submit(&self, view: &dyn BlockView, pos: SectionPos) {
        self.submit_with_biomes(view, None, pos);
    }

    /// `submit` with the biome source for tints.
    pub fn submit_with_biomes(
        &self,
        view: &dyn BlockView,
        biomes: Option<&dyn crate::tint::BiomeSource>,
        pos: SectionPos,
    ) {
        let radius = self.tables.lock().colors.radius();
        {
            let mut pending = self.pending.lock();
            if pending.iter().any(|(p, _)| *p == pos) {
                return;
            }
            pending.push((
                pos,
                Arc::new(MeshSnapshot::capture_with_biomes(view, biomes, radius, pos)),
            ));
        }
        self.kick();
    }

    /// Replaces the tint/fluid tables used for sections meshed from now on.
    pub fn set_tables(&self, tables: MeshTables) {
        *self.tables.lock() = tables;
    }

    pub fn tables(&self) -> MeshTables {
        self.tables.lock().clone()
    }

    /// Capture and mesh `positions` on the worker pool: the render thread only schedules.
    pub fn submit_from(&self, source: &Arc<dyn SnapshotSource>, positions: &[SectionPos]) {
        if positions.is_empty() {
            return;
        }
        use std::sync::atomic::Ordering;
        let jobs = positions.to_vec();
        self.in_flight.fetch_add(jobs.len(), Ordering::Relaxed);
        let baked = self.baked.clone();
        let tables = self.tables();
        let radius = tables.colors.radius();
        let atlas = self.atlas.clone();
        let tx = self.result_tx.clone();
        let source = source.clone();
        let in_flight = self.in_flight.clone();
        rayon::spawn(move || {
            jobs.into_par_iter().for_each(|pos| {
                // A `None` capture is a chunk the server forgot: send the removal
                // marker so the renderer frees the section (vanilla drops the
                // chunk's render data with the chunk, `ClientChunkCache.drop`).
                let mesh = match source.capture(pos, radius) {
                    Some(snapshot) => mesh_section_with(&snapshot, &baked, &*atlas, &tables, pos),
                    None => SectionMesh::removed(pos),
                };
                let _ = tx.send(mesh);
                in_flight.fetch_sub(1, Ordering::Relaxed);
            });
        });
    }

    /// Mesh all pending sections on the rayon pool. Called from the render
    /// loop; rayon's work-stealing keeps the render thread responsive because
    /// this call only *schedules* the work (spawn) and returns.
    fn kick(&self) {
        let jobs: Vec<(SectionPos, Arc<MeshSnapshot>)> = self.pending.lock().drain(..).collect();
        if jobs.is_empty() {
            return;
        }
        let baked = self.baked.clone();
        let atlas = self.atlas.clone();
        let tables = self.tables.lock().clone();
        let tx = self.result_tx.clone();
        rayon::spawn(move || {
            jobs.into_par_iter()
                .map(|(pos, snapshot)| mesh_section_with(&snapshot, &baked, &*atlas, &tables, pos))
                .for_each(|mesh| {
                    let _ = tx.send(mesh);
                });
        });
    }

    /// Drain finished meshes, up to `budget` per call, into `out` (cleared first).
    pub fn drain(&self, budget: usize, out: &mut Vec<SectionMesh>) {
        out.clear();
        while out.len() < budget {
            match self.results.try_recv() {
                Ok(m) => out.push(m),
                Err(_) => break,
            }
        }
    }

    pub fn pending_len(&self) -> usize {
        self.pending.lock().len()
            + self.results.len()
            + self.in_flight.load(std::sync::atomic::Ordering::Relaxed)
    }
}
