//! Rain and snow columns (vanilla `WeatherEffectRenderer`): one quad per column of the
//! weather radius around the camera, rain columns scrolling down `rain.png`, snow columns
//! drifting through `snow.png`, both fading with distance from the camera. All geometry is
//! rebuilt per frame into one reused CPU buffer and drawn in two batched calls (rain, then
//! snow) of one pipeline — no allocation per frame after the buffers warm up.

use std::sync::Arc;

use crate::java_random::JavaRandom;

/// The weather radius option's default and fancy value (vanilla `Options.weatherRadius`,
/// `IntRange(3, 10)`; the fast preset sets 5). Columns are drawn in a square of this half
/// extent around the camera's block.
pub const WEATHER_RADIUS: i32 = 10;

/// One weather column extracted this frame (vanilla
/// `WeatherEffectRenderer.ColumnInstance`): the block column, the y range to cover, the
/// texture offsets its random seed fixed, and the light at its sample position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherColumn {
    /// The column's block x and z.
    pub x: i32,
    pub z: i32,
    /// Bottom and top y to cover (already heightmap- and camera-clipped).
    pub bottom_y: i32,
    pub top_y: i32,
    /// U offset in texture tiles: 0 for rain (the shader picks its quarter by seed), the
    /// snow column's horizontal drift.
    pub u_offset: f32,
    /// V offset in texture tiles: the rain scroll or the snow's vertical drift.
    pub v_offset: f32,
    /// `(block, sky)` light levels at the column's sample position (vanilla
    /// `LightCoordsUtil.getLightCoords`).
    pub light: (u8, u8),
    /// The vanilla per-column seed `x*x*3121 + x*45238971 ^ z*z*418711 + z*13761`, for the
    /// shader's texture quarter choice.
    pub seed: i32,
}

/// The extracted weather state for one frame (vanilla `WeatherRenderState`): the rain
/// intensity (`ClientLevel.getRainLevel(partial)`, already lerped by the client), the radius,
/// and the rain and snow columns.
#[derive(Clone, Debug, Default)]
pub struct WeatherFrame {
    /// `WeatherRenderState.intensity`: the lerped rain level, 0..1.
    pub intensity: f32,
    /// `WeatherRenderState.radius`: the weather radius option.
    pub radius: i32,
    /// Rain columns (precipitation RAIN biomes), in scan order.
    pub rain: Vec<WeatherColumn>,
    /// Snow columns (precipitation SNOW biomes), in scan order.
    pub snow: Vec<WeatherColumn>,
}

impl WeatherFrame {
    pub fn is_empty(&self) -> bool {
        self.rain.is_empty() && self.snow.is_empty()
    }
}

/// The vanilla per-column seed (`WeatherEffectRenderer`'s render loop and
/// `createRainColumnInstance`): `x*x*3121 + x*45238971 ^ z*z*418711 + z*13761`, wrapping.
pub fn column_seed(x: i32, z: i32) -> i32 {
    x.wrapping_mul(x)
        .wrapping_mul(3121)
        .wrapping_add(x.wrapping_mul(45238971))
        ^ z.wrapping_mul(z)
            .wrapping_mul(418711)
            .wrapping_add(z.wrapping_mul(13761))
}

/// Vanilla's column orientation (`WeatherEffectRenderer`'s constructor): the unit vector
/// perpendicular to the direction from the camera block to the column, for the offsets
/// x, z in -16..=15 (the table's index `(z + 16) * 32 + (x + 16)`).
pub fn column_size(x: i32, z: i32) -> (f32, f32) {
    let (dx, dz) = (x as f32, z as f32);
    let distance = (dx * dx + dz * dz).sqrt();
    if distance == 0.0 {
        (0.0, 0.0)
    } else {
        (-dz / distance, dx / distance)
    }
}

/// Vanilla's distance fade (`WeatherEffectRenderer.prepareInstances`): alpha
/// `lerp(min(distanceSq / radiusSq, 1), maxAlpha, 0.5) * intensity` with `maxAlpha` 1 for
/// rain and 0.8 for snow.
pub fn column_alpha(
    relative_x: f32,
    relative_z: f32,
    radius: i32,
    max_alpha: f32,
    intensity: f32,
) -> f32 {
    let radius_sq = (radius * radius) as f32;
    let distance_sq = relative_x * relative_x + relative_z * relative_z;
    let t = (distance_sq / radius_sq).min(1.0);
    (max_alpha + (0.5 - max_alpha) * t) * intensity
}

/// One lightning bolt to draw this frame (vanilla `LightningBoltRenderState`: the entity's
/// position and its geometry seed). Collected from the frame's entity instances.
#[derive(Clone, Copy, Debug)]
pub struct LightningBolt {
    /// Camera-relative base position (the entity's).
    pub pos: [f32; 3],
    /// The geometry seed (`LightningBolt.seed`; Haste derives it from the entity id —
    /// vanilla's is a client-side random, so the exact shape already differs per client).
    pub seed: u64,
}

/// One lightning vertex: camera-relative position and RGBA colour (vanilla
/// `rendertype_lightning`'s POSITION_COLOR; additive, fogged, no texture or lightmap).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LightningVertex {
    pos: [f32; 3],
    _pad: f32,
    color: [f32; 4],
}

/// One weather vertex: camera-relative position, texture coordinates in atlas texels, RGBA
/// colour (the distance fade times intensity in alpha), the wrap rectangle and the light
/// levels (sampled against the lightmap in the shader).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct WeatherVertex {
    pos: [f32; 3],
    uv: [f32; 2],
    color: [f32; 4],
    rect: [u16; 4],
    light: [u32; 2],
}

/// Uniforms for the weather pass: the world matrix, fog colour and bands (the same fields
/// the entity pass reads from the chunk camera).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct WeatherUniforms {
    view_proj: [[f32; 4]; 4],
    fog_color: [f32; 4],
    fog_env_start: f32,
    fog_env_end: f32,
    fog_rd_start: f32,
    fog_rd_end: f32,
    /// 1 when the target view is sRGB (GL fallback: hardware encodes; see weather.wgsl).
    hw_srgb: f32,
    _pad: [f32; 3],
}

/// The weather pass: draws the frame's rain and snow columns as camera-relative quads over
/// `environment/rain.png` and `environment/snow.png` in the sky atlas, two batched draws.
pub struct WeatherRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipeline: wgpu::RenderPipeline,
    /// The lightning pass (vanilla `RenderPipelines.LIGHTNING`: additive, fogged,
    /// POSITION_COLOR), from its own vertex buffer after the columns.
    lightning_pipeline: wgpu::RenderPipeline,
    /// Lightning's own bind group: the uniforms at 0 (view matrix, fog) and the atlas at 1
    /// is unused by the shader but the layout is shared with the weather pass.
    lightning_bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    /// Sky-atlas and lightmap views; set once at startup (`set_textures` builds the group).
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    capacity: usize,
    vertices: Vec<WeatherVertex>,
    /// The lightning quads of this frame (own buffer, own pipeline).
    lightning_buffer: wgpu::Buffer,
    lightning_capacity: usize,
    lightning_vertices: Vec<LightningVertex>,
    /// Rectangle of `environment/rain.png` in the sky atlas (x, y, w, h).
    rain_rect: [u32; 4],
    /// Rectangle of `environment/snow.png` in the sky atlas.
    snow_rect: [u32; 4],
    /// First snow vertex index (the rain columns precede them).
    snow_start: u32,
    vertex_count: u32,
    /// Lightning vertex count this frame.
    lightning_count: u32,
    /// Columns drawn this frame (for stats).
    pub drawn: u32,
    /// Whether the one-time diagnostics line was logged.
    logged: bool,
    /// Whether the target view is sRGB and the hardware encodes (GL fallback).
    hw_srgb: bool,
}

impl WeatherRenderer {
    /// Builds the pass over the sky atlas's rain and snow rectangles. The atlas view and
    /// lightmap must be supplied via [`Self::set_textures`] before the first frame; until
    /// then a white placeholder keeps the group valid.
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        target_format: wgpu::TextureFormat,
        rain_rect: [u32; 4],
        snow_rect: [u32; 4],
    ) -> WeatherRenderer {
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("weather-uniforms"),
            size: std::mem::size_of::<WeatherUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let white = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("weather-white"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
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
            white.as_image_copy(),
            &[255, 255, 255, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let white_view = white.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("weather-sampler"),
            ..Default::default()
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("weather-layout"),
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
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("weather-bind"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&white_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&white_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("weather"),
            source: wgpu::ShaderSource::Wgsl(include_str!("weather.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("weather-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let attributes: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x2, 2 => Float32x4, 3 => Uint16x4, 4 => Uint32x2
        ];
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<WeatherVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("weather"),
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
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        // The lightning pass (vanilla `RenderPipelines.LIGHTNING`): additive blend
        // (SRC_ALPHA, ONE), depth-tested without write, POSITION_COLOR with fog. Shares the
        // weather bind group layout (the shader reads only the uniforms).
        let lightning_attributes: [wgpu::VertexAttribute; 2] =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
        let lightning_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LightningVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &lightning_attributes,
        };
        let lightning_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("lightning"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_lightning"),
                compilation_options: Default::default(),
                buffers: &[Some(lightning_layout)],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_lightning"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let lightning_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lightning-bind"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&white_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&white_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let capacity = 4096;
        WeatherRenderer {
            device: device.clone(),
            queue,
            pipeline,
            lightning_pipeline,
            lightning_bind_group,
            uniforms,
            bind_group,
            vertex_buffer: make_buffer(&device, capacity),
            capacity,
            vertices: Vec::with_capacity(capacity),
            lightning_buffer: make_lightning_buffer(&device, 4096),
            lightning_capacity: 4096,
            lightning_vertices: Vec::new(),
            rain_rect,
            snow_rect,
            snow_start: 0,
            vertex_count: 0,
            lightning_count: 0,
            drawn: 0,
            logged: false,
            hw_srgb: target_format.is_srgb(),
        }
    }

    /// Binds the sky atlas and lightmap views (once at startup, when the renderer's own
    /// are created).
    pub fn set_textures(
        &mut self,
        device: &wgpu::Device,
        atlas: &wgpu::TextureView,
        lightmap: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) {
        let layout = self.pipeline.get_bind_group_layout(0);
        self.bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("weather-bind"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(atlas),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(lightmap),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
    }

    /// Builds this frame's vertices and uploads them and the uniforms. `camera` is the
    /// frame's world matrix (camera-relative vertices), `fog` the frame's fog bands and
    /// colour (linear; the entity pass's group-0 camera matches).
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        weather: &WeatherFrame,
        camera: &crate::Camera,
        view_proj: glam::Mat4,
        fog: &crate::fog::FogData,
        fog_color: [f32; 3],
        game_time: i64,
        partial_tick: f32,
    ) {
        self.vertices.clear();
        self.drawn = 0;
        let cam = camera.pos.as_vec3();
        let cbx = cam.x.floor() as i32;
        let cbz = cam.z.floor() as i32;
        let push = |out: &mut Vec<WeatherVertex>,
                    columns: &[WeatherColumn],
                    rect: [u32; 4],
                    max_alpha: f32| {
            for c in columns {
                // `prepareInstances`: relative position, distance fade, column orientation.
                let relative_x = c.x as f32 + 0.5 - cam.x;
                let relative_z = c.z as f32 + 0.5 - cam.z;
                let alpha = column_alpha(
                    relative_x,
                    relative_z,
                    weather.radius,
                    max_alpha,
                    weather.intensity,
                );
                let (size_x, size_z) = column_size(c.x - cbx, c.z - cbz);
                let half_x = size_x / 2.0;
                let half_z = size_z / 2.0;
                let x0 = relative_x - half_x;
                let x1 = relative_x + half_x;
                let z0 = relative_z - half_z;
                let z1 = relative_z + half_z;
                let y0 = c.bottom_y as f32 - cam.y;
                let y1 = c.top_y as f32 - cam.y;
                // Rain: the full rain.png width (`uOffset` is 0, so u spans the whole
                // texture); the scroll sits in `v_offset`, the vertex V running
                // `bottomY * 0.25` to `topY * 0.25` past it (vanilla `prepareInstances`).
                // Snow: the u/v drift the column's random fixed, still spanning a full
                // texture width. The atlas rect is in texels; the UVs may leave the rect
                // (vanilla's sampler REPEATs), and the fragment wraps them back inside.
                let r = rect.map(|v| v as f32);
                let u0 = r[0] + c.u_offset * r[2];
                let u1 = u0 + r[2];
                let v0 = r[1] + (c.v_offset + c.bottom_y as f32 * 0.25) * r[3];
                let v1 = r[1] + (c.v_offset + c.top_y as f32 * 0.25) * r[3];
                let rect16 = rect.map(|v| v as u16);
                let light = [c.light.0 as u32, c.light.1 as u32];
                let color = [1.0, 1.0, 1.0, alpha];
                let corners = [
                    ([x0, y1, z0], [u0, v0]),
                    ([x1, y1, z1], [u1, v0]),
                    ([x1, y0, z1], [u1, v1]),
                    ([x0, y0, z0], [u0, v1]),
                ];
                for i in [0usize, 1, 2, 0, 2, 3] {
                    out.push(WeatherVertex {
                        pos: corners[i].0,
                        uv: corners[i].1,
                        color,
                        rect: rect16,
                        light,
                    });
                }
            }
        };
        push(&mut self.vertices, &weather.rain, self.rain_rect, 1.0);
        self.snow_start = self.vertices.len() as u32;
        push(&mut self.vertices, &weather.snow, self.snow_rect, 0.8);
        self.vertex_count = self.vertices.len() as u32;
        self.drawn = (weather.rain.len() + weather.snow.len()) as u32;
        // One-time diagnostics: what the pass received (columns, vertices, intensity).
        if !self.logged {
            self.logged = true;
            tracing::info!(
                "weather pass: {} rain + {} snow columns, {} vertices, intensity {:.3}, \
                 rects rain {:?} snow {:?}",
                weather.rain.len(),
                weather.snow.len(),
                self.vertex_count,
                weather.intensity,
                self.rain_rect,
                self.snow_rect
            );
        }
        let _ = (game_time, partial_tick);
        if self.vertices.len() > self.capacity {
            self.capacity = self.vertices.len().next_power_of_two();
            self.vertex_buffer = make_buffer(&self.device, self.capacity);
        }
        if !self.vertices.is_empty() {
            self.queue
                .write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.vertices));
        }
        let u = WeatherUniforms {
            view_proj: view_proj.to_cols_array_2d(),
            fog_color: [fog_color[0], fog_color[1], fog_color[2], 1.0],
            fog_env_start: fog.environmental_start,
            fog_env_end: fog.environmental_end,
            fog_rd_start: fog.render_distance_start,
            fog_rd_end: fog.render_distance_end,
            hw_srgb: f32::from(self.hw_srgb),
            _pad: [0.0; 3],
        };
        self.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));
    }

    /// Draws the rain then snow columns into the main pass (after translucent terrain,
    /// where the sky renderer's overlays run).
    pub fn draw<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) -> u32 {
        if self.vertex_count == 0 {
            return 0;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..self.snow_start, 0..1);
        pass.draw(self.snow_start..self.vertex_count, 0..1);
        2
    }

    /// Builds and uploads this frame's lightning bolts (vanilla
    /// `LightningBoltRenderer.submit`): the offsets walk down from the seed, then four
    /// layers of three branches each draw their segment quads. `bolts` are camera-relative.
    pub fn prepare_lightning(&mut self, bolts: &[LightningBolt]) {
        self.lightning_vertices.clear();
        for bolt in bolts {
            push_lightning(&mut self.lightning_vertices, bolt);
        }
        self.lightning_count = self.lightning_vertices.len() as u32;
        if self.lightning_vertices.len() > self.lightning_capacity {
            self.lightning_capacity = self.lightning_vertices.len().next_power_of_two();
            self.lightning_buffer = make_lightning_buffer(&self.device, self.lightning_capacity);
        }
        if !self.lightning_vertices.is_empty() {
            self.queue.write_buffer(
                &self.lightning_buffer,
                0,
                bytemuck::cast_slice(&self.lightning_vertices),
            );
        }
    }

    /// Draws the lightning bolts into the main pass (one draw).
    pub fn draw_lightning<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) -> u32 {
        if self.lightning_count == 0 {
            return 0;
        }
        pass.set_pipeline(&self.lightning_pipeline);
        pass.set_bind_group(0, &self.lightning_bind_group, &[]);
        pass.set_vertex_buffer(0, self.lightning_buffer.slice(..));
        pass.draw(0..self.lightning_count, 0..1);
        1
    }
}

/// One bolt's geometry (vanilla `LightningBoltRenderer.submit`): the segment offsets walk
/// down from a seeded random, then for each of the 4 layers each of the 3 branches draws
/// four quads per segment. The bolt spans y 0..128 (8 segments of 16) at the entity's
/// position.
fn push_lightning(out: &mut Vec<LightningVertex>, bolt: &LightningBolt) {
    let mut x_offsets = [0i32; 8];
    let mut z_offsets = [0i32; 8];
    let (mut x_offset, mut z_offset) = (0i32, 0i32);
    let mut random = JavaRandom(bolt.seed);
    for i in (0..8).rev() {
        x_offsets[i] = x_offset;
        z_offsets[i] = z_offset;
        x_offset += random.next_int(11) - 5;
        z_offset += random.next_int(11) - 5;
    }
    let (final_x, final_z) = (x_offset, z_offset);
    for layer in 0..4 {
        let mut randomx = JavaRandom(bolt.seed);
        for branch in 0..3 {
            let is_trunk = branch == 0;
            let branch_start = 7 - branch;
            let branch_end = if is_trunk { 0 } else { branch_start - 3 + 1 };
            let (mut segment_x, mut segment_z) = (
                x_offsets[branch_start] - final_x,
                z_offsets[branch_start] - final_z,
            );
            for segment in (branch_end..=branch_start).rev() {
                let (end_x, end_z) = (segment_x, segment_z);
                if is_trunk {
                    segment_x += randomx.next_int(11) - 5;
                    segment_z += randomx.next_int(11) - 5;
                } else {
                    segment_x += randomx.next_int(31) - 15;
                    segment_z += randomx.next_int(31) - 15;
                }
                let mut top_radius = 0.1 + layer as f32 * 0.2;
                let mut bottom_radius = top_radius;
                if is_trunk {
                    top_radius *= segment as f32 * 0.1 + 1.0;
                    bottom_radius *= (segment - 1) as f32 * 0.1 + 1.0;
                }
                // The four quads (their corner flags; vanilla `quad`'s booleans in order).
                let quad = |out: &mut Vec<LightningVertex>,
                            right_x: bool,
                            right_z: bool,
                            left_x: bool,
                            left_z: bool| {
                    let v = |b: bool, r: f32| if b { r } else { -r };
                    let y0 = (segment * 16) as f32;
                    let y1 = ((segment + 1) * 16) as f32;
                    let color = [0.45, 0.45, 0.5, 0.3];
                    let corners = [
                        (
                            [
                                bolt.pos[0] + segment_x as f32 + v(right_x, bottom_radius),
                                bolt.pos[1] + y0,
                                bolt.pos[2] + segment_z as f32 + v(right_z, bottom_radius),
                            ],
                            color,
                        ),
                        (
                            [
                                bolt.pos[0] + end_x as f32 + v(right_x, top_radius),
                                bolt.pos[1] + y1,
                                bolt.pos[2] + end_z as f32 + v(right_z, top_radius),
                            ],
                            color,
                        ),
                        (
                            [
                                bolt.pos[0] + end_x as f32 + v(left_x, top_radius),
                                bolt.pos[1] + y1,
                                bolt.pos[2] + end_z as f32 + v(left_z, top_radius),
                            ],
                            color,
                        ),
                        (
                            [
                                bolt.pos[0] + segment_x as f32 + v(left_x, bottom_radius),
                                bolt.pos[1] + y0,
                                bolt.pos[2] + segment_z as f32 + v(left_z, bottom_radius),
                            ],
                            color,
                        ),
                    ];
                    for i in [0usize, 1, 2, 0, 2, 3] {
                        out.push(LightningVertex {
                            pos: corners[i].0,
                            _pad: 0.0,
                            color: corners[i].1,
                        });
                    }
                };
                quad(out, false, false, true, false);
                quad(out, true, false, true, true);
                quad(out, true, true, false, true);
                quad(out, false, true, false, false);
            }
        }
    }
}

fn make_lightning_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("lightning-vertices"),
        size: (capacity * std::mem::size_of::<LightningVertex>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn make_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("weather-vertices"),
        size: (capacity * std::mem::size_of::<WeatherVertex>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_seed_matches_vanilla_expression() {
        // WeatherEffectRenderer: x * x * 3121 + x * 45238971 ^ z * z * 418711 + z * 13761.
        let x = 5i32;
        let z = -3i32;
        let vanilla = (x * x * 3121 + x * 45238971) ^ (z * z * 418711 + z * 13761);
        assert_eq!(column_seed(x, z), vanilla);
        // Wrapping: large coordinates stay in range.
        let _ = column_seed(1_000_000, -1_000_000);
    }

    #[test]
    fn column_size_matches_vanilla_table() {
        // The table at (z=0, x=1): deltaX=1-16 -> not this index; the table is indexed by
        // camera-relative offset. For offset (1, 0): (-0, 1)/1 = (0, 1).
        let (sx, sz) = column_size(1, 0);
        assert!((sx - 0.0).abs() < 1e-6 && (sz - 1.0).abs() < 1e-6);
        // Offset (-1, 0): (0, -1).
        let (sx, sz) = column_size(-1, 0);
        assert!((sx - 0.0).abs() < 1e-6 && (sz + 1.0).abs() < 1e-6);
        // Offset (0, 1): (-1, 0).
        let (sx, sz) = column_size(0, 1);
        assert!((sx + 1.0).abs() < 1e-6 && (sz - 0.0).abs() < 1e-6);
        // Offset (3, 4): length 5, so (-4/5, 3/5).
        let (sx, sz) = column_size(3, 4);
        assert!((sx + 0.8).abs() < 1e-6 && (sz - 0.6).abs() < 1e-6);
        // The camera's own block: vanilla divides by zero length; the column is degenerate
        // and draws nothing visible (a zero-width quad).
        let (sx, sz) = column_size(0, 0);
        assert_eq!((sx, sz), (0.0, 0.0));
    }

    #[test]
    fn alpha_fades_from_max_at_camera_to_half_at_radius() {
        // Vanilla: lerp(min(distSq/rSq, 1), maxAlpha, 0.5) * intensity.
        // At the camera's own block (distSq 0.5 of rSq 100): 1 - 0.005/2.
        let a = column_alpha(0.5, 0.5, 10, 1.0, 1.0);
        assert!((a - 0.9975).abs() < 1e-6, "{a}");
        // Snow at the camera: 0.8 + (0.5 - 0.8) * 0.005.
        let a = column_alpha(0.5, 0.5, 10, 0.8, 1.0);
        assert!((a - 0.7985).abs() < 1e-6, "{a}");
        // At the radius edge: 0.5 * intensity.
        let a = column_alpha(10.0, 0.0, 10, 1.0, 0.5);
        assert!((a - 0.25).abs() < 1e-6, "{a}");
        // Beyond the radius: clamped to the 0.5 end.
        let far = column_alpha(20.0, 0.0, 10, 1.0, 1.0);
        let edge = column_alpha(10.0, 0.0, 10, 1.0, 1.0);
        assert!((far - edge).abs() < 1e-6);
        // Halfway in squared distance: exactly maxAlpha + (0.5-maxAlpha) * 0.5.
        let a = column_alpha(5.0, 5.0, 10, 1.0, 1.0);
        assert!((a - 0.75).abs() < 1e-6, "{a}");
    }
}
