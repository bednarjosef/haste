// Frame timing: CPU phases of `Renderer::render` and GPU time per pass from timestamp queries
// (when the adapter has TIMESTAMP_QUERY). GPU results arrive a frame or two late; one readback
// is in flight at a time.
//
// The main pass is further split into sub-passes (sky, opaque terrain, cutout, translucent,
// entities, particles, overlays) with bracketing timestamp writes inside the pass, so a slow
// main pass can be attributed without a profiler. Sub-pass times are wall-clock spans between
// their begin/end timestamps and overlap nothing (they partition the pass's draw calls), but
// their sum can exceed the pass span on tiled GPUs that reorder work; on RADV it matches.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Passes with GPU timestamps, in query order (begin, end per pass).
pub const PASSES: [&str; 3] = ["main", "hand", "gui"];

/// Sub-passes of the main pass, in draw order: sky, opaque terrain, cutout terrain,
/// translucent terrain, entities, particles, overlays (clouds and weather).
pub const MAIN_SUBPASSES: [&str; 7] = [
    "sky",
    "opaque",
    "cutout",
    "translucent",
    "entities",
    "particles",
    "overlays",
];

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
    pub gpu_sub_ms: [f32; 7],
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
    ran_sub: [bool; 7],
    pub latest: [f32; 3],
    pub latest_sub: [f32; 7],
}

/// Total query slots: 2 per pass plus 2 per main-pass sub-pass bracket.
const SUB_BASE: u32 = (PASSES.len() as u32) * 2;
const QUERY_COUNT: u32 = SUB_BASE + (MAIN_SUBPASSES.len() as u32) * 2;

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
            ran_sub: [false; 7],
            latest: [0.0; 3],
            latest_sub: [0.0; 7],
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

    /// Begin sub-pass `i`'s bracket: one timestamp write inside `pass`. Each query may be
    /// written at most once per pass, so a bracket that is already open must not begin again.
    /// No-op while the previous readback is still pending.
    pub fn begin_subpass(&self, pass: &mut wgpu::RenderPass<'_>, i: u32) {
        if !self.in_flight {
            pass.write_timestamp(&self.query_set, SUB_BASE + i * 2);
        }
    }

    /// End sub-pass `i`'s bracket.
    pub fn end_subpass(&self, pass: &mut wgpu::RenderPass<'_>, i: u32) {
        if !self.in_flight {
            pass.write_timestamp(&self.query_set, SUB_BASE + i * 2 + 1);
        }
    }

    /// Resolves this frame's queries (call after the passes, before finishing the encoder).
    /// Passes that did not run this frame keep stale values; they are zeroed first.
    pub fn resolve(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        ran: [bool; 3],
        ran_sub: [bool; 7],
    ) -> bool {
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
                self.latest_sub[i] = if self.ran_sub[i] && t[hi] >= t[lo] {
                    ms(t[lo], t[hi])
                } else {
                    0.0
                };
            }
        }
        self.readback.unmap();
        self.in_flight = false;
    }
}
