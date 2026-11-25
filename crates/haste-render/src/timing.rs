// Frame timing: CPU phases of `Renderer::render` and GPU time per pass from timestamp queries
// (when the adapter has TIMESTAMP_QUERY). GPU results arrive a frame or two late; one readback
// is in flight at a time.
//
// The main pass is further split into sub-passes with bracketing timestamp writes inside the
// pass, so a slow main pass can be attributed without a profiler. Sub-pass times are
// wall-clock spans between their begin/end timestamps and overlap nothing (they partition
// the pass's draw calls), but their sum can exceed the pass span on tiled GPUs that reorder
// work; on RADV it matches. Solid and CutoutMipped terrain share a pipeline and are reported
// together as "opaque"; a wgpu query may only be written once per pass, so each sub-pass
// bracket must cover all its draws including arena re-runs.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Passes with GPU timestamps, in query order (begin, end per pass).
pub const PASSES: [&str; 3] = ["main", "hand", "gui"];

/// Sub-passes of the main pass. "solid" and "cutout_mipped" are separate brackets (a wgpu
/// query may only be written once per pass; they interleave per arena in the layer-major
/// draw order) but reported as one "opaque" phase in the bench summary, matching vanilla's
/// ChunkSectionLayerGroup.OPAQUE.
pub const MAIN_SUBPASSES: [&str; 8] = [
    "sky",
    "solid",
    "cutout",
    "cutout_mipped",
    "translucent",
    "entities",
    "particles",
    "overlays",
];

/// Sub-pass indices within [`MAIN_SUBPASSES`].
pub const SKY: u32 = 0;
/// The first terrain layer bracket (solid): `close_terrain` closes 1..=4.
pub const FIRST_LAYER: u32 = 1;
pub const ENTITIES: u32 = 5;
pub const PARTICLES: u32 = 6;
pub const OVERLAYS: u32 = 7;

/// The main pass's sub-pass bracket state: which brackets are open and which ran this frame.
/// A wgpu query may only be written once per pass, so exactly one bracket per sub-pass; the
/// terrain layers (1..=4) are layer-major, so a layer change or the entity insertion closes
/// the open terrain bracket, and the last layer's bracket is closed after the draw loop (it
/// never sees a layer change). The entities bracket closes right after the entity draws, so
/// the translucent terrain below is not timed as "entities".
#[derive(Default)]
pub struct SubpassBrackets {
    open: [bool; 8],
    ran: [bool; 8],
}

impl SubpassBrackets {
    /// Opens sub-pass `i`'s bracket. `t` None: no timer this frame, only bookkeeping.
    /// `ran` only marks brackets whose timestamp was written (sub-passes are sampled every
    /// [`SUBPASS_SAMPLE_FRAMES`]th frame), so the readback never reads an unwritten query.
    pub fn begin(&mut self, pass: &mut wgpu::RenderPass<'_>, t: Option<&GpuTimer>, i: u32) {
        if let Some(t) = t {
            self.ran[i as usize] |= t.begin_subpass(pass, i);
        }
        self.open[i as usize] = true;
    }

    /// Closes sub-pass `i`'s bracket if one is open.
    pub fn end(&mut self, pass: &mut wgpu::RenderPass<'_>, t: Option<&GpuTimer>, i: u32) {
        if self.open[i as usize] {
            if let Some(t) = t {
                t.end_subpass(pass, i);
            }
            self.open[i as usize] = false;
        }
    }

    /// Closes any open terrain bracket (the solid..translucent layers, 1..=4).
    pub fn close_terrain(&mut self, pass: &mut wgpu::RenderPass<'_>, t: Option<&GpuTimer>) {
        for s in FIRST_LAYER..ENTITIES {
            self.end(pass, t, s);
        }
    }

    /// Begins terrain layer `sub`'s bracket, closing the open terrain bracket first.
    pub fn begin_terrain_layer(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        t: Option<&GpuTimer>,
        sub: u32,
    ) {
        self.close_terrain(pass, t);
        self.begin(pass, t, sub);
    }

    /// Begins the entities bracket after the opaque terrain (vanilla
    /// `LevelRenderer.renderLevel` draws the entities before the translucent terrain).
    pub fn begin_entities(&mut self, pass: &mut wgpu::RenderPass<'_>, t: Option<&GpuTimer>) {
        self.close_terrain(pass, t);
        self.begin(pass, t, ENTITIES);
    }

    /// Begins the particles bracket after all terrain and entities.
    pub fn begin_particles(&mut self, pass: &mut wgpu::RenderPass<'_>, t: Option<&GpuTimer>) {
        self.close_terrain(pass, t);
        self.end(pass, t, ENTITIES);
        self.begin(pass, t, PARTICLES);
    }

    /// Begins the overlays bracket (weather, clouds) after the particles.
    pub fn begin_overlays(&mut self, pass: &mut wgpu::RenderPass<'_>, t: Option<&GpuTimer>) {
        self.end(pass, t, PARTICLES);
        self.begin(pass, t, OVERLAYS);
    }

    /// Which sub-passes ran, for `GpuTimer::resolve`.
    pub fn ran(&self) -> [bool; 8] {
        self.ran
    }
}

/// One frame's timings in milliseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameTimings {
    /// Entity posing, sky, overlay and hand preparation.
    pub prepare_ms: f32,
    /// Waiting for the surface texture (`get_current_texture`); 0 offscreen.
    pub acquire_ms: f32,
    /// Cave-culling walk, frustum culling, sorting, indirect/origin uploads.
    pub cull_ms: f32,
    /// Recording all passes (including GUI building).
    pub encode_ms: f32,
    /// `queue.submit`.
    pub submit_ms: f32,
    /// `queue.present`.
    pub present_ms: f32,
    /// GPU time of the main, hand and GUI passes (latest available; 0 without timestamps).
    pub gpu_ms: [f32; 3],
    /// GPU time of the main pass's sub-passes (see [`MAIN_SUBPASSES`]); 0 without timestamps
    /// or when the sub-pass did not run.
    pub gpu_sub_ms: [f32; 8],
}

pub struct GpuTimer {
    pub query_set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    mapped: Arc<AtomicBool>,
    in_flight: bool,
    period_ns: f32,
    /// Which passes ran in the frame being read back.
    ran: [bool; 3],
    /// Which main-pass sub-passes ran in the frame being read back.
    ran_sub: [bool; 8],
    pub latest: [f32; 3],
    pub latest_sub: [f32; 8],
    /// Frames rendered so far; sub-pass brackets run on multiples of
    /// [`SUBPASS_SAMPLE_FRAMES`].
    frame: u32,
    /// Whether the sub-pass brackets are written at all: off by default, since a sampled
    /// frame costs ~2.5 ms more GPU time and one frame in 64 lands in every p99 (vanilla
    /// writes no timestamps); the offscreen bench turns it on for its breakdown, and
    /// `HASTE_GPU_BREAKDOWN=1` does for any run.
    pub sample_subpasses: bool,
}

/// Total query slots: 2 per pass plus 2 per main-pass sub-pass bracket.
const SUB_BASE: u32 = (PASSES.len() as u32) * 2;
const QUERY_COUNT: u32 = SUB_BASE + (MAIN_SUBPASSES.len() as u32) * 2;

/// Sub-pass brackets are written only every Nth frame: on RADV (Radeon 860M) each
/// inside-pass timestamp write costs ~0.15 ms of GPU time, so the 16 bracket writes
/// a frame costs ~2.5 ms — far too much to pay every frame just for attribution
/// (measured at rd 32: frame interval 11.6 ms with brackets, 9.1 ms without). The
/// pass begin/end timestamps stay every frame; they cost nothing measurable.
const SUBPASS_SAMPLE_FRAMES: u32 = 64;

impl GpuTimer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<GpuTimer> {
        if !device.features().contains(
            wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES,
        ) {
            return None;
        }
        if std::env::var_os("HASTE_NO_GPU_TIMER").is_some() {
            return None;
        }
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("pass-timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: QUERY_COUNT,
        });
        let size = QUERY_COUNT as u64 * 8;
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("timestamps-resolve"),
            size,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("timestamps-readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Some(GpuTimer {
            query_set,
            resolve,
            readback,
            mapped: Arc::new(AtomicBool::new(false)),
            in_flight: false,
            period_ns: queue.get_timestamp_period(),
            ran: [false; 3],
            ran_sub: [false; 8],
            latest: [0.0; 3],
            latest_sub: [0.0; 8],
            frame: 0,
            sample_subpasses: std::env::var_os("HASTE_GPU_BREAKDOWN").is_some(),
        })
    }

    /// Timestamp writes for pass `i`, or None while the previous readback is still pending.
    pub fn writes(&self, i: u32) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        (!self.in_flight).then(|| wgpu::RenderPassTimestampWrites {
            query_set: &self.query_set,
            beginning_of_pass_write_index: Some(i * 2),
            end_of_pass_write_index: Some(i * 2 + 1),
        })
    }

    /// Timestamp writes for pass `i` when it is recorded as several render passes (the
    /// main pass around the GPU occlusion culling): the first part writes the beginning,
    /// the last the end.
    pub fn writes_part(
        &self,
        i: u32,
        first: bool,
        last: bool,
    ) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        (!self.in_flight && (first || last)).then(|| wgpu::RenderPassTimestampWrites {
            query_set: &self.query_set,
            beginning_of_pass_write_index: first.then_some(i * 2),
            end_of_pass_write_index: last.then_some(i * 2 + 1),
        })
    }

    /// Begin sub-pass `i`'s bracket: one timestamp write inside `pass`. Each query may be
    /// written at most once per pass, so a bracket that is already open must not begin again.
    /// No-op while the previous readback is still pending or on a frame that does not sample
    /// sub-passes (every [`SUBPASS_SAMPLE_FRAMES`]th); returns whether the bracket was
    /// written, so the caller's `ran_sub` flag matches what the readback will contain.
    pub fn begin_subpass(&self, pass: &mut wgpu::RenderPass<'_>, i: u32) -> bool {
        if self.samples_subpasses() {
            pass.write_timestamp(&self.query_set, SUB_BASE + i * 2);
            true
        } else {
            false
        }
    }

    /// End sub-pass `i`'s bracket. No-op unless this frame samples sub-passes (the same
    /// condition as [`begin_subpass`], so the pair always matches).
    pub fn end_subpass(&self, pass: &mut wgpu::RenderPass<'_>, i: u32) {
        if self.samples_subpasses() {
            pass.write_timestamp(&self.query_set, SUB_BASE + i * 2 + 1);
        }
    }

    fn samples_subpasses(&self) -> bool {
        self.sample_subpasses && !self.in_flight && self.frame.is_multiple_of(SUBPASS_SAMPLE_FRAMES)
    }

    /// Resolves this frame's queries (call after the passes, before finishing the encoder).
    /// Passes that did not run this frame keep stale values; they are zeroed first.
    pub fn resolve(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        ran: [bool; 3],
        ran_sub: [bool; 8],
    ) -> bool {
        self.frame = self.frame.wrapping_add(1);
        if self.in_flight {
            return false;
        }
        // One resolve of the whole set (resolve offsets must be 256-byte aligned); results of
        // passes that did not run are ignored when read.
        encoder.resolve_query_set(&self.query_set, 0..QUERY_COUNT, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.readback, 0, self.readback.size());
        self.ran = ran;
        self.ran_sub = ran_sub;
        true
    }

    /// Starts mapping the readback after submit.
    pub fn after_submit(&mut self, resolved: bool) {
        if !resolved {
            return;
        }
        self.in_flight = true;
        let flag = self.mapped.clone();
        self.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| {
                flag.store(r.is_ok(), Ordering::Release)
            });
    }

    /// Reads finished results (non-blocking).
    pub fn poll(&mut self, device: &wgpu::Device) {
        if !self.in_flight {
            return;
        }
        let _ = device.poll(wgpu::PollType::Poll);
        if !self.mapped.swap(false, Ordering::Acquire) {
            return;
        }
        {
            let Ok(data) = self.readback.slice(..).get_mapped_range() else {
                self.in_flight = false;
                return;
            };
            let t: &[u64] = bytemuck::cast_slice(&data);
            let ms = |lo: u64, hi: u64| (hi.saturating_sub(lo)) as f32 * self.period_ns / 1e6;
            for i in 0..PASSES.len() {
                self.latest[i] = if self.ran[i] && t[i * 2 + 1] >= t[i * 2] {
                    ms(t[i * 2], t[i * 2 + 1])
                } else {
                    0.0
                };
            }
            for i in 0..MAIN_SUBPASSES.len() {
                let (lo, hi) = (SUB_BASE as usize + i * 2, SUB_BASE as usize + i * 2 + 1);
                // Not run this readback (brackets are only sampled every
                // SUBPASS_SAMPLE_FRAMES): keep the last sampled value, like the
                // pass times keep theirs while a readback is pending.
                if self.ran_sub[i] && t[hi] >= t[lo] {
                    self.latest_sub[i] = ms(t[lo], t[hi]);
                }
            }
        }
        self.readback.unmap();
        self.in_flight = false;
    }
}
