//! Drawing the pool: camera-facing quads over the particle atlas in one batched draw (vanilla
//! `QuadParticleGroup` and `QuadParticleFeatureRenderer`).

use std::sync::Arc;

use glam::Vec3;

use crate::entity::EntityVertex;
use crate::renderer::Camera;

use super::resources::ParticleTypes;
use super::*;

/// Draws the pool: camera-facing quads over the particle atlas, one draw call for all
/// particles. The vertex/pipeline shape is the entity renderer's (same shader, textureLoad
/// sprite rects, vertex light colour, fog); particles differ in culling (billboards face
/// the camera) and depth (blended, no depth write).
pub struct ParticleRenderer {
    pub(super) device: Arc<wgpu::Device>,
    pub(super) queue: Arc<wgpu::Queue>,
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) layout: wgpu::BindGroupLayout,
    pub(super) bind_group: Option<wgpu::BindGroup>,
    pub(super) vertex_buffer: wgpu::Buffer,
    pub(super) capacity: usize,
    pub(super) vertices: Vec<EntityVertex>,
    /// Frames since the last debug log of the drawn count.
    pub(super) log_clock: u32,
    pub drawn: u32,
}

impl ParticleRenderer {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        target_format: wgpu::TextureFormat,
        camera_layout: &wgpu::BindGroupLayout,
        lightmap_layout: &wgpu::BindGroupLayout,
    ) -> ParticleRenderer {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle-layout"),
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
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particle"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../entity.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particle-pipeline-layout"),
            bind_group_layouts: &[Some(camera_layout), Some(&layout), Some(lightmap_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particle"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(EntityVertex::LAYOUT)],
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
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // Billboards are seen from whichever side faces the camera.
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let capacity = 4096;
        let vertex_buffer = make_vertex_buffer(&device, capacity);
        ParticleRenderer {
            device,
            queue,
            pipeline,
            layout,
            bind_group: None,
            vertex_buffer,
            capacity,
            vertices: Vec::new(),
            log_clock: 0,
            drawn: 0,
        }
    }

    /// Uploads the atlas of `types` as the particle texture (one texture for all types).
    pub fn set_types(&mut self, types: &Arc<ParticleTypes>) {
        let (pixels, size) = types.atlas();
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("particle-atlas"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 4),
                rows_per_image: Some(size),
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );
        self.bind_group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle-bind"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(
                    &texture.create_view(&Default::default()),
                ),
            }],
        }));
    }

    /// Poses every particle as a camera-facing quad into the vertex buffer (positions
    /// interpolated within the tick; light resampled when a particle leaves its block).
    /// The camera bind group of the main pass is at group 0; roll is ignored (billboards
    /// do not tilt with the hurt tilt).
    pub fn prepare(
        &mut self,
        particles: &mut Particles,
        camera: Camera,
        partial: f32,
        world: &dyn BlockView,
    ) {
        self.vertices.clear();
        self.drawn = 0;
        // Debug visibility: how many particles the frame poses (once a second).
        self.log_clock += 1;
        let log = self.log_clock.is_multiple_of(60);
        let types = particles.types.clone();
        let Some(types) = types.as_deref() else {
            return;
        };
        let (right, up) = billboard_axes(camera.yaw, camera.pitch);
        for p in &mut particles.pool {
            let pos = p.prev_pos.lerp(p.pos, partial);
            // Light at the particle's position, resampled when it crosses a block boundary.
            let block = BlockPos::new(
                pos.x.floor() as i32,
                pos.y.floor() as i32,
                pos.z.floor() as i32,
            );
            if p.sky_light == UNSAMPLED || block != p.light_at {
                p.light_at = block;
                p.sky_light = world.sky_light(block);
                p.block_light = world.block_light(block);
            }
            let (rect, uvs) = match p.block_sprite {
                // A terrain particle: its block sprite's fixed 4x4 sub-piece.
                Some(s) => (
                    s.rect,
                    [
                        [s.uv[0], s.uv[3]],
                        [s.uv[2], s.uv[3]],
                        [s.uv[2], s.uv[1]],
                        [s.uv[0], s.uv[1]],
                    ],
                ),
                None => {
                    let sprites = types.sprites(p.kind);
                    if sprites.is_empty() {
                        continue;
                    }
                    // The sprite for the particle's age (multi-sprite types animate over
                    // the list, like vanilla's `setSpriteFromAge`).
                    let index = ((p.age / p.lifetime) * sprites.len() as f32).floor() as usize;
                    let rect = sprites[index.min(sprites.len() - 1)];
                    let (x, y, w, h) = (
                        rect[0] as f32,
                        rect[1] as f32,
                        rect[2] as f32,
                        rect[3] as f32,
                    );
                    (rect, [[x, y + h], [x + w, y + h], [x + w, y], [x, y]])
                }
            };
            let light = if p.behaviour.full_light {
                // HugeExplosionParticle.getLightCoords: always full light.
                (15u8, 15u8)
            } else {
                (p.block_light, p.sky_light)
            };
            let alpha = p.behaviour.alpha.unwrap_or(1.0);
            // The light is applied by the shader from the light texture, not multiplied into
            // the colour here (that would light twice).
            let color = [p.color[0], p.color[1], p.color[2], alpha];
            let rel = (pos.as_dvec3() - camera.pos).as_vec3();
            // The quad size the behaviour rolled at spawn, grown for its age
            // (`SingleQuadParticle.getQuadSize`).
            let growth = p.behaviour.growth.scale((p.age + partial) / p.lifetime);
            let half = p.quad * growth;
            let r = right * half;
            let u = up * half;
            let corners = [rel - r - u, rel + r - u, rel + r + u, rel - r + u];
            let rect16 = rect;
            for i in [0usize, 1, 2, 0, 2, 3] {
                self.vertices.push(EntityVertex {
                    pos: corners[i].to_array(),
                    uv: uvs[i],
                    color,
                    rect: rect16,
                    light: [light.0 as u16, light.1 as u16, 0, 0],
                });
            }
            self.drawn += 1;
        }
        if log && (self.drawn > 0 || particles.alive() > 0) {
            tracing::debug!(
                "particles: {} drawn of {} alive, {} vertices",
                self.drawn,
                particles.alive(),
                self.vertices.len()
            );
        }
        if self.vertices.len() > self.capacity {
            self.capacity = self.vertices.len().next_power_of_two();
            self.vertex_buffer = make_vertex_buffer(&self.device, self.capacity);
        }
        if !self.vertices.is_empty() {
            self.queue
                .write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.vertices));
        }
    }

    /// Draws into the main pass after the entities (camera bind group must be set at
    /// group 0). One draw call for every particle.
    pub fn draw<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) -> u32 {
        let Some(bind) = &self.bind_group else {
            return 0;
        };
        if self.vertices.is_empty() {
            return 0;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(1, bind, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..self.vertices.len() as u32, 0..1);
        1
    }
}

pub(super) fn make_vertex_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("particle-vertices"),
        size: (capacity * std::mem::size_of::<EntityVertex>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Billboard axes for a camera look (yaw/pitch in degrees, Minecraft's convention):
/// screen-right horizontal, up perpendicular to the look. Never degenerate (right and look
/// are orthogonal unit vectors).
pub(super) fn billboard_axes(yaw: f32, pitch: f32) -> (Vec3, Vec3) {
    let yaw = yaw.to_radians();
    let pitch = pitch.to_radians();
    let look = Vec3::new(
        -pitch.cos() * yaw.sin(),
        -pitch.sin(),
        pitch.cos() * yaw.cos(),
    );
    let right = Vec3::new(-yaw.cos(), 0.0, -yaw.sin());
    let up = right.cross(look).normalize();
    (right, up)
}
