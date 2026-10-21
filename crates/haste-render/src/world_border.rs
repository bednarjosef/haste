// The world border's forcefield wall (vanilla `WorldBorderRenderer`): four quads around the
// camera, one per border side, drawn after the weather (`LevelRenderer` order) with alpha
// blending. The vertices are rebuilt only when the border's square changes (vanilla's
// `shouldRebuildWorldBorderBuffer`); each frame writes just the small uniform (the camera
// offset, the tint and alpha, the scroll). The per-side draws are ordered nearest first and
// skipped beyond the render distance, like `WorldBorderRenderer.draw`.

use std::time::Instant;

use glam::Mat4;
use haste_core::Identifier;

use crate::Camera;

mod geometry;
#[cfg(test)]
mod tests;

use geometry::{alpha, build_wall, closest_sides, scroll_offset, Bounds};

/// Vanilla `WorldBorderRenderer.prepare`'s `depthFar`: the wall's half height (blocks). The
/// camera's far plane (`renderer/camera.rs`) bounds what can be seen anyway.
const DEPTH_FAR: f32 = 4096.0;

/// The border as the frame sees it: the square, the status tint and the render distance
/// (vanilla `renderDistance`, the view distance in blocks).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldBorderFrame {
    pub min_x: f64,
    pub max_x: f64,
    pub min_z: f64,
    pub max_z: f64,
    /// Vanilla `BorderStatus.getColor`: the wall's RGB (0xRRGGBB).
    pub tint: u32,
    /// The render distance in blocks.
    pub render_distance: f64,
}

impl WorldBorderFrame {
    fn bounds(&self) -> Bounds {
        Bounds {
            min_x: self.min_x,
            max_x: self.max_x,
            min_z: self.min_z,
            max_z: self.max_z,
        }
    }
}

/// Uniforms for the wall pass. The padding rounds the struct to the WGSL uniform layout's
/// size (the `vec3` members force 16-byte alignment).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BorderUniforms {
    view_proj: [[f32; 4]; 4],
    /// The wall's origin relative to the camera (vanilla's ModelOffset).
    offset: [f32; 3],
    /// 1 when the target view is sRGB and the hardware encodes (GL fallback).
    hw_srgb: f32,
    /// The tint (sRGB) and the distance alpha.
    color: [f32; 4],
    /// The scroll: the texture matrix's translation along u and v.
    tex_offset: f32,
    /// Fills the WGSL struct out to 128 bytes: its trailing `vec3` pads to 16-byte alignment.
    _pad: [f32; 7],
}

/// The forcefield wall pass (vanilla `WorldBorderRenderer` with `RenderPipelines.WORLD_BORDER`).
pub struct WorldBorderRenderer {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    /// The border the frame draws, set by the frame before `prepare`.
    border: Option<WorldBorderFrame>,
    /// The square the wall's vertices were built for (vanilla `lastBorderMinX` …).
    built_for: Option<Bounds>,
    /// The wall's origin, for the camera offset (vanilla `lastMinX`/`lastMinZ`).
    origin: (f64, f64),
    /// Whether this frame draws the wall (vanilla's `!(state.alpha <= 0.0)`).
    active: bool,
    /// The sides nearest first, with their quad indices, for this frame.
    sides: [(f64, usize); 4],
    /// The render distance in blocks for the per-side cut-off.
    render_distance: f64,
    start: Instant,
    hw_srgb: bool,
}

impl WorldBorderRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        assets: &mut haste_assets::Assets,
    ) -> WorldBorderRenderer {
        let texture = forcefield_view(device, queue, assets);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("world-border-sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("world-border-uniforms"),
            size: std::mem::size_of::<BorderUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // The wall's vertices: 24 per rebuild (four quads of two triangles each).
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("world-border-vertices"),
            size: (24 * std::mem::size_of::<[f32; 5]>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("world-border-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("world-border-bind"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&texture),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("world-border"),
            source: wgpu::ShaderSource::Wgsl(include_str!("world_border.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("world-border-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        // Position (three floats) and texture coordinates (two floats) per vertex.
        let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2];
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<[f32; 5]>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("world-border"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout)],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            // The wall is seen from both sides (vanilla culling off).
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        WorldBorderRenderer {
            pipeline,
            uniforms,
            bind_group,
            vertices,
            border: None,
            built_for: None,
            origin: (0.0, 0.0),
            active: false,
            sides: [(0.0, 0); 4],
            render_distance: 0.0,
            start: Instant::now(),
            hw_srgb: target_format.is_srgb(),
        }
    }

    /// The border the frames to come draw (vanilla's `WorldBorderRenderState` filled from
    /// `WorldBorder`); `None` draws nothing.
    pub fn set_border(&mut self, border: Option<WorldBorderFrame>) {
        self.border = border;
    }

    /// Vanilla `WorldBorderRenderer.extract` + `prepare` for this frame: the alpha from the
    /// camera's distance, the wall rebuilt when the square changed, the per-side order and
    /// the uniforms. Draws nothing when the alpha is zero.
    pub fn prepare(&mut self, queue: &wgpu::Queue, camera: &Camera, view_proj: Mat4) {
        self.active = false;
        let Some(border) = self.border else {
            return;
        };
        let bounds = border.bounds();
        let (x, z) = (camera.pos.x, camera.pos.z);
        let fade = alpha(&bounds, x, z, border.render_distance);
        if fade <= 0.0 {
            return;
        }
        if self.built_for != Some(bounds) {
            // Vanilla's `v0` is `-frac(cameraY / 2)`, taken when the wall is built.
            let y = camera.pos.y;
            let half = y * 0.5;
            let v0 = (-(half - half.floor())) as f32;
            let wall = build_wall(&bounds, x, z, border.render_distance, DEPTH_FAR, v0);
            queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&wall.vertices));
            self.origin = (wall.origin_x, wall.origin_z);
            self.built_for = Some(bounds);
        }
        self.sides = closest_sides(&bounds, x, z);
        self.render_distance = border.render_distance;
        let uniforms = BorderUniforms {
            view_proj: view_proj.to_cols_array_2d(),
            offset: [
                (self.origin.0 - x) as f32,
                (-camera.pos.y) as f32,
                (self.origin.1 - z) as f32,
            ],
            hw_srgb: if self.hw_srgb { 1.0 } else { 0.0 },
            color: tint_color(border.tint, fade),
            tex_offset: scroll_offset(self.start.elapsed().as_millis()),
            _pad: [0.0; 7],
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        self.active = true;
    }

    /// Draws this frame's wall (vanilla `WorldBorderRenderer.draw`): each side nearer than
    /// the render distance, nearest first. Returns the draw count.
    pub fn draw<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) -> u32 {
        if !self.active {
            return 0;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        let mut draws = 0;
        for (distance, side) in self.sides {
            if distance < self.render_distance {
                let first = (side * 6) as u32;
                pass.draw(first..first + 6, 0..1);
                draws += 1;
            }
        }
        draws
    }
}

/// The tint as vanilla's ColorModulator: the RGB (sRGB, 0..=1) and the distance alpha.
fn tint_color(tint: u32, alpha: f32) -> [f32; 4] {
    let channel = |shift: u32| ((tint >> shift) & 0xff) as f32 / 255.0;
    [channel(16), channel(8), channel(0), alpha]
}

/// The forcefield texture (vanilla `textures/misc/forcefield.png`), or a white texel when
/// the assets lack it (the wall then shows the tint alone).
fn forcefield_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    assets: &mut haste_assets::Assets,
) -> wgpu::TextureView {
    let (width, height, pixels) = match assets.texture(&Identifier::minecraft("misc/forcefield")) {
        Ok(img) => {
            let (w, h) = img.dimensions();
            (w, h, img.into_raw())
        }
        Err(e) => {
            tracing::warn!("world border texture: {e:#}");
            (1, 1, vec![255, 255, 255, 255])
        }
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("world-border-forcefield"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * width),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    texture.create_view(&Default::default())
}
