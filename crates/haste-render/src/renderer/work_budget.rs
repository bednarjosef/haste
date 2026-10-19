//! The background-work budget (`--work-budget`): how much render-thread time inside a
//! frame may go to mesh uploads. `fixed` is exactly the historical behaviour (2 ms per
//! frame, 8 ms while a mesh backlog stands); `adaptive` predicts the next frame's cost
//! from the recent frames and gives the uploads the time left to the frame-time target,
//! so a burst of work no longer lands on whichever frame it arrived in.
//!
//! The prediction is an exponentially weighted average plus variance per phase: the
//! frame's CPU cost (the feed phase, which contains the uploads, plus the render call)
//! and the GPU main pass. The CPU cost is the side the uploads compete with; the GPU
//! time is a floor for the frame interval (uploads that fit under it are free), so the
//! budget is `max(target, predicted GPU) - predicted CPU`, clamped to a floor (the
//! backlog always drains, never slower than `fixed`'s 2 ms) and a cap (one frame never
//! stalls more than `fixed`'s 8 ms).

use std::time::Duration;

use super::Renderer;

#[cfg(test)]
mod tests;

/// The budget mode (`--work-budget`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkBudgetMode {
    /// The historical fixed budgets: 2 ms per frame, 8 ms while a backlog stands.
    #[default]
    Fixed,
    /// Predict the next frame's cost and budget the uploads to the frame-time target.
    Adaptive,
}

/// `fixed`'s per-frame upload budget.
pub(crate) const FIXED_MS: f32 = 2.0;
/// `fixed`'s budget while a mesh backlog stands (pending > [`BACKLOG_THRESHOLD`]).
pub(crate) const BACKLOG_MS: f32 = 8.0;
/// Pending sections above which `fixed` uses [`BACKLOG_MS`].
pub(crate) const BACKLOG_THRESHOLD: usize = 1024;
/// `adaptive`'s floor: the backlog always drains at least at `fixed`'s base rate.
const FLOOR_MS: f32 = FIXED_MS;
/// `adaptive`'s cap: one frame never stalls more than `fixed`'s backlog budget.
const CAP_MS: f32 = BACKLOG_MS;
/// Time constant of the frame-cost averages (seconds): long enough that one hitch does
/// not collapse the budget, short enough that a turn's burst dominates it.
const TAU: f32 = 0.25;
/// The prediction's safety margin: one standard deviation of the CPU cost added to the
/// average (a spiky frame cost leaves less room than a smooth one).
const SIGMA_WEIGHT: f32 = 1.0;
/// The GPU prediction's margin (half a standard deviation: the GPU side only lifts the
/// target, so a full sigma would starve the budget for jitter that overlaps the CPU).
const GPU_SIGMA_WEIGHT: f32 = 0.5;

/// One predicted quantity: an exponentially weighted average and variance.
#[derive(Clone, Copy, Debug, Default)]
struct Ewma {
    mean: f32,
    m2: f32,
}

impl Ewma {
    /// Folds `x` in over `dt` seconds since the previous sample.
    fn update(&mut self, x: f32, dt: f32) {
        let a = 1.0 - (-dt / TAU).exp();
        let dev = x - self.mean;
        self.mean += a * dev;
        self.m2 += a * (dev * dev - self.m2);
    }

    fn sigma(&self) -> f32 {
        self.m2.max(0.0).sqrt()
    }

    /// The predicted value: the average plus `weight` standard deviations.
    fn predict(&self, weight: f32) -> f32 {
        self.mean + weight * self.sigma()
    }
}

/// The renderer's work-budget state: the mode, the frame-cost predictions and the
/// frame-time target (`None`: the rolling average frame interval).
pub(super) struct WorkBudget {
    mode: WorkBudgetMode,
    /// The frame's CPU cost (feed phase incl. uploads, plus the render call), ms.
    cpu: Ewma,
    /// The GPU main pass time, ms (0 without timestamp queries).
    gpu: Ewma,
    /// The frame-to-frame interval, ms.
    interval: Ewma,
    /// The frame-time target from the client (the display's refresh interval, or the
    /// user's max FPS); `None` while unthrottled, then the rolling interval is the
    /// target: never make the next frame worse than the recent ones.
    target: Option<f32>,
    /// The last computed budget, for `FrameStats`.
    last_budget_ms: f32,
}

impl WorkBudget {
    pub(super) fn new() -> WorkBudget {
        WorkBudget {
            mode: WorkBudgetMode::Fixed,
            cpu: Ewma::default(),
            gpu: Ewma::default(),
            interval: Ewma::default(),
            target: None,
            last_budget_ms: FIXED_MS,
        }
    }

    /// The budget mode (`--work-budget`).
    pub(super) fn mode(&self) -> WorkBudgetMode {
        self.mode
    }

    /// Sets the budget mode (the renderer's `set_work_budget`).
    pub(super) fn set_mode(&mut self, mode: WorkBudgetMode) {
        self.mode = mode;
    }

    /// The frame-time target (`None`: unthrottled, the rolling interval is the target).
    pub(super) fn set_target(&mut self, target: Option<Duration>) {
        self.target = target.map(|t| t.as_secs_f32() * 1000.0);
    }

    /// One rendered frame's observation (called once per frame, before the feed):
    /// `interval_ms` since the previous frame, the last render call's `render_ms` and
    /// the feed phase's `feed_ms` (the two make the frame's CPU cost; the uploads are
    /// inside `feed_ms`, so the budget balances itself against the target), and the GPU
    /// main pass time of the last readback.
    pub(super) fn note_frame(
        &mut self,
        interval_ms: f32,
        render_ms: f32,
        feed_ms: f32,
        gpu_main_ms: f32,
    ) {
        let dt = (interval_ms / 1000.0).clamp(0.0005, 0.25);
        self.cpu.update(render_ms + feed_ms, dt);
        self.gpu.update(gpu_main_ms, dt);
        self.interval.update(interval_ms, dt);
    }

    /// The upload budget for this frame, ms, given the mesh backlog (`pending`).
    pub(super) fn budget_ms(&mut self, pending: usize) -> f32 {
        let budget = match self.mode {
            WorkBudgetMode::Fixed => {
                if pending > BACKLOG_THRESHOLD {
                    BACKLOG_MS
                } else {
                    FIXED_MS
                }
            }
            WorkBudgetMode::Adaptive => {
                let target = self.target.unwrap_or(self.interval.mean);
                let frame = target.max(self.gpu.predict(GPU_SIGMA_WEIGHT));
                (frame - self.cpu.predict(SIGMA_WEIGHT)).clamp(FLOOR_MS, CAP_MS)
            }
        };
        self.last_budget_ms = budget;
        budget
    }

    /// The budget the last `budget_ms` call computed, for `FrameStats`.
    pub(super) fn last_budget_ms(&self) -> f32 {
        self.last_budget_ms
    }
}

impl Renderer {
    /// The background-work budget mode (`--work-budget`): `fixed` is exactly the
    /// historical 2 ms / 8 ms budgets, `adaptive` predicts the next frame's cost and
    /// gives the uploads the time left to the frame-time target.
    pub fn set_work_budget(&mut self, mode: WorkBudgetMode) {
        self.work_budget.set_mode(mode);
    }

    /// The budget mode in use.
    pub fn work_budget_mode(&self) -> WorkBudgetMode {
        self.work_budget.mode
    }

    /// The frame-time target the adaptive budget works to (`None`: unthrottled, then the
    /// rolling average frame interval is the target). The client sets it to the
    /// display's refresh interval while vsync is on, or the user's max-FPS interval.
    pub fn set_work_budget_target(&mut self, target: Option<Duration>) {
        self.work_budget.set_target(target);
    }

    /// One rendered frame's observation for the adaptive budget (called once per frame,
    /// before the feed): the interval since the previous frame, the last render call's
    /// time and the last feed phase's time. The GPU main pass time comes from the
    /// renderer's own latest timing.
    pub fn note_frame(&mut self, interval_ms: f32, render_ms: f32, feed_ms: f32) {
        self.work_budget
            .note_frame(interval_ms, render_ms, feed_ms, self.timings.gpu_ms[0]);
    }
}
