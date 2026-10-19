//! The benchmarks: `--bench` (offscreen, along the fixed camera path) and `--bench-window`
//! (a real window holding a still camera), their samples and the printed/JSON summary.

use super::*;

mod memory;
mod offscreen;
pub(super) mod window;
pub use offscreen::bench;
pub use window::bench_window;
pub(super) use window::WindowBench;

/// The bench camera at normalised time `t` in 0..=1 (t is elapsed/total). The first half is a
/// slow 360-degree turn in place around the spawn eye; the second half flies straight along +X
/// at 10 blocks/s at eye height + 20. `base` is the settle-frame camera; only the camera moves,
/// never the player. Deterministic: same t, same camera.
pub fn bench_camera(t: f32, base: &Camera, total_seconds: f32) -> Camera {
    let half = 0.5_f32;
    let mut camera = *base;
    if t < half {
        // One full turn over the first half.
        camera.yaw = base.yaw + (t / half) * 360.0;
    } else {
        camera.yaw = base.yaw + 360.0;
        // 10 blocks/s for the second half (which is total/2 seconds long).
        let flight_seconds = total_seconds * (t - half);
        camera.pos.x = base.pos.x + 10.0 * flight_seconds as f64;
        camera.pos.y = base.pos.y + 20.0;
    }
    camera
}

/// The windowed bench's camera path (`--bench-path`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BenchPath {
    /// Hold the settle camera still (the original behaviour).
    #[default]
    Still,
    /// Hold, then 180-degree turns at a fixed speed every few seconds, then a straight
    /// flight leg — the same motion every run, so `--premesh off` and `--premesh cone`
    /// runs compare over identical turns.
    Turns,
}

/// The `turns` path camera at `elapsed` seconds into the measured run (deterministic,
/// same path for every run): a 2 s hold, then cycles of a 1 s 180-degree turn (direction
/// alternating) and a 3 s hold for 60% of the run, then a straight flight along +X at
/// 10 blocks/s (the offscreen bench's flight speed), 20 blocks up.
pub fn bench_camera_turns(elapsed: f32, base: &Camera, total_seconds: f32) -> Camera {
    let mut camera = *base;
    let hold = 2.0;
    let turn_span = (total_seconds * 0.6 - hold).max(0.0);
    // The turn state, clamped to the end of the turning phase (the flight leg holds
    // wherever the last turn left the look).
    let t = (elapsed - hold).max(0.0).min(turn_span);
    let cycle = (t / 4.0).floor() as i32;
    let in_cycle = t - 4.0 * cycle as f32;
    let turned = in_cycle.min(1.0) * 180.0;
    let (start, dir) = if cycle % 2 == 0 {
        (0.0, 1.0)
    } else {
        (180.0, -1.0)
    };
    camera.yaw = base.yaw + start + dir * turned;
    if elapsed - hold > turn_span {
        let flight = elapsed - hold - turn_span;
        camera.pos.x = base.pos.x + 10.0 * flight as f64;
        camera.pos.y = base.pos.y + 20.0;
    }
    camera
}

/// One recorded bench frame.
#[derive(Clone, Copy, Debug)]
pub(super) struct BenchSample {
    pub(super) frame_ms: f32,
    pub(super) sections_drawn: u32,
    pub(super) sections_total: u32,
    pub(super) draw_calls: u32,
    pub(super) timings: haste_render::FrameTimings,
    /// WorldFeed::feed time (dirty collection, submit, mesh upload).
    pub(super) feed_ms: f32,
    /// Sections that were not drawn by the previous sample's frame (meshes became visible).
    pub(super) sections_new: u32,
    /// Section meshes uploaded this frame and their vertex+index bytes.
    pub(super) uploads: u32,
    pub(super) upload_bytes: u64,
    /// Terrain indices drawn (six per quad): the vertex-fetch load.
    pub(super) indices_drawn: u32,
    /// Frame index since the measure phase started.
    pub(super) index: u32,
    /// Sections this frame's mesh gate wants that have no mesh yet (pop-in; 0 unless
    /// the bench turned tracking on).
    pub(super) visible_unmeshed: u32,
}

/// The pop-in statistics over a bench's samples: episodes (a run of frames with
/// visible-but-unmeshed sections, framed by frames where everything visible has a
/// mesh), their length in frames, and the visible-unmeshed section-frames overall.
#[derive(Debug, Default)]
pub(super) struct PopinSummary {
    episodes: u32,
    /// Frames from an episode's first unmeshed frame to the first fully meshed one.
    episode_frames: Vec<u32>,
    /// Total frames that had any visible-but-unmeshed section.
    frames_with_unmeshed: u32,
    /// Sum of the per-frame unmeshed counts (section-frames).
    unmeshed_section_frames: u64,
}

/// Aggregates the samples' pop-in series: an episode starts when the count leaves zero
/// and ends when it returns to it.
pub(super) fn popin_summary(samples: &[BenchSample]) -> PopinSummary {
    let mut s = PopinSummary::default();
    let mut in_episode = false;
    let mut episode_len = 0u32;
    for sample in samples {
        if sample.visible_unmeshed > 0 {
            s.frames_with_unmeshed += 1;
            s.unmeshed_section_frames += sample.visible_unmeshed as u64;
            episode_len += 1;
            in_episode = true;
        } else if in_episode {
            s.episodes += 1;
            s.episode_frames.push(episode_len);
            episode_len = 0;
            in_episode = false;
        }
    }
    if in_episode {
        s.episodes += 1;
        s.episode_frames.push(episode_len);
    }
    s
}

impl PopinSummary {
    /// (episodes, average frames per episode, worst episode frames): 0s when no episode
    /// ever started (everything visible was always meshed).
    fn stats(&self) -> (u32, f32, u32) {
        let avg = if self.episode_frames.is_empty() {
            0.0
        } else {
            self.episode_frames.iter().sum::<u32>() as f32 / self.episode_frames.len() as f32
        };
        let worst = self.episode_frames.iter().copied().max().unwrap_or(0);
        (self.episodes, avg, worst)
    }
}

/// Summary of a bench run: aggregate frame times plus the renderer's phase and pass averages.
#[derive(Debug)]
struct BenchSummary {
    frames: u32,
    avg_ms: f32,
    /// 1% low: the 99th percentile frame time.
    p99_ms: f32,
    worst_ms: f32,
    sections_drawn: f32,
    sections_total: f32,
    draw_calls: f32,
    /// Terrain indices drawn per frame (six per quad): the vertex-fetch load.
    indices_drawn: f32,
    prepare_ms: f32,
    acquire_ms: f32,
    cull_ms: f32,
    encode_ms: f32,
    submit_ms: f32,
    present_ms: f32,
    /// GPU time of the main, hand and GUI passes.
    gpu_ms: [f32; 3],
    /// GPU time of the main pass's sub-passes: sky, opaque terrain (solid + cutout-mipped),
    /// cutout terrain, translucent terrain, entities, particles, overlays. Timing.rs carries
    /// the solid and cutout-mipped phases separately (queries are written once per pass);
    /// the bench reports them as one opaque phase like vanilla's ChunkSectionLayerGroup.
    gpu_sub_ms: [f32; 7],
    /// Pop-in (visible-but-unmeshed sections over the run) and the pre-mesh verdicts.
    popin: PopinSummary,
    premeshed: u32,
    premesh_hits: u32,
    premesh_waste: u32,
}

impl BenchSummary {
    fn fps(&self) -> f32 {
        if self.avg_ms > 0.0 {
            1000.0 / self.avg_ms
        } else {
            0.0
        }
    }

    /// One block of text, printed and also written to `--bench-out` as JSON.
    fn text(&self, meta: &BenchMeta) -> String {
        format!(
            "bench: {} frames in {:.0} s\n\
             frame time: avg {:.2} ms ({:.0} FPS), 1% low {:.2} ms, worst {:.2} ms\n\
             breakdown ms: prepare {:.2} acquire {:.2} cull {:.2} encode {:.2} submit {:.2} present {:.2} | gpu main {:.2} hand {:.2} gui {:.2}\n\
             gpu main ms: sky {:.3} opaque {:.3} cutout {:.3} translucent {:.3} entities {:.3} particles {:.3} overlays {:.3}\n\
             sections drawn {}/{} avg, draw calls {:.0} avg, indices {:.0} avg (quads {:.0})\n\
             pop-in: {} episodes, {:.1} frames avg to mesh ({} worst), {} frames with unmeshed, {} visible-unmeshed section-frames\n\
             premesh: {} pre-meshed, {} hits ({:.0}%), {} wasted\n\
             render distance {} (requested {}), {}x{}, adapter {}",
            self.frames,
            meta.seconds,
            self.avg_ms,
            self.fps(),
            self.p99_ms,
            self.worst_ms,
            self.prepare_ms,
            self.acquire_ms,
            self.cull_ms,
            self.encode_ms,
            self.submit_ms,
            self.present_ms,
            self.gpu_ms[0],
            self.gpu_ms[1],
            self.gpu_ms[2],
            self.gpu_sub_ms[0],
            self.gpu_sub_ms[1],
            self.gpu_sub_ms[2],
            self.gpu_sub_ms[3],
            self.gpu_sub_ms[4],
            self.gpu_sub_ms[5],
            self.gpu_sub_ms[6],
            self.sections_drawn,
            self.sections_total,
            self.draw_calls,
            self.indices_drawn,
            self.indices_drawn / 6.0,
            self.popin.stats().0,
            self.popin.stats().1,
            self.popin.stats().2,
            self.popin.frames_with_unmeshed,
            self.popin.unmeshed_section_frames,
            self.premeshed,
            self.premesh_hits,
            if self.premeshed > 0 {
                100.0 * self.premesh_hits as f32 / self.premeshed as f32
            } else {
                0.0
            },
            self.premesh_waste,
            meta.render_distance,
            meta.requested_render_distance,
            meta.width,
            meta.height,
            meta.adapter,
        )
    }
}

/// The `count` slowest sample indices, slowest first (earliest frame breaks ties). The
/// allocation is at the end of the bench, after all frames are recorded.
fn slowest_indices(samples: &[BenchSample], count: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..samples.len()).collect();
    order.sort_by(|&a, &b| {
        samples[b]
            .frame_ms
            .total_cmp(&samples[a].frame_ms)
            .then(samples[a].index.cmp(&samples[b].index))
    });
    order.truncate(count);
    order
}

/// The `count` slowest bench frames with their full breakdown: frame index and time, the
/// renderer phases (prepare/cull/encode/submit, GPU main pass), the feed phase, mesh uploads
/// (count and bytes) and how many sections the frame drew that the previous one did not.
/// GPU pass times arrive a frame or two late (timestamp readback), so they are the latest
/// available at that frame, not necessarily that exact frame's passes.
fn print_slowest_frames(samples: &[BenchSample]) {
    const COUNT: usize = 20;
    let order = slowest_indices(samples, COUNT);
    if order.is_empty() {
        return;
    }
    println!("slowest {} frames (of {}):", order.len(), samples.len());
    println!("  frame   ms | feed prep cull enc sub gpuM | up #/KB sect+ drawn/total calls");
    for &i in &order {
        let s = &samples[i];
        println!(
            "  {:5} {:5.2} | {:5.2} {:5.2} {:5.2} {:4.2} {:5.2} {:5.2} | {:3} {:5.0} {:4} {}/{} {}",
            s.index,
            s.frame_ms,
            s.feed_ms,
            s.timings.prepare_ms,
            s.timings.cull_ms,
            s.timings.encode_ms,
            s.timings.submit_ms,
            s.timings.gpu_ms[0],
            s.uploads,
            s.upload_bytes as f32 / 1024.0,
            s.sections_new,
            s.sections_drawn,
            s.sections_total,
            s.draw_calls,
        );
    }
}

/// Run metadata printed with the summary.
struct BenchMeta {
    seconds: f32,
    /// The effective render distance: the requested one capped by the server's view distance.
    render_distance: i32,
    requested_render_distance: u8,
    width: u32,
    height: u32,
    adapter: String,
}

/// Percentile of sorted ascending `values` in 0..=1 (linear interpolation between samples,
/// nearest at the ends). Matches `haste_testkit::Bench::percentile_frame_ms`'s definition.
fn percentile(sorted: &[f32], p: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let p = p.clamp(0.0, 1.0);
    let idx = (sorted.len() as f32 - 1.0) * p;
    let lo = idx.floor() as usize;
    let hi = idx.ceil() as usize;
    let frac = idx - lo as f32;
    sorted[lo] * (1.0 - frac) + sorted[hi.min(sorted.len() - 1)] * frac
}

/// Aggregates the samples into a summary (avg, 99th percentile, phase averages), plus
/// the renderer's pre-mesh verdicts (`premeshed`, hits, waste).
fn bench_summary(samples: &[BenchSample], premesh: (u32, u32, u32)) -> BenchSummary {
    let n = samples.len().max(1) as f32;
    let mut times: Vec<f32> = samples.iter().map(|s| s.frame_ms).collect();
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    fn sum(samples: &[BenchSample], n: f32, f: impl Fn(&BenchSample) -> f32) -> f32 {
        samples.iter().map(f).sum::<f32>() / n
    }
    let t = |f: fn(&haste_render::FrameTimings) -> f32| sum(samples, n, |s| f(&s.timings));
    let gpu = |i: usize| sum(samples, n, |s| s.timings.gpu_ms[i]);
    // Solid + cutout-mipped (renderer sub-passes 1 and 3) fold into "opaque".
    let opaque = sum(samples, n, |s| {
        s.timings.gpu_sub_ms[1] + s.timings.gpu_sub_ms[3]
    });
    let gpu_sub = |i: usize| sum(samples, n, |s| s.timings.gpu_sub_ms[i]);
    BenchSummary {
        frames: samples.len() as u32,
        avg_ms: sum(samples, n, |s| s.frame_ms),
        p99_ms: percentile(&times, 0.99),
        worst_ms: times.last().copied().unwrap_or(0.0),
        sections_drawn: sum(samples, n, |s| s.sections_drawn as f32),
        sections_total: sum(samples, n, |s| s.sections_total as f32),
        draw_calls: sum(samples, n, |s| s.draw_calls as f32),
        indices_drawn: sum(samples, n, |s| s.indices_drawn as f32),
        prepare_ms: t(|x| x.prepare_ms),
        acquire_ms: t(|x| x.acquire_ms),
        cull_ms: t(|x| x.cull_ms),
        encode_ms: t(|x| x.encode_ms),
        submit_ms: t(|x| x.submit_ms),
        present_ms: t(|x| x.present_ms),
        gpu_ms: [gpu(0), gpu(1), gpu(2)],
        gpu_sub_ms: [
            gpu_sub(0),
            opaque,
            gpu_sub(2),
            gpu_sub(4),
            gpu_sub(5),
            gpu_sub(6),
            gpu_sub(7),
        ],
        popin: popin_summary(samples),
        premeshed: premesh.0,
        premesh_hits: premesh.1,
        premesh_waste: premesh.2,
    }
}

#[cfg(test)]
mod tests;
