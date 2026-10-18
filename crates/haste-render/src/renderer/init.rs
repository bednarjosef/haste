// Renderer construction (vanilla `Minecraft`'s one-time GPU setup): atlas bake
// and upload, camera/atlas/lightmap bind groups, terrain pipelines, and the
// per-pass renderers (sky, weather, entities, overlay, particles, GUI, blur).
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use wgpu::util::DeviceExt;

use crate::arena::MeshArenas;
use crate::mesher::Mesher;
use crate::pipeline::{upload_atlas, Gpu};
use crate::vertex::PackedVertex;

use super::{CameraUniform, LightmapFrame, Renderer};

impl Renderer {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn init(
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

        let (atlas_texture, view, sampler, sprite_rects, _len) =
            upload_atlas(&gpu.device, &gpu.queue, &atlas)?;
        let animation = crate::animation::SpriteAnimator::new(
            atlas.clone(),
            gpu.queue.clone(),
            atlas_texture.clone(),
        )?;

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
        // The hand's own uniform, same layout (drawn by the entity pipeline in its own pass).
        let hand_camera_uniform =
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("hand-camera"),
                    contents: &[0; std::mem::size_of::<CameraUniform>()],
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });
        let hand_camera_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hand-camera-bind"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: hand_camera_uniform.as_entire_binding(),
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

        // The vanilla light texture (LightTexture): one 16x16 texel map for all terrain and
        // entity shading, re-uploaded when its inputs change.
        let lightmap = crate::lightmap::LightmapTexture::new(
            &gpu.device,
            &gpu.queue,
            crate::lightmap::LightmapInputs::overworld_noon(),
        );
        let lightmap_bind_group_layout =
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("lightmap-layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });
        let lightmap_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lightmap-bind"),
            layout: &lightmap_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(lightmap.view()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(lightmap.sampler()),
                },
            ],
        });
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("chunk"),
                source: wgpu::ShaderSource::Wgsl(include_str!("../chunk.wgsl").into()),
            });
        let surface_format = surface
            .as_ref()
            .map(|(_, f)| *f)
            .unwrap_or(wgpu::TextureFormat::Rgba8UnormSrgb);
        let make_pipeline = |label: &str, fragment: &str, blend: Option<wgpu::BlendState>| {
            gpu.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(&gpu.device.create_pipeline_layout(
                        &wgpu::PipelineLayoutDescriptor {
                            label: Some("chunk-layout"),
                            bind_group_layouts: &[
                                Some(&camera_bind_group_layout),
                                Some(&atlas_bind_group_layout),
                                Some(&lightmap_bind_group_layout),
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
                        entry_point: Some(fragment),
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
                        // Vanilla's terrain depth test: TERRAIN_SNIPPET inherits
                        // `DepthStencilState.DEFAULT` (GREATER_THAN_OR_EQUAL), drawn
                        // against a depth cleared to 0. Haste clears to 1, so the
                        // same rule is LessEqual: a quad may share a depth (vanilla's
                        // coplanar model elements, e.g. the grass block's side
                        // overlay over its side texture, both draw).
                        depth_compare: Some(wgpu::CompareFunction::LessEqual),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                })
        };
        let pipeline_solid = make_pipeline("chunk-solid", "fs_main", None);
        let pipeline_cutout = make_pipeline("chunk-cutout", "fs_cutout", None);
        let pipeline_translucent = make_pipeline(
            "chunk-translucent",
            "fs_translucent",
            // Vanilla BlendFunction.TRANSLUCENT: (SRC_ALPHA, ONE_MINUS_SRC_ALPHA)
            // both equations — the fragment colour is NOT premultiplied (block.fsh
            // outputs the plain texel colour), so this and not the premultiplied
            // state is the faithful blend.
            Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
            }),
        );

        // Offscreen targets when windowless.
        let offscreen = if surface.is_none() {
            Some(Self::make_offscreen(
                &gpu.device,
                size,
                surface_format,
                Self::gui_format_for(&gpu, surface_format),
            )?)
        } else {
            None
        };

        let mesher = Mesher::new(baked.clone(), atlas.clone());
        let colormaps = crate::tint::Colormaps::load(assets).unwrap_or_else(|e| {
            tracing::warn!("colormaps unavailable ({e:#}): uniform grass/foliage colour");
            crate::tint::Colormaps::uniform(0x91BD59)
        });
        mesher.set_tables(crate::mesher::MeshTables::new(
            Arc::new(crate::tint::TintTable::new(states)),
            Arc::new(crate::tint::BiomeColors::new(&[], &colormaps)),
            Arc::new(crate::fluid::FluidTable::new(
                states,
                &crate::fluid::Fluids::vanilla(&crate::fluid::FluidSprites::from_atlas(&*atlas)),
                &crate::fluid::FluidRules::vanilla(),
                crate::fluid::FluidSprites::from_atlas(&*atlas),
            )),
            // No shape data at construction (the caller installs it with
            // `set_shapes`): no state occludes AO until then.
            Arc::new(crate::mesher::AoTable::all_solid(0)),
            // Fluid faces fall back to full-if-opaque until then.
            Arc::new(crate::fluid::FluidOccluders::absent()),
        ));
        let arenas = MeshArenas::new(gpu.device.clone(), gpu.queue.clone());
        let timer = crate::timing::GpuTimer::new(&gpu.device, &gpu.queue);
        let origin_buffer = Self::make_origin_buffer(&gpu.device, 4096);
        let indirect_buffer = Self::make_indirect_buffer(&gpu.device, 4096);

        // GUI: the font (with unifont when the assets index has it) and a renderer drawing into
        // a non-sRGB view of the target so blending happens on sRGB values like vanilla.
        let font = Arc::new(assets.font_default().context("loading the default font")?);
        let mut gui = crate::gui::GuiRenderer::new(
            gpu.device.clone(),
            gpu.queue.clone(),
            Self::gui_format_for(&gpu, surface_format),
            font.clone(),
        );
        let entity = crate::entity::EntityRenderer::new(
            gpu.device.clone(),
            gpu.queue.clone(),
            surface_format,
            &camera_bind_group_layout,
            &lightmap_bind_group_layout,
            font.clone(),
            assets,
        );
        let sky = crate::sky::SkyRenderer::new(
            gpu.device.clone(),
            gpu.queue.clone(),
            surface_format,
            assets,
        );
        let clouds = crate::clouds::CloudRenderer::new(
            gpu.device.clone(),
            gpu.queue.clone(),
            surface_format,
            assets,
        );
        let (rain_rect, snow_rect) = sky.weather_rects();
        let mut weather = crate::weather::WeatherRenderer::new(
            gpu.device.clone(),
            gpu.queue.clone(),
            surface_format,
            rain_rect,
            snow_rect,
        );
        weather.set_textures(
            &gpu.device,
            sky.atlas_view(),
            lightmap.view(),
            lightmap.sampler(),
        );
        let overlay = crate::overlay::OverlayRenderer::new(
            gpu.device.clone(),
            gpu.queue.clone(),
            surface_format,
            &camera_bind_group_layout,
            assets,
        );
        let particles = crate::particles::ParticleRenderer::new(
            gpu.device.clone(),
            gpu.queue.clone(),
            surface_format,
            &camera_bind_group_layout,
            &lightmap_bind_group_layout,
        );
        let mut gui_assets = assets.reopen()?;
        gui.preload_sprites(&mut gui_assets, crate::gui::HUD_SPRITES);

        // The menu background blur: its two internal textures use the non-sRGB view of the
        // target format (vanilla blurs the stored sRGB bytes of its RGBA8_UNORM target);
        // the frame's colour is copied in, so no bind group on the target itself.
        let blur = crate::blur::MenuBlur::new(
            &gpu.device,
            Self::gui_format_for(&gpu, surface_format),
            size,
        );

        let panorama = crate::panorama::PanoramaRenderer::new(
            &gpu.device,
            Self::gui_format_for(&gpu, surface_format),
        );
        let section_vis = Arc::new(Mutex::new(ahash::AHashMap::new()));
        let walk = crate::occlusion::AsyncOcclusionWalker::new(section_vis.clone())?;

        Ok(Renderer {
            gpu,
            surface: surface.map(|(s, _)| s),
            surface_format,
            surface_state: None,
            offscreen,
            atlas,
            atlas_texture,
            animation,
            animation_tick: 0,
            baked,
            mesher,
            colormaps,
            sections: crate::section_slab::SectionSlab::new(),
            arenas,
            section_vis,
            visible_mask: Default::default(),
            camera_spectator: false,
            cull_sections: true,
            visible_for: None,
            scheduled_for: None,
            walk_in_flight: false,
            bonus_list: Vec::new(),
            bonus_dirty: false,
            walk,
            vis_dirty: false,
            last_walk: std::time::Instant::now(),
            min_section: -4,
            section_count: 24,
            view_distance: 32,
            draw_list: Vec::new(),
            drain_buf: Vec::new(),
            mesh_carry: Vec::new(),
            layer_buckets: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            origins: Vec::new(),
            origin_buffer,
            origin_capacity: 4096,
            indirect: Vec::new(),
            indirect_buffer,
            indirect_capacity: 4096,
            camera_uniform,
            camera_bind_group_layout,
            camera_bind_group,
            hand_camera_uniform,
            hand_camera_bind_group,
            atlas_bind_group_layout,
            lightmap,
            lightmap_base: crate::lightmap::LightmapInputs::overworld_noon(),
            lightmap_frame: LightmapFrame::default(),
            lightmap_bind_group,
            atlas_bind_group,
            pipeline_solid,
            pipeline_cutout,
            pipeline_translucent,
            depth: None,
            size,
            gui,
            blur,
            panorama,
            menu_blur_radius: 5,
            blur_unavailable_logged: false,
            assets: gui_assets,
            entity,
            sky,
            clouds,
            weather,
            overlay,
            particles,
            first_person: None,
            timings: Default::default(),
            last_uploads: (0, 0),
            timer,
            present_mode: wgpu::PresentMode::Fifo,
            nametags: Vec::new(),
            entity_shadows: true,
            capture_pending: false,
            capture: None,
            surface_copy: false,
            surface_copy_logged: false,
            block_entities_drawn: 0,
            frame_game_time: 0,
            last_fog: None,
            lightning_bolts: Vec::new(),
        })
    }
}
