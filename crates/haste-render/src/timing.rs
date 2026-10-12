// Frame timing: CPU phases of `Renderer::render` and GPU time per pass from timestamp queries
// (when the adapter has TIMESTAMP_QUERY). GPU results arrive a frame or two late; one readback
// is in flight at a time.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Passes with GPU timestamps, in query order (begin, end per pass).
pub const PASSES: [&str; 3] = ["main", "hand", "gui"];

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
    pub latest: [f32; 3],
}

impl GpuTimer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<GpuTimer> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let count = PASSES.len() as u32 * 2;
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("pass-timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count,
        });
        let size = count as u64 * 8;
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
            latest: [0.0; 3],
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

    /// Resolves this frame's queries (call after the passes, before finishing the encoder).
    /// Passes that did not run this frame keep stale values; they are zeroed first.
    pub fn resolve(&mut self, encoder: &mut wgpu::CommandEncoder, ran: [bool; 3]) -> bool {
        if self.in_flight {
            return false;
        }
        // One resolve of the whole set (resolve offsets must be 256-byte aligned); results of
        // passes that did not run are ignored when read.
        let count = PASSES.len() as u32 * 2;
        encoder.resolve_query_set(&self.query_set, 0..count, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.readback, 0, self.readback.size());
        self.ran = ran;
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
            for i in 0..PASSES.len() {
                self.latest[i] = if self.ran[i] && t[i * 2 + 1] >= t[i * 2] {
                    (t[i * 2 + 1] - t[i * 2]) as f32 * self.period_ns / 1e6
                } else {
                    0.0
                };
            }
        }
        self.readback.unmap();
        self.in_flight = false;
    }
}
