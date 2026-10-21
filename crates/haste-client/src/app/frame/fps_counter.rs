//! The title bar's frame-time statistics: frames per second and the per-phase breakdown,
//! reported twice a second. Split out of `frame.rs`; a pure move.

use super::*;

/// Frame-time statistics for the title bar, reported twice a second.
pub(crate) struct FpsCounter {
    since: Instant,
    frames: u32,
    worst_ms: f32,
    last: Instant,
    /// Smoothed frames per second (the F3 overlay reads it every frame).
    pub(crate) fps: f32,
    /// Per-phase sums since the last breakdown: feed, entities, render, and the renderer's
    /// own phases; logged every 2 s.
    phase_sum: [f32; 3],
    render_sum: haste_render::FrameTimings,
    phase_frames: u32,
    phase_since: Instant,
}

impl FpsCounter {
    pub(crate) fn new() -> Self {
        let now = Instant::now();
        Self {
            since: now,
            frames: 0,
            worst_ms: 0.0,
            last: now,
            fps: 0.0,
            phase_sum: [0.0; 3],
            render_sum: Default::default(),
            phase_frames: 0,
            phase_since: now,
        }
    }

    pub(super) fn phases(
        &mut self,
        feed: f32,
        entities: f32,
        render: f32,
        t: haste_render::FrameTimings,
    ) {
        self.phase_sum[0] += feed;
        self.phase_sum[1] += entities;
        self.phase_sum[2] += render;
        let s = &mut self.render_sum;
        s.prepare_ms += t.prepare_ms;
        s.acquire_ms += t.acquire_ms;
        s.cull_ms += t.cull_ms;
        s.encode_ms += t.encode_ms;
        s.submit_ms += t.submit_ms;
        s.present_ms += t.present_ms;
        for i in 0..3 {
            s.gpu_ms[i] += t.gpu_ms[i];
        }
        self.phase_frames += 1;
    }

    /// Average per-phase milliseconds over the last 2 s.
    pub(super) fn breakdown(&mut self, renderer: &Renderer) -> Option<String> {
        if self.phase_since.elapsed() < Duration::from_secs(2) || self.phase_frames == 0 {
            return None;
        }
        let n = self.phase_frames as f32;
        let s = self.render_sum;
        let size = renderer.size();
        let line = format!(
            "frame ms: feed {:.2} entities {:.2} render {:.2} (prepare {:.2} acquire {:.2} cull {:.2} encode {:.2} submit {:.2} present {:.2}) gpu main {:.2} hand {:.2} gui {:.2}; {}x{} {:?}",
            self.phase_sum[0] / n,
            self.phase_sum[1] / n,
            self.phase_sum[2] / n,
            s.prepare_ms / n,
            s.acquire_ms / n,
            s.cull_ms / n,
            s.encode_ms / n,
            s.submit_ms / n,
            s.present_ms / n,
            s.gpu_ms[0] / n,
            s.gpu_ms[1] / n,
            s.gpu_ms[2] / n,
            size.0,
            size.1,
            renderer.present_mode(),
        );
        self.phase_sum = [0.0; 3];
        self.render_sum = Default::default();
        self.phase_frames = 0;
        self.phase_since = Instant::now();
        Some(line)
    }

    pub(crate) fn frame(&mut self, stats: &FrameStats) -> Option<String> {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32() * 1000.0;
        self.last = now;
        self.frames += 1;
        self.worst_ms = self.worst_ms.max(dt);
        // Exponential moving average: smooth enough for F3's every-frame read, settles within
        // a second of a real change.
        let instant = 1000.0 / dt.max(0.001);
        self.fps = if self.fps == 0.0 {
            instant
        } else {
            self.fps * 0.95 + instant * 0.05
        };
        let span = now.duration_since(self.since).as_secs_f32();
        if span < 0.5 {
            return None;
        }
        let line = format!(
            "{:.0} FPS, {:.2} ms avg, {:.2} ms worst, sections {}/{}, pending {}",
            self.frames as f32 / span,
            span * 1000.0 / self.frames as f32,
            self.worst_ms,
            stats.chunk_sections_drawn,
            stats.chunk_sections_total,
            stats.meshes_pending
        );
        self.since = now;
        self.frames = 0;
        self.worst_ms = 0.0;
        Some(line)
    }
}
