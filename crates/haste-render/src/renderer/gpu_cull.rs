//! GPU occlusion culling of the terrain draws (Haste's own; vanilla culls sections only
//! on the CPU, with `SectionOcclusionGraph` and the frustum). Two phases per frame, so
//! nothing pops in:
//!
//! 1. early: the opaque draws of sections that were visible last frame are drawn
//!    (`select_early` compacts them into per-run indirect arguments with a draw count);
//! 2. a depth pyramid is built from that depth (`pyramid.rs`), and every draw's section
//!    box is tested against it (`test_late`): opaque draws visible now but not drawn
//!    early are drawn late, translucent draws keep their order and get zero instances
//!    when hidden, and the visible set becomes next frame's early set.
//!
//! A section hidden behind terrain therefore costs one box test instead of its vertex
//! work. The CPU cull (frustum, occlusion walk, facing groups) still decides what enters
//! the draw list; this only removes draws from it.

mod pyramid;
mod stats;

use pyramid::{Pyramid, PyramidPipelines};
use stats::CullStats;

use super::SectionDraw;

/// One draw's input to the cull shader (`Draw` in cull.wgsl).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CullDraw {
    lo: [f32; 3],
    slot: u32,
    hi: [f32; 3],
    run: u32,
    run_start: u32,
    translucent: u32,
    _pad: [u32; 2],
}

/// `Params` in cull.wgsl.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    view_proj: [[f32; 4]; 4],
    size: [f32; 2],
    levels: u32,
    draws: u32,
    runs: u32,
    _pad: [u32; 3],
}

/// Size of one `DrawIndexedIndirectArgs`.
const ARGS: u64 = std::mem::size_of::<wgpu::util::DrawIndexedIndirectArgs>() as u64;

/// The buffers that grow with the draw list and the section slab.
struct Buffers {
    draws: wgpu::Buffer,
    args_early: wgpu::Buffer,
    args_late: wgpu::Buffer,
    draw_capacity: usize,
    counts: wgpu::Buffer,
    run_capacity: usize,
    /// Per slab slot: visible last frame / this frame, swapped every frame.
    vis: [wgpu::Buffer; 2],
    slot_capacity: usize,
}

pub struct GpuCull {
    /// Off without indirect draw counts, in the quad format, or with `HASTE_GPU_CULL=0`.
    enabled: bool,
    /// Only when enabled: the depth reduction reads the depth texture with `textureLoad`,
    /// which the GL backend cannot translate (the GL fallback has no indirect draw count,
    /// so it never culls).
    pyramid_pipelines: Option<PyramidPipelines>,
    pyramid: Option<Pyramid>,
    layout: wgpu::BindGroupLayout,
    select: wgpu::ComputePipeline,
    test: wgpu::ComputePipeline,
    params: wgpu::Buffer,
    buffers: Buffers,
    /// The bind groups for each `current` parity, with the argument buffer and pyramid
    /// view they were built for.
    groups: Option<([wgpu::BindGroup; 2], wgpu::Buffer, wgpu::TextureView)>,
    /// Which `vis` buffer this frame writes.
    current: usize,
    staging: Vec<CullDraw>,
    runs: u32,
    /// Opaque draws in this frame's list (for the stats).
    opaque: u32,
    stats: CullStats,
    /// Whether this frame's draws go through the culling.
    active: bool,
}

impl GpuCull {
    pub fn new(device: &wgpu::Device, draw_count: bool) -> GpuCull {
        let enabled = draw_count
            && !crate::quad_format::quad_format_enabled()
            && std::env::var("HASTE_GPU_CULL").map_or(true, |v| v != "0");
        tracing::info!("GPU occlusion culling: {enabled}");
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpu-cull"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu_cull/cull.wgsl").into()),
        });
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gpu-cull"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage(1, true),
                storage(2, true),
                storage(3, false),
                storage(4, false),
                storage(5, false),
                storage(6, true),
                storage(7, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpu-cull"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu-cull-params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        GpuCull {
            enabled,
            pyramid_pipelines: enabled.then(|| PyramidPipelines::new(device)),
            pyramid: None,
            select: pipeline("select_early"),
            test: pipeline("test_late"),
            layout,
            params,
            buffers: Buffers::new(device, 1024, 256, 4096),
            groups: None,
            current: 0,
            staging: Vec::new(),
            runs: 0,
            opaque: 0,
            stats: CullStats::new(),
            active: false,
        }
    }

    /// Whether this frame's terrain is drawn in the two culled phases.
    pub fn active(&self) -> bool {
        self.active
    }

    /// Takes this frame's draw list (after its indirect arguments were written):
    /// uploads each draw's section box, slot and run.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        draws: &[SectionDraw],
        slots: usize,
    ) {
        self.stats.poll();
        self.active = self.enabled && !draws.is_empty();
        if !self.active {
            return;
        }
        self.runs = cull_draws(draws, &mut self.staging);
        self.opaque = self.staging.iter().filter(|d| d.translucent == 0).count() as u32;
        if self
            .buffers
            .grow(device, draws.len(), self.runs as usize, slots)
        {
            self.groups = None;
        }
        queue.write_buffer(&self.buffers.draws, 0, bytemuck::cast_slice(&self.staging));
    }

    /// Records the early phase's selection (before the main pass) and prepares the
    /// pyramid for `depth`.
    #[allow(clippy::too_many_arguments)]
    pub fn encode_early(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view_proj: glam::Mat4,
        size: (u32, u32),
        depth: &wgpu::TextureView,
        args_in: &wgpu::Buffer,
    ) {
        let Some(pipelines) = &self.pyramid_pipelines else {
            return;
        };
        let pyramid = pipelines.pyramid(device, self.pyramid.take(), depth, size);
        let stale = match &self.groups {
            Some((_, args, view)) => args != args_in || view != &pyramid.all_levels,
            None => true,
        };
        if stale {
            let group = |cur: usize| {
                self.buffers
                    .bind_group(device, &self.layout, &self.params, args_in, &pyramid, cur)
            };
            self.groups = Some((
                [group(0), group(1)],
                args_in.clone(),
                pyramid.all_levels.clone(),
            ));
        }
        let draws = self.staging.len() as u32;
        let params = Params {
            view_proj: view_proj.to_cols_array_2d(),
            size: [size.0 as f32, size.1 as f32],
            levels: pyramid.levels(),
            draws,
            runs: self.runs,
            _pad: [0; 3],
        };
        self.pyramid = Some(pyramid);
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        self.current ^= 1;
        encoder.clear_buffer(&self.buffers.counts, 0, Some(self.runs as u64 * 8));
        encoder.clear_buffer(&self.buffers.vis[self.current], 0, None);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("gpu-cull-early"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.select);
        pass.set_bind_group(0, &self.groups.as_ref().unwrap().0[self.current], &[]);
        pass.dispatch_workgroups(draws.div_ceil(64), 1, 1);
    }

    /// Records the pyramid build and the late test (between the early and late draws).
    pub fn encode_late(&self, encoder: &mut wgpu::CommandEncoder) {
        let (Some(pipelines), Some(pyramid), Some((groups, _, _))) =
            (&self.pyramid_pipelines, &self.pyramid, &self.groups)
        else {
            return;
        };
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("gpu-cull-late"),
            timestamp_writes: None,
        });
        pipelines.build(&mut pass, pyramid);
        pass.set_pipeline(&self.test);
        pass.set_bind_group(0, &groups[self.current], &[]);
        pass.dispatch_workgroups((self.staging.len() as u32).div_ceil(64), 1, 1);
    }

    /// Samples this frame's draw counts for the log (after the main pass).
    pub fn record_stats(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) {
        if self.active {
            self.stats.record(
                device,
                encoder,
                &self.buffers.counts,
                self.runs,
                self.opaque,
            );
        }
    }

    /// Call after the frame's submit (posts the stats readback's map).
    pub fn after_submit(&mut self) {
        self.stats.after_submit();
    }

    /// The early phase's arguments and the byte offset of run `run`'s draw count.
    pub fn early(&self, run: u32) -> (&wgpu::Buffer, &wgpu::Buffer, u64) {
        (
            &self.buffers.args_early,
            &self.buffers.counts,
            run as u64 * 4,
        )
    }

    /// The late phase's arguments and the byte offset of run `run`'s draw count.
    pub fn late(&self, run: u32) -> (&wgpu::Buffer, &wgpu::Buffer, u64) {
        (
            &self.buffers.args_late,
            &self.buffers.counts,
            (self.runs + run) as u64 * 4,
        )
    }
}

/// Fills `out` with each draw's cull input and returns the number of runs: a run is a
/// stretch of draws sharing a layer and an arena, which the draw loop issues as one
/// multi-draw (`terrain_draw.rs` splits the list the same way).
fn cull_draws(draws: &[SectionDraw], out: &mut Vec<CullDraw>) -> u32 {
    out.clear();
    let mut run = 0u32;
    let mut run_start = 0u32;
    for (i, d) in draws.iter().enumerate() {
        if i > 0 && (d.layer != draws[i - 1].layer || d.arena != draws[i - 1].arena) {
            run += 1;
            run_start = i as u32;
        }
        out.push(CullDraw {
            lo: d.lo,
            slot: d.slot,
            hi: d.hi,
            run,
            run_start,
            translucent: u32::from(d.layer == haste_assets::RenderLayer::Translucent),
            _pad: [0; 2],
        });
    }
    if draws.is_empty() {
        0
    } else {
        run + 1
    }
}

impl Buffers {
    fn new(device: &wgpu::Device, draws: usize, runs: usize, slots: usize) -> Buffers {
        let buffer = |label, size: u64, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let indirect = storage | wgpu::BufferUsages::INDIRECT;
        Buffers {
            draws: buffer(
                "gpu-cull-draws",
                (draws * std::mem::size_of::<CullDraw>()) as u64,
                storage,
            ),
            args_early: buffer("gpu-cull-args-early", draws as u64 * ARGS, indirect),
            args_late: buffer("gpu-cull-args-late", draws as u64 * ARGS, indirect),
            draw_capacity: draws,
            counts: buffer(
                "gpu-cull-counts",
                runs as u64 * 8,
                indirect | wgpu::BufferUsages::COPY_SRC,
            ),
            run_capacity: runs,
            vis: [
                buffer("gpu-cull-visible", slots as u64 * 4, storage),
                buffer("gpu-cull-visible", slots as u64 * 4, storage),
            ],
            slot_capacity: slots,
        }
    }

    /// Grows whatever is too small (to the next power of two); true when any buffer
    /// was replaced. New visibility buffers start all hidden: those sections are
    /// tested late for one frame.
    fn grow(&mut self, device: &wgpu::Device, draws: usize, runs: usize, slots: usize) -> bool {
        if draws <= self.draw_capacity && runs <= self.run_capacity && slots <= self.slot_capacity {
            return false;
        }
        *self = Buffers::new(
            device,
            draws.max(self.draw_capacity).next_power_of_two(),
            runs.max(self.run_capacity).next_power_of_two(),
            slots.max(self.slot_capacity).next_power_of_two(),
        );
        true
    }

    fn bind_group(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        params: &wgpu::Buffer,
        args_in: &wgpu::Buffer,
        pyramid: &Pyramid,
        current: usize,
    ) -> wgpu::BindGroup {
        fn buf(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry {
                binding,
                resource: buffer.as_entire_binding(),
            }
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpu-cull"),
            layout,
            entries: &[
                buf(0, params),
                buf(1, &self.draws),
                buf(2, args_in),
                buf(3, &self.args_early),
                buf(4, &self.args_late),
                buf(5, &self.counts),
                buf(6, &self.vis[current ^ 1]),
                buf(7, &self.vis[current]),
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(&pyramid.all_levels),
                },
            ],
        })
    }
}

#[cfg(test)]
mod tests;
