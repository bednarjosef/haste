//! Moving held-back dirty sections back into the mesh queue when the renderer wants
//! them (vanilla `LevelRenderer.scanChunks`'s re-check of its dirty set): on visible-set
//! changes, when a chunk a first-build hold waited on loads, and an always-on rotating
//! sweep that covers the whole held-back list every few frames.

use haste_core::SectionPos;

use super::{Renderer, WorldFeed};

/// The waiting re-check sweep covers the held-back list over this many frames: vanilla
/// re-checks its whole dirty set every frame (`LevelRenderer.scanChunks`); Haste's
/// re-check runs on visible-set changes instead, so a section that became wanted
/// without one (a sub-2-degree look drift — the look-step test never fires — or a
/// bonus bit arriving) would wait for the next turn. The sweep is the cheap always-on
/// backstop: one `SWEEP_FRACTION`-th of the list per frame, so nothing waits more than
/// a few frames whatever the camera does.
const SWEEP_FRACTION: usize = 8;
/// The sweep's minimum slice: small lists still get fully re-checked every few frames.
const SWEEP_MIN: usize = 64;
/// The eight horizontal neighbour chunks of a section (the ring vanilla
/// `SectionUpdateTracker.hasAllNeighbors` requires loaded for a first build).
const NEIGHBOUR_CHUNKS: [(i32, i32); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

impl WorldFeed {
    /// The always-on backstop of [`Self::promote_wanted`]: re-checks a rotating slice
    /// of `waiting` every frame (a `SWEEP_FRACTION`-th of the list), so a section that
    /// became wanted without a visible-set change — a sub-2-degree look drift, or a
    /// bonus bit arriving next to a newly uploaded section — meshes within a few
    /// frames instead of waiting for the next turn. Vanilla re-checks its whole dirty
    /// set every frame; the slice keeps that cost flat at any render distance.
    pub(super) fn sweep_wanted(&mut self, renderer: &Renderer) {
        if self.waiting.is_empty() {
            return;
        }
        let mut slice = self.waiting.len() / SWEEP_FRACTION + SWEEP_MIN;
        while slice > 0 {
            if self.sweep_cursor >= self.waiting.len() {
                self.sweep_cursor = 0;
            }
            let pos = self.waiting[self.sweep_cursor];
            if renderer.wants_section(pos) {
                self.waiting.swap_remove(self.sweep_cursor);
                self.queue.push(pos);
                self.sort_needed = true;
                self.promoted = true;
            } else {
                self.sweep_cursor += 1;
            }
            slice -= 1;
        }
    }

    /// Moves held-back sections that became wanted back into the queue (a new visible
    /// set or a turned look both change what the renderer wants), or when a chunk a
    /// first-build hold waited on has loaded ([`Self::rescan_waiting`]).
    pub(super) fn promote_wanted(&mut self, renderer: &Renderer) {
        if renderer.want_version() == self.last_want_version && !self.rescan_waiting {
            return;
        }
        self.rescan_waiting = false;
        self.last_want_version = renderer.want_version();
        if self.waiting.is_empty() {
            return;
        }
        let mut i = 0;
        while i < self.waiting.len() {
            if renderer.wants_section(self.waiting[i]) {
                let pos = self.waiting.swap_remove(i);
                self.queue.push(pos);
                self.sort_needed = true;
                self.promoted = true;
            } else {
                i += 1;
            }
        }
    }

    /// Registers a first-build hold's neighbour chunks ([`Self::first_build_waits`]):
    /// the hold waits on the section's neighbour chunks loading.
    pub(super) fn note_first_build_hold(&mut self, pos: SectionPos) {
        for (nx, nz) in NEIGHBOUR_CHUNKS {
            self.first_build_waits
                .insert(haste_core::ChunkPos::new(pos.x + nx, pos.z + nz));
        }
    }
}
