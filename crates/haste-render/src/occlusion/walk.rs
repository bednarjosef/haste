// This file is derived from Pomme (https://github.com/josefbednar/pomme),
// pomme-client/src/renderer/chunk/occlusion_graph.rs, reused under GPL-3.0
// (vanilla-style section visibility for cave culling: the occlusion walk's
// worker thread). See docs/THIRD_PARTY.md.
//
// Split from occlusion.rs: the walk algorithm (`compute_visible_mask`) lives
// there; this is the scheduling around it (vanilla `SectionOcclusionGraph`'s
// background full update and the shared maps it snapshots).

use std::sync::Arc;
use std::thread::JoinHandle;

use ahash::AHashMap;
use crossbeam_channel::{unbounded, Receiver, Sender};
use glam::DVec3;
use haste_core::{ChunkPos, SectionPos};
use parking_lot::Mutex;

use super::{compute_visible_mask, VisibilitySet, VisibleMask};

/// Per-column empty-section bits, the renderer's mirror of vanilla's
/// `ClientChunkCache` empty sections (`SectionOcclusionGraph.emptySections`):
/// bit `si` set when the section is loaded and known to hold no blocks.
pub type EmptyColumns = AHashMap<ChunkPos, u32>;

/// Snapshot the shared section visibility sets and empty-section bits into a
/// dense grid for the walk: the walk reads each visited section's set once, and
/// a slice index beats a hash probe per node. A section with a mesh carries its
/// mesh's set; a known-empty one is see-through (vanilla swaps `UNCOMPILED` for
/// `EMPTY`, whose `facesCanSeeEachother` is true, without compiling it); any
/// other section is opaque to the walk, like vanilla's `UNCOMPILED`, so the
/// walk under-reaches through unmeshed terrain exactly as vanilla's does. Both
/// maps are copied into the reusable buffers under their locks (a fast drain)
/// and the grid is built after they are released, so the render thread's
/// per-upload lock never waits on the grid build. `empty_grid` gets one flag per
/// node, set for the known-empty sections (reused across walks).
#[allow(clippy::too_many_arguments)]
fn dense_visibility_grid(
    vis: &Mutex<AHashMap<(ChunkPos, i32), VisibilitySet>>,
    empty: &Mutex<EmptyColumns>,
    entries: &mut Vec<((ChunkPos, i32), VisibilitySet)>,
    empty_entries: &mut Vec<(ChunkPos, u32)>,
    cam: ChunkPos,
    r: i32,
    h: i32,
    empty_grid: &mut Vec<bool>,
) -> Vec<VisibilitySet> {
    entries.clear();
    entries.extend(vis.lock().iter().map(|(k, v)| (*k, *v)));
    empty_entries.clear();
    empty_entries.extend(empty.lock().iter().map(|(k, v)| (*k, *v)));
    let w = 2 * r + 1;
    let mut dense = vec![VisibilitySet::none(); (w * w * h) as usize];
    empty_grid.clear();
    empty_grid.resize((w * w * h) as usize, false);
    for (c, bits) in empty_entries.iter() {
        let (dx, dz) = (c.x - cam.x + r, c.z - cam.z + r);
        if dx < 0 || dz < 0 || dx >= w || dz >= w {
            continue;
        }
        let base = ((dx * w + dz) * h) as usize;
        for si in 0..h.min(32) {
            if bits & (1 << si) != 0 {
                dense[base + si as usize] = VisibilitySet::all();
                empty_grid[base + si as usize] = true;
            }
        }
    }
    for ((c, si), v) in entries.iter() {
        let dx = c.x - cam.x + r;
        let dz = c.z - cam.z + r;
        if dx < 0 || dz < 0 || dx >= w || dz >= w || *si < 0 || *si >= h {
            continue;
        }
        dense[((dx * w + dz) * h + *si) as usize] = *v;
    }
    dense
}

/// One walk request: the camera section and render distance the mask is
/// computed for, plus the world vertical extent and the eye position. Runs on
/// the worker: snapshot the visibility sets under the shared lock, then walk.
#[derive(Clone, Copy)]
struct WalkJob {
    cam_section: SectionPos,
    view_distance: i32,
    section_count: i32,
    min_section: i32,
    eye: DVec3,
}

impl WalkJob {
    fn run(
        &self,
        vis: &Mutex<AHashMap<(ChunkPos, i32), VisibilitySet>>,
        empty: &Mutex<EmptyColumns>,
        entries: &mut Vec<((ChunkPos, i32), VisibilitySet)>,
        empty_entries: &mut Vec<(ChunkPos, u32)>,
        empty_grid: &mut Vec<bool>,
    ) -> VisibleMask {
        let r = self.view_distance + 1;
        let h = self.section_count;
        let cam = ChunkPos::new(self.cam_section.x, self.cam_section.z);
        let dense =
            dense_visibility_grid(vis, empty, entries, empty_entries, cam, r, h, empty_grid);
        let w = 2 * r + 1;
        let mut mask = compute_visible_mask(
            |c, si| {
                let dx = c.x - cam.x + r;
                let dz = c.z - cam.z + r;
                if dx < 0 || dz < 0 || dx >= w || dz >= w || si < 0 || si >= h {
                    return VisibilitySet::none();
                }
                dense[((dx * w + dz) * h + si) as usize]
            },
            cam,
            self.cam_section.y - self.min_section,
            self.eye,
            self.min_section * 16,
            h,
            r,
            self.view_distance,
        );
        // Vanilla's walk passes through empty sections but only puts sections with
        // something to draw into its `sectionTree`; the draw list visits the order, so
        // dropping the empty ones here saves the render thread a lookup each (at render
        // distance 32 the walk reaches ~66k nodes, ~8k of them non-empty).
        mask.retain_order(|n| !empty_grid[n as usize]);
        mask
    }
}

/// The section visibility walk, off the render thread. Vanilla runs the full
/// `SectionOcclusionGraph` update on a background executor and keeps drawing
/// the previous graph until the new one is swapped in; this does the same: the
/// render thread only schedules a walk (`schedule`) and collects the finished
/// mask (`poll`), never walking itself. One dedicated worker thread runs the
/// walks serially, so at most one is in flight.
pub struct AsyncOcclusionWalker {
    done_rx: Receiver<VisibleMask>,
    job_tx: Option<Sender<WalkJob>>,
    worker: Option<JoinHandle<()>>,
}

impl AsyncOcclusionWalker {
    /// Shares `vis` and `empty` with the worker: the render thread inserts on
    /// upload / section-state change, the worker snapshots under the locks
    /// when a walk starts.
    pub fn new(
        vis: Arc<Mutex<AHashMap<(ChunkPos, i32), VisibilitySet>>>,
        empty: Arc<Mutex<EmptyColumns>>,
    ) -> std::io::Result<Self> {
        let (job_tx, job_rx) = unbounded::<WalkJob>();
        let (done_tx, done_rx) = unbounded::<VisibleMask>();
        let vis_worker = vis;
        let empty_worker = empty;
        let worker = std::thread::Builder::new()
            .name("haste-occlusion".into())
            .spawn(move || {
                // The walk's snapshot buffers, reused across jobs (no allocation per walk).
                let mut entries: Vec<((ChunkPos, i32), VisibilitySet)> = Vec::new();
                let mut empty_entries: Vec<(ChunkPos, u32)> = Vec::new();
                let mut empty_grid: Vec<bool> = Vec::new();
                while let Ok(job) = job_rx.recv() {
                    let mask = job.run(
                        &vis_worker,
                        &empty_worker,
                        &mut entries,
                        &mut empty_entries,
                        &mut empty_grid,
                    );
                    if done_tx.send(mask).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            done_rx,
            job_tx: Some(job_tx),
            worker: Some(worker),
        })
    }

    /// Ask the worker to walk for `cam_section` / `view_distance`. Cheap: a
    /// channel send; the render thread never walks.
    pub fn schedule(
        &self,
        cam_section: SectionPos,
        view_distance: i32,
        section_count: i32,
        min_section: i32,
        eye: DVec3,
    ) {
        let job = WalkJob {
            cam_section,
            view_distance,
            section_count,
            min_section,
            eye,
        };
        if let Some(tx) = &self.job_tx {
            let _ = tx.send(job);
        }
    }

    /// Collect the newest finished mask, if any. Non-blocking: a walk that is
    /// not done yet simply keeps the previous mask.
    pub fn poll(&self) -> Option<VisibleMask> {
        let mut latest = None;
        while let Ok(mask) = self.done_rx.try_recv() {
            latest = Some(mask);
        }
        latest
    }
}

impl Drop for AsyncOcclusionWalker {
    fn drop(&mut self) {
        // Drop the request channel first so the worker exits; join it.
        self.job_tx.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
