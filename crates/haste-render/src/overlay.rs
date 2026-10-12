// World overlays: the selection outline of the targeted block (its outline shape boxes as black
// lines at 40% alpha, slightly inflated) and break cracks (`textures/block/destroy_stage_0..9`
// projected onto the block's model faces, multiplied 2 * src * dst into the terrain).
//
// The renderer holds no world: the caller samples the target's outline boxes and the states of
// breaking blocks with `Renderer::sample_world`.

use std::sync::Arc;

use glam::{DVec3, Vec3};
use haste_assets::{Assets, BakedBlockModels};
use haste_core::math::Aabb;
use haste_core::{BlockPos, BlockStateId, Identifier};

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct OverlayVertex {
    pub pos: [f32; 3],
    pub data: [f32; 4],
}

impl OverlayVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<OverlayVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &Self::ATTRIBUTES,
    };
}

/// What `sample_world` found for this frame's overlays.
#[derive(Default, Clone, Debug)]
pub struct OverlayWorld {
    /// The targeted block and its outline boxes (block-local).
    pub target: Option<(BlockPos, Vec<Aabb>)>,
    /// Breaking blocks with their states.
    pub breaking: Vec<(BlockPos, BlockStateId, u8)>,
}

pub struct OverlayRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    lines: wgpu::RenderPipeline,
    cracks: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    buffer: wgpu::Buffer,
    capacity: usize,
    vertices: Vec<OverlayVertex>,
    line_count: u32,
    crack_count: u32,
    pub world: OverlayWorld,
}

impl OverlayRenderer {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        target_format: wgpu::TextureFormat,
        camera_layout: &wgpu::BindGroupLayout,
        assets: &mut Assets,
    ) -> OverlayRenderer {
        // The ten crack stages side by side in a 160x16 strip.
        let mut strip = image::RgbaImage::new(160, 16);
        for stage in 0..10 {
            match assets.texture(&Identifier::minecraft(&format!(
                "block/destroy_stage_{stage}"
            ))) {
                Ok(img) => {
                    let img =
                        image::imageops::resize(&img, 16, 16, image::imageops::FilterType::Nearest);
                    image::imageops::replace(&mut strip, &img, stage * 16, 0);
                }
                Err(e) => tracing::warn!("crack texture {stage}: {e:#}"),
            }
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cracks"),
            size: wgpu::Extent3d {
                width: 160,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Raw values: mid grey (128) leaves the terrain unchanged under 2 * src * dst.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            strip.as_raw(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(160 * 4),
                rows_per_image: Some(16),
            },
            wgpu::Extent3d {
                width: 160,
                height: 16,
                depth_or_array_layers: 1,
            },
        );
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("overlay-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("overlay-bind"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(
                    &texture.create_view(&Default::default()),
                ),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("overlay"),
            source: wgpu::ShaderSource::Wgsl(include_str!("overlay.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("overlay-pipeline-layout"),
            bind_group_layouts: &[Some(camera_layout), Some(&layout)],
            immediate_size: 0,
        });
        let make = |label: &str,
                    fs: &str,
                    topology: wgpu::PrimitiveTopology,
                    blend: wgpu::BlendState,
                    bias: wgpu::DepthBiasState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(OverlayVertex::LAYOUT)],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::COLOR,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth24Plus,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias,
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let lines = make(
            "outline",
            "fs_line",
            wgpu::PrimitiveTopology::LineList,
            wgpu::BlendState::ALPHA_BLENDING,
            // wgpu has no depth bias for lines: the vertices are pulled toward the camera
            // instead (`prepare`).
            Default::default(),
        );
        let cracks = make(
            "cracks",
            "fs_crack",
            wgpu::PrimitiveTopology::TriangleList,
            wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Dst,
                    dst_factor: wgpu::BlendFactor::Src,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent::OVER,
            },
            // Pulled toward the camera so the decal wins over the face it lies on.
            wgpu::DepthBiasState {
                constant: -2,
                slope_scale: -1.0,
                clamp: 0.0,
            },
        );
        let capacity = 1024;
        let buffer = make_buffer(&device, capacity);
        OverlayRenderer {
            device,
            queue,
            lines,
            cracks,
            bind_group,
            buffer,
            capacity,
            vertices: Vec::new(),
            line_count: 0,
            crack_count: 0,
            world: OverlayWorld::default(),
        }
    }

    /// Builds the frame's overlay geometry (camera-relative). Only blocks that match the
    /// frame's `block_target` / `breaking` lists and were sampled are drawn.
    pub fn prepare(
        &mut self,
        camera: DVec3,
        block_target: Option<BlockPos>,
        breaking: &[(BlockPos, u8)],
        baked: &BakedBlockModels,
    ) {
        self.vertices.clear();
        if let (Some(pos), Some((sampled, boxes))) = (block_target, &self.world.target) {
            if pos == *sampled {
                let origin = DVec3::new(pos.x as f64, pos.y as f64, pos.z as f64) - camera;
                for b in boxes {
                    push_box_lines(&mut self.vertices, origin, b, [0.0, 0.0, 0.0, 0.4]);
                }
            }
        }
        // Lines lie on the neighbours' faces too: move them 0.3% toward the camera (positions are
        // camera-relative) so they win the depth test without z-fighting.
        for v in &mut self.vertices {
            v.pos = v.pos.map(|c| c * 0.997);
        }
        self.line_count = self.vertices.len() as u32;
        for &(pos, stage) in breaking {
            let Some(&(_, state, _)) = self.world.breaking.iter().find(|b| b.0 == pos) else {
                continue;
            };
            let origin = DVec3::new(pos.x as f64, pos.y as f64, pos.z as f64) - camera;
            push_cracks(
                &mut self.vertices,
                origin.as_vec3(),
                baked.quads(state),
                stage.min(9),
            );
        }
        self.crack_count = self.vertices.len() as u32 - self.line_count;
        if self.vertices.len() > self.capacity {
            self.capacity = self.vertices.len().next_power_of_two();
            self.buffer = make_buffer(&self.device, self.capacity);
        }
        if !self.vertices.is_empty() {
            self.queue
                .write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.vertices));
        }
    }

    /// Draws into the main pass after terrain (camera bind group at group 0).
    pub fn draw<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) -> u32 {
        if self.vertices.is_empty() {
            return 0;
        }
        let mut calls = 0;
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        if self.crack_count > 0 {
            pass.set_pipeline(&self.cracks);
            pass.draw(self.line_count..self.line_count + self.crack_count, 0..1);
            calls += 1;
        }
        if self.line_count > 0 {
            pass.set_pipeline(&self.lines);
            pass.draw(0..self.line_count, 0..1);
            calls += 1;
        }
        calls
    }
}

fn make_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("overlay-vertices"),
        size: (capacity * std::mem::size_of::<OverlayVertex>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The 12 edges of a box (block-local, inflated by 0.002 like vanilla's outline).
fn push_box_lines(out: &mut Vec<OverlayVertex>, origin: DVec3, b: &Aabb, color: [f32; 4]) {
    let e = 0.002;
    let lo = (origin + b.min - DVec3::splat(e)).as_vec3();
    let hi = (origin + b.max + DVec3::splat(e)).as_vec3();
    let c = |x: bool, y: bool, z: bool| {
        Vec3::new(
            if x { hi.x } else { lo.x },
            if y { hi.y } else { lo.y },
            if z { hi.z } else { lo.z },
        )
    };
    let edges = [
        (c(false, false, false), c(true, false, false)),
        (c(false, false, true), c(true, false, true)),
        (c(false, true, false), c(true, true, false)),
        (c(false, true, true), c(true, true, true)),
        (c(false, false, false), c(false, true, false)),
        (c(true, false, false), c(true, true, false)),
        (c(false, false, true), c(false, true, true)),
        (c(true, false, true), c(true, true, true)),
        (c(false, false, false), c(false, false, true)),
        (c(true, false, false), c(true, false, true)),
        (c(false, true, false), c(false, true, true)),
        (c(true, true, false), c(true, true, true)),
    ];
    for (a, b) in edges {
        out.push(OverlayVertex {
            pos: a.to_array(),
            data: color,
        });
        out.push(OverlayVertex {
            pos: b.to_array(),
            data: color,
        });
    }
}

/// Crack decals over a block's model quads: the stage texture projected by block-local
/// position on the face's plane (16 texels per block).
fn push_cracks(
    out: &mut Vec<OverlayVertex>,
    origin: Vec3,
    quads: &[haste_assets::BakedQuad],
    stage: u8,
) {
    use haste_assets::Direction as D;
    let column = stage as f32 * 16.0;
    for q in quads {
        let uv = |p: [f32; 3]| -> [f32; 2] {
            let (u, v) = match q.direction {
                D::Up => (p[0], p[2]),
                D::Down => (p[0], 1.0 - p[2]),
                D::North => (1.0 - p[0], 1.0 - p[1]),
                D::South => (p[0], 1.0 - p[1]),
                D::West => (p[2], 1.0 - p[1]),
                D::East => (1.0 - p[2], 1.0 - p[1]),
            };
            [u * 16.0, v * 16.0]
        };
        for i in [0usize, 1, 2, 0, 2, 3] {
            let p = q.positions[i];
            let t = uv(p);
            out.push(OverlayVertex {
                pos: (origin + Vec3::from(p)).to_array(),
                data: [t[0], t[1], column, 0.0],
            });
        }
    }
}
