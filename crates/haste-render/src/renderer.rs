// The Renderer (docs/INTERFACES.md): wgpu device, atlas upload, section
// submission, camera/frustum, per-frame render with sky colour and fog.
// Offscreen construction + readback for oracles's screenshot tests.
// Shading design from Pomme (chunk shaders); see THIRD_PARTY.md.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Arc;

use anyhow::{Context, Result};
use haste_core::view::BlockView;
use haste_core::{ChunkPos, SectionPos};
use wgpu::util::DeviceExt;

use crate::mesher::{Mesher, SectionMesh};
use crate::pipeline::{upload_atlas, upload_section, Gpu, SectionGpuMesh};
use crate::vertex::PackedVertex;

/// Camera for one frame (docs/INTERFACES.md FrameInput).
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub pos: glam::DVec3,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_deg: f32,
}

/// Sky/fog state for one frame.
#[derive(Clone, Copy, Debug)]
pub struct SkyState {
    pub time_of_day: i64,
    pub rain: f32,
    pub dimension_has_sky: bool,
    pub fog_start: f32,
    pub fog_end: f32,
}

/// Per-frame render statistics (docs/INTERFACES.md FrameStats).
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    pub frame_ms: f32,
    pub chunk_sections_drawn: u32,
    pub chunk_sections_total: u32,
    pub meshes_pending: u32,
    pub draw_calls: u32,
}

/// One frame's draw input.
pub struct FrameInput<'a> {
    pub camera: Camera,
    pub entities: &'a [haste_core::model::EntityInstance],
    pub gui: &'a haste_core::gui::GuiFrame,
    pub sky: SkyState,
    pub partial_tick: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs the client jar, datagen reports, and a Vulkan driver (source scripts/env.sh)"]
    fn offscreen_render_flat_world() {
        let jar = haste_core::paths::client_jar();
        if !jar.exists() {
            eprintln!("ignored: needs the client jar in the cache");
            return;
        }
        let mut assets = haste_assets::Assets::open(
            &jar,
            &haste_core::paths::cache_dir().join("assets"),
            "26.3",
        )
        .unwrap();
        let states = haste_assets::BlocksJsonStates::load(
            &haste_core::paths::datagen_reports().join("blocks.json"),
        )
        .unwrap();

        let mut renderer = Renderer::new_offscreen(320, 240, &mut assets, &states).unwrap();

        // A flat stone world at y=0.
        struct Flat;
        impl haste_core::view::BlockView for Flat {
            fn block_state(&self, p: haste_core::BlockPos) -> haste_core::BlockStateId {
                if p.y == 0 {
                    haste_core::BlockStateId(1)
                } else {
                    haste_core::BlockStateId::AIR
                }
            }
            fn is_chunk_loaded(&self, _: haste_core::ChunkPos) -> bool {
                true
            }
            fn sky_light(&self, p: haste_core::BlockPos) -> u8 {
                if p.y <= 0 {
                    0
                } else {
                    15
                }
            }
            fn block_light(&self, _: haste_core::BlockPos) -> u8 {
                0
            }
            fn min_y(&self) -> i32 {
                -64
            }
            fn height(&self) -> i32 {
                384
            }
        }

        // Mesh and upload a few sections around the origin.
        let dirty: Vec<SectionPos> = (-1..=1)
            .flat_map(|x| (-1..=1).map(move |z| SectionPos::new(x, 0, z)))
            .collect();
        renderer.update_world(&Flat, &dirty, ChunkPos::new(0, 0), 4);

        let frame = FrameInput {
            camera: Camera {
                pos: glam::DVec3::new(8.0, 20.0, 8.0),
                yaw: 0.0,
                pitch: 60.0,
                fov_deg: 70.0,
            },
            entities: &[],
            gui: &haste_core::gui::GuiFrame::default(),
            sky: SkyState {
                time_of_day: 6000,
                rain: 0.0,
                dimension_has_sky: true,
                fog_start: 100.0,
                fog_end: 200.0,
            },
            partial_tick: 0.0,
        };
        let stats = renderer.render(&frame).unwrap();
        eprintln!(
            "offscreen frame: {:.2} ms, {} drawn / {} sections, {} draw calls",
            stats.frame_ms,
            stats.chunk_sections_drawn,
            stats.chunk_sections_total,
            stats.draw_calls
        );
        assert!(stats.chunk_sections_drawn > 0, "no sections drawn");

        let image = renderer.read_back().unwrap();
        assert_eq!(image.dimensions(), (320, 240));
        // The frame must not be entirely the clear colour: some pixel differs
        // from the sky blue (0.47, 0.65, 1.0) => (120, 165, 255).
        let sky = image::Rgba([120, 165, 255, 255]);
        let any_different = image.pixels().any(|p| p.0 != sky.0);
        assert!(
            any_different,
            "frame is entirely clear colour; terrain not rendered"
        );
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    cam_pos: [f32; 4],
    view_proj: [[f32; 4]; 4],
    fog_color: [f32; 4],
    fog_start: f32,
    fog_end: f32,
    _pad: [f32; 2],
}

/// The renderer. Construct with a window (windowed mode) or offscreen
/// (`new_offscreen`, oracles's screenshot tests: fixed-size texture target,
/// `read_back` returns the frame as an RgbaImage).
#[allow(dead_code)]
pub struct Renderer {
    gpu: Gpu,
    surface: Option<wgpu::Surface<'static>>,
    surface_format: wgpu::TextureFormat,
    surface_state: Option<(u32, u32, wgpu::SurfaceConfiguration)>,
    offscreen: Option<Offscreen>,
    atlas: Arc<haste_assets::AtlasBuilder>,
    baked: Arc<haste_assets::BakedBlockModels>,
    mesher: Mesher,
    sections: HashMap<SectionPos, Vec<SectionGpuMesh>>,
    camera_uniform: wgpu::Buffer,
    camera_bind_group_layout: wgpu::BindGroupLayout,
    camera_bind_group: wgpu::BindGroup,
    atlas_bind_group_layout: wgpu::BindGroupLayout,
    atlas_bind_group: wgpu::BindGroup,
    pipeline_solid: wgpu::RenderPipeline,
    pipeline_cutout: wgpu::RenderPipeline,
    pipeline_translucent: wgpu::RenderPipeline,
    depth: Option<wgpu::TextureView>,
    size: (u32, u32),
}

#[allow(dead_code)]
struct Offscreen {
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
    width: u32,
    height: u32,
}

impl Renderer {
    /// Windowed constructor (docs/INTERFACES.md).
    pub fn new(
        window: Arc<winit::window::Window>,
        assets: &mut haste_assets::Assets,
        states: &dyn haste_core::view::BlockStateTable,
    ) -> Result<Renderer> {
        let display = DisplayHandleWrapper {
            window: window.clone(),
        };
        let display: Box<dyn crate::pipeline::ShareableDisplayHandle> = Box::new(display);
        let gpu = Gpu::new(Some(display))?;
        let surface = gpu
            .instance
            .create_surface(wgpu::SurfaceTarget::Window(Box::new(window.clone())))?;
        let cap = surface.get_capabilities(&gpu.adapter);
        let format = cap
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(cap.formats[0]);
        let size = window.inner_size();
        let mut renderer = Self::init(
            gpu,
            Some((surface, format)),
            (size.width.max(1), size.height.max(1)),
            assets,
            states,
        )?;
        renderer.configure_surface();
        Ok(renderer)
    }

    /// Offscreen constructor (oracles screenshot tests): renders into a texture.
    pub fn new_offscreen(
        w: u32,
        h: u32,
        assets: &mut haste_assets::Assets,
        states: &dyn haste_core::view::BlockStateTable,
    ) -> Result<Renderer> {
        let gpu = Gpu::new(None)?;
        Self::init(gpu, None, (w.max(1), h.max(1)), assets, states)
    }

    #[allow(clippy::too_many_arguments)]
    fn init(
        gpu: Gpu,
        surface: Option<(wgpu::Surface<'static>, wgpu::TextureFormat)>,
        size: (u32, u32),
        assets: &mut haste_assets::Assets,
        states: &dyn haste_core::view::BlockStateTable,
    ) -> Result<Renderer> {
        // Bake + atlas.
        let mut atlas = haste_assets::AtlasBuilder::build(assets, &[], false)?;
        let baked = haste_assets::BakedBlockModels::bake(assets, states, &mut atlas)?;
        let baked = Arc::new(baked);
        let atlas = Arc::new(atlas);

        let (_texture, view, sampler, sprite_rects, _len) =
            upload_atlas(&gpu.device, &gpu.queue, &atlas)?;

        // Camera uniform.
        let camera_uniform = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("camera"),
                contents: &[0; std::mem::size_of::<CameraUniform>()],
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let camera_bind_group_layout =
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("camera-layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });
        let camera_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera-bind"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_uniform.as_entire_binding(),
            }],
        });

        // Atlas bind group (texture, sampler, sprite rects).
        let atlas_bind_group_layout =
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("atlas-layout"),
                    entries: &[
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
                        wgpu::BindGroupLayoutEntry {
                            binding: 3,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: true },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                    ],
                });
        let atlas_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atlas-bind"),
            layout: &atlas_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: sprite_rects.as_entire_binding(),
                },
            ],
        });

        // Pipelines: one per render pass family (solid+cutout_mipped opaque
        // with mips; cutout discard; translucent blend, no depth write).
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("chunk"),
                source: wgpu::ShaderSource::Wgsl(include_str!("chunk.wgsl").into()),
            });
        let surface_format = surface
            .as_ref()
            .map(|(_, f)| *f)
            .unwrap_or(wgpu::TextureFormat::Rgba8UnormSrgb);
        let make_pipeline = |label: &str, blend: Option<wgpu::BlendState>| {
            gpu.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(&gpu.device.create_pipeline_layout(
                        &wgpu::PipelineLayoutDescriptor {
                            label: Some("chunk-layout"),
                            bind_group_layouts: &[
                                Some(&camera_bind_group_layout),
                                Some(&atlas_bind_group_layout),
                            ],
                            immediate_size: 0,
                        },
                    )),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_main"),
                        compilation_options: Default::default(),
                        buffers: &[
                            Some(PackedVertex::LAYOUT),
                            Some(PackedVertex::INSTANCE_LAYOUT),
                        ],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_main"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: surface_format,
                            blend,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleList,
                        cull_mode: Some(wgpu::Face::Back),
                        ..Default::default()
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth24Plus,
                        depth_write_enabled: Some(true),
                        depth_compare: Some(wgpu::CompareFunction::Less),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                })
        };
        let pipeline_solid = make_pipeline("chunk-solid", None);
        let pipeline_cutout = make_pipeline("chunk-cutout", None);
        let pipeline_translucent = make_pipeline(
            "chunk-translucent",
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );

        // Offscreen targets when windowless.
        let offscreen = if surface.is_none() {
            Some(Self::make_offscreen(&gpu.device, size, surface_format)?)
        } else {
            None
        };

        let mesher = Mesher::new(baked.clone(), atlas.clone());

        Ok(Renderer {
            gpu,
            surface: surface.map(|(s, _)| s),
            surface_format,
            surface_state: None,
            offscreen,
            atlas,
            baked,
            mesher,
            sections: HashMap::new(),
            camera_uniform,
            camera_bind_group_layout,
            camera_bind_group,
            atlas_bind_group_layout,
            atlas_bind_group,
            pipeline_solid,
            pipeline_cutout,
            pipeline_translucent,
            depth: None,
            size,
        })
    }

    fn make_offscreen(
        device: &wgpu::Device,
        size: (u32, u32),
        format: wgpu::TextureFormat,
    ) -> Result<Offscreen> {
        let (w, h) = size;
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen-color"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen-depth"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth24Plus,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        Ok(Offscreen {
            color_view: color.create_view(&Default::default()),
            depth_view: depth.create_view(&Default::default()),
            color,
            depth,
            width: w,
            height: h,
        })
    }

    fn configure_surface(&mut self) {
        let Some(surface) = &self.surface else { return };
        let (w, h) = self.size;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: self.surface_format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: w,
            height: h,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&self.gpu.device, &config);
        self.surface_state = Some((w, h, config));
        self.ensure_depth();
    }

    fn ensure_depth(&mut self) {
        let depth = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width: self.size.0,
                height: self.size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth24Plus,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.depth = Some(depth.create_view(&Default::default()));
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.size = (w.max(1), h.max(1));
        if self.offscreen.is_some() {
            if let Ok(off) = Self::make_offscreen(&self.gpu.device, self.size, self.surface_format)
            {
                self.offscreen = Some(off);
            }
        } else {
            self.configure_surface();
        }
    }

    /// Submit dirty sections to the mesher (snapshots taken from `world`) and
    /// upload finished meshes (per-frame budget).
    pub fn update_world(
        &mut self,
        world: &dyn BlockView,
        dirty: &[SectionPos],
        _camera_chunk: ChunkPos,
        _view_distance: i32,
    ) {
        for pos in dirty {
            self.mesher.submit(world, *pos);
        }
        for mesh in self.mesher.drain(32) {
            self.upload_mesh(mesh);
        }
    }

    fn upload_mesh(&mut self, mesh: SectionMesh) {
        let pos = mesh.pos;
        let uploads = upload_section(&self.gpu.device, &mesh);
        self.sections.insert(pos, uploads);
    }

    /// Build the view-projection matrix (right-handed, OpenGL-style depth like
    /// wgpu's default reverse... wgpu uses 0..1 z; glam's perspective_rh
    /// produces -1..1, so use perspective_infinite_rh style correction).
    fn view_projection(camera: &Camera, aspect: f32) -> glam::Mat4 {
        let proj = glam::Mat4::perspective_rh(camera.fov_deg.to_radians(), aspect, 0.05, 4096.0);
        let yaw = camera.yaw.to_radians();
        let pitch = camera.pitch.to_radians();
        // Minecraft: yaw 0 = +z (south), pitch positive = down.
        let forward = glam::Vec3::new(
            -pitch.cos() * yaw.sin(),
            -pitch.sin(),
            pitch.cos() * yaw.cos(),
        )
        .normalize();
        let eye = glam::Vec3::new(
            camera.pos.x as f32,
            camera.pos.y as f32,
            camera.pos.z as f32,
        );
        let view = glam::Mat4::look_to_rh(eye, forward, glam::Vec3::Y);
        proj * view
    }

    /// Frustum culling of sections (Pomme cull.comp reference; CPU here).
    fn frustum_planes(m: &glam::Mat4) -> [glam::Vec4; 6] {
        // Gribb/Hartmann column extraction from the combined matrix.
        let c = |i: usize| m.col(i);
        let (c0, c1, c2, c3) = (c(0), c(1), c(2), c(3));
        [c3 + c0, c3 - c0, c3 + c1, c3 - c1, c3 + c2, c3 - c2].map(|row| row.normalize())
    }

    fn aabb_visible(planes: &[glam::Vec4; 6], min: [f32; 3], max: [f32; 3]) -> bool {
        // Conservative: AABB vs plane with the positive vertex.
        for p in planes {
            let px = if p.x >= 0.0 { max[0] } else { min[0] };
            let py = if p.y >= 0.0 { max[1] } else { min[1] };
            let pz = if p.z >= 0.0 { max[2] } else { min[2] };
            if p.x * px + p.y * py + p.z * pz + p.w < 0.0 {
                return false;
            }
        }
        true
    }

    /// Vanilla sky colour by time of day (approximate: angle-based blend).
    fn sky_color(sky: &SkyState) -> [f32; 3] {
        if !sky.dimension_has_sky {
            return [0.02, 0.02, 0.05];
        }
        // 24000 ticks per day; noon at 6000. Angle 0 = sunrise.
        let angle = (sky.time_of_day as f32 / 24000.0) * std::f32::consts::TAU;
        let daylight = (angle.cos().clamp(-1.0, 1.0) + 0.6).clamp(0.0, 1.0);
        let day = [0.47, 0.65, 1.0];
        let night = [0.01, 0.01, 0.03];
        [
            night[0] + (day[0] - night[0]) * daylight,
            night[1] + (day[1] - night[1]) * daylight,
            night[2] + (day[2] - night[2]) * daylight,
        ]
    }

    /// Render one frame. Returns stats (frame time measured around the submit).
    pub fn render(&mut self, frame: &FrameInput) -> Result<FrameStats> {
        let started = std::time::Instant::now();
        let aspect = self.size.0 as f32 / self.size.1 as f32;
        let view_proj = Self::view_projection(&frame.camera, aspect);
        let fog_color = Self::sky_color(&frame.sky);
        let camera_uniform = CameraUniform {
            cam_pos: [
                frame.camera.pos.x as f32,
                frame.camera.pos.y as f32,
                frame.camera.pos.z as f32,
                0.0,
            ],
            view_proj: view_proj.to_cols_array_2d(),
            fog_color: [fog_color[0], fog_color[1], fog_color[2], 0.0],
            fog_start: frame.sky.fog_start,
            fog_end: frame.sky.fog_end,
            _pad: [0.0; 2],
        };
        self.gpu
            .queue
            .write_buffer(&self.camera_uniform, 0, bytemuck::bytes_of(&camera_uniform));

        // Which target are we drawing into?
        // The acquired SurfaceTexture must outlive the pass (it presents on drop
        // after submit); hold it alongside the views.
        let mut held_surface_texture: Option<wgpu::SurfaceTexture> = None;
        let (color_view, depth_view) = match (&self.offscreen, &self.depth) {
            (Some(off), _) => (off.color_view.clone(), off.depth_view.clone()),
            (None, Some(d)) => {
                let surface = self.surface.as_ref().context("no surface")?;
                let frame_tex = match surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(t)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
                    wgpu::CurrentSurfaceTexture::Timeout
                    | wgpu::CurrentSurfaceTexture::Occluded => {
                        anyhow::bail!("surface frame unavailable (timeout/occluded)")
                    }
                    wgpu::CurrentSurfaceTexture::Outdated
                    | wgpu::CurrentSurfaceTexture::Lost
                    | wgpu::CurrentSurfaceTexture::Validation => {
                        anyhow::bail!("surface lost/outdated; reconfigure needed")
                    }
                };
                held_surface_texture = Some(frame_tex);
                let tex = held_surface_texture.as_ref().unwrap();
                (tex.texture.create_view(&Default::default()), d.clone())
            }
            _ => anyhow::bail!("no render target"),
        };
        let present = held_surface_texture.is_some();
        let _ = present;

        // Frustum cull. Collect owned draw data so later buffer work does not
        // borrow self.sections.
        let planes = Self::frustum_planes(&view_proj);
        struct Draw<'a> {
            layer: haste_assets::RenderLayer,
            index_count: u32,
            vertex_buffer: &'a wgpu::Buffer,
            index_buffer: &'a wgpu::Buffer,
            origin: [i32; 3],
            dist: f32,
        }
        let mut draws: Vec<Draw> = Vec::new();
        for meshes in self.sections.values() {
            for m in meshes {
                if Self::aabb_visible(&planes, m.aabb.0, m.aabb.1) {
                    let center = glam::Vec3::from([
                        (m.aabb.0[0] + m.aabb.1[0]) * 0.5,
                        (m.aabb.0[1] + m.aabb.1[1]) * 0.5,
                        (m.aabb.0[2] + m.aabb.1[2]) * 0.5,
                    ]);
                    let dc = center
                        - glam::Vec3::new(
                            frame.camera.pos.x as f32,
                            frame.camera.pos.y as f32,
                            frame.camera.pos.z as f32,
                        );
                    draws.push(Draw {
                        layer: m.layer,
                        index_count: m.index_count,
                        vertex_buffer: &m.vertex_buffer,
                        index_buffer: &m.index_buffer,
                        origin: [m.pos.x << 4, m.pos.y << 4, m.pos.z << 4],
                        dist: dc.length_squared(),
                    });
                }
            }
        }
        // Translucent far-to-near; opaque front-to-back for early-Z.
        draws.sort_by(|a, b| {
            a.layer.cmp(&b.layer).then(
                a.dist
                    .partial_cmp(&b.dist)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        });
        let sections_total = self.sections.len() as u32;
        let drawn = draws.len() as u32;

        let mut draw_calls = 0u32;
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: fog_color[0] as f64,
                            g: fog_color[1] as f64,
                            b: fog_color[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            pass.set_bind_group(1, &self.atlas_bind_group, &[]);

            // Layered draw order (RenderLayer Ord: Solid < Cutout < CutoutMipped
            // < Translucent), distance-sorted inside each. Section origins go
            // through an instance-rate vertex buffer (attribute 7), one
            // instance per section draw.
            let origins: Vec<[i32; 3]> = draws.iter().map(|d| d.origin).collect();
            let origin_buffer =
                self.gpu
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("section-origins"),
                        contents: bytemuck::cast_slice(&origins),
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    });
            for (instance, d) in draws.iter().enumerate() {
                let pipeline = match d.layer {
                    haste_assets::RenderLayer::Solid | haste_assets::RenderLayer::CutoutMipped => {
                        &self.pipeline_solid
                    }
                    haste_assets::RenderLayer::Cutout => &self.pipeline_cutout,
                    haste_assets::RenderLayer::Translucent => &self.pipeline_translucent,
                };
                pass.set_pipeline(pipeline);
                pass.set_vertex_buffer(0, d.vertex_buffer.slice(..));
                pass.set_vertex_buffer(1, origin_buffer.slice(..));
                pass.set_index_buffer(d.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..d.index_count, 0, instance as u32..instance as u32 + 1);
                draw_calls += 1;
            }
        }

        // Submit, then present (wgpu 30: present lives on the queue).
        self.gpu.queue.submit([encoder.finish()]);
        if let Some(tex) = held_surface_texture {
            self.gpu.queue.present(tex);
        }

        Ok(FrameStats {
            frame_ms: started.elapsed().as_secs_f32() * 1000.0,
            chunk_sections_drawn: drawn,
            chunk_sections_total: sections_total,
            meshes_pending: self.mesher.pending_len() as u32,
            draw_calls,
        })
    }

    /// Read the last offscreen frame back as an RGBA image (screenshot tests).
    pub fn read_back(&mut self) -> Result<image::RgbaImage> {
        let off = self
            .offscreen
            .as_ref()
            .context("read_back requires an offscreen renderer")?;
        let (w, h) = (off.width, off.height);
        let bytes_per_row = NonZeroU32::new(w * 4).unwrap();
        let buffer = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (w * h * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        let format = self.surface_format;
        let block_size = 4; // Rgba8UnormSrgb or Bgra8
        encoder.copy_texture_to_buffer(
            off.color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row.get()),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.gpu.queue.submit([encoder.finish()]);

        let (tx, rx) = std::sync::mpsc::channel();
        let buffer = std::sync::Arc::new(buffer);
        let map = buffer.clone();
        map.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        self.gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .ok();
        rx.recv()
            .context("waiting for the readback map")?
            .context("mapping the readback buffer")?;

        let data = map.get_mapped_range(..).context("mapped range")?.to_vec();
        map.unmap();
        let _ = block_size;
        let _ = format;

        // Rgba8UnormSrgb: the texture was created with this offscreen format.
        let mut image = image::RgbaImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                image.put_pixel(
                    x,
                    y,
                    image::Rgba([data[i], data[i + 1], data[i + 2], data[i + 3]]),
                );
            }
        }
        Ok(image)
    }
}

/// An owned display handle: the window Arc keeps the underlying connection
/// alive; the handle is fetched from the window on demand.
struct DisplayHandleWrapper {
    #[allow(dead_code)]
    window: Arc<winit::window::Window>,
}

impl core::fmt::Debug for DisplayHandleWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DisplayHandleWrapper")
            .finish_non_exhaustive()
    }
}

impl winit::raw_window_handle::HasDisplayHandle for DisplayHandleWrapper {
    fn display_handle(
        &self,
    ) -> Result<winit::raw_window_handle::DisplayHandle<'_>, winit::raw_window_handle::HandleError>
    {
        self.window.display_handle()
    }
}
