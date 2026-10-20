//! How much the GPU occlusion culling removes: every 64th culled frame the per-run draw
//! counts are copied out and mapped without waiting, and every few seconds the latest
//! split is logged at debug level (opaque draws drawn early, drawn late, and in the
//! list): `RUST_LOG=haste_render::renderer::gpu_cull=debug`.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

const IDLE: u8 = 0;
const COPIED: u8 = 1;
const MAPPING: u8 = 2;
const MAPPED: u8 = 3;

pub(super) struct CullStats {
    buffer: Option<wgpu::Buffer>,
    state: Arc<AtomicU8>,
    frame: u32,
    /// Runs and opaque draws of the frame whose counts were copied.
    runs: u32,
    opaque: u32,
    last_log: Option<std::time::Instant>,
}

impl CullStats {
    pub(super) fn new() -> CullStats {
        CullStats {
            buffer: None,
            state: Arc::new(AtomicU8::new(IDLE)),
            frame: 0,
            runs: 0,
            opaque: 0,
            last_log: None,
        }
    }

    /// Copies this frame's counts out when a sample is due (after the late test).
    pub(super) fn record(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        counts: &wgpu::Buffer,
        runs: u32,
        opaque: u32,
    ) {
        self.frame = self.frame.wrapping_add(1);
        if !self.frame.is_multiple_of(64) || self.state.load(Ordering::Acquire) != IDLE {
            return;
        }
        let size = runs as u64 * 8;
        if self.buffer.as_ref().is_none_or(|b| b.size() < size) {
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gpu-cull-stats"),
                size: size.next_power_of_two(),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        encoder.copy_buffer_to_buffer(counts, 0, self.buffer.as_ref().unwrap(), 0, size);
        self.runs = runs;
        self.opaque = opaque;
        self.state.store(COPIED, Ordering::Release);
    }

    /// Posts the map of a copy submitted this frame.
    pub(super) fn after_submit(&mut self) {
        if self.state.load(Ordering::Acquire) != COPIED {
            return;
        }
        self.state.store(MAPPING, Ordering::Release);
        let state = self.state.clone();
        self.buffer
            .as_ref()
            .unwrap()
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| {
                state.store(if r.is_ok() { MAPPED } else { IDLE }, Ordering::Release)
            });
    }

    /// Reads a finished map (the renderer's timer polls the device every frame).
    pub(super) fn poll(&mut self) {
        if self.state.load(Ordering::Acquire) != MAPPED {
            return;
        }
        let buffer = self.buffer.as_ref().unwrap();
        let (early, late) = {
            let Ok(data) = buffer.slice(..).get_mapped_range() else {
                self.state.store(IDLE, Ordering::Release);
                return;
            };
            let counts: &[u32] = bytemuck::cast_slice(&data);
            let runs = self.runs as usize;
            let early: u32 = counts[..runs].iter().sum();
            let late: u32 = counts[runs..runs * 2].iter().sum();
            (early, late)
        };
        buffer.unmap();
        self.state.store(IDLE, Ordering::Release);
        let now = std::time::Instant::now();
        if self
            .last_log
            .is_none_or(|t| now - t >= std::time::Duration::from_secs(5))
        {
            self.last_log = Some(now);
            tracing::info!(
                "GPU occlusion culling: {} of {} opaque draws drawn ({} early, {} late)",
                early + late,
                self.opaque,
                early,
                late
            );
        }
    }
}
