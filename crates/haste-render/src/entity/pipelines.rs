//! Entity renderer construction: the entity, shadow, beam and end-portal pipelines, the
//! lazily filled atlas texture and the reused vertex buffers.

use super::*;

impl EntityRenderer {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        target_format: wgpu::TextureFormat,
        camera_layout: &wgpu::BindGroupLayout,
        lightmap_layout: &wgpu::BindGroupLayout,
        font: Arc<haste_assets::BitmapFont>,
        assets: &mut Assets,
        baked: Arc<haste_assets::BakedBlockModels>,
        terrain_atlas: Arc<haste_assets::AtlasBuilder>,
    ) -> EntityRenderer {
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("entity-atlas"),
            size: wgpu::Extent3d {
                width: ATLAS_SIZE,
                height: ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("entity-layout"),
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
            label: Some("entity-bind"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(
                    &atlas.create_view(&Default::default()),
                ),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("entity"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../entity.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("entity-pipeline-layout"),
            bind_group_layouts: &[Some(camera_layout), Some(&layout), Some(lightmap_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("entity"),
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
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: Some(true),
                // Vanilla's entity pipelines (ENTITY_SNIPPET and friends) use
                // `DepthStencilState.DEFAULT` (GREATER_THAN_OR_EQUAL, reverse-Z);
                // with Haste's depth cleared to 1 that is LessEqual.
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        // `ENTITY_SHADOW` (RenderPipelines.ENTITY_SHADOW): same vertex format and atlas as the
        // entity pipeline, but translucent with the depth test kept and no depth write
        // (`DepthStencilState(GREATER_THAN_OR_EQUAL, false)`) and no back-face culling.
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("entity-shadow"),
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
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: Some(false),
                // Vanilla's `DepthStencilState(GREATER_THAN_OR_EQUAL, false)` (reverse-Z) is
                // LessEqual against Haste's depth cleared to 1. The shadow quad is coplanar
                // with the block top, and vanilla offsets it a hair towards the camera
                // (`VIEW_OFFSET_Z_LAYERING`); the terrain's u16 position quantization
                // (24 blocks over 65536 steps) can land the top up to half a 0.37 mm step
                // nearer than the shadow's exact plane, so the bias must cover ~6 depth24
                // units of that quantization noise. -16 does with margin and moves the
                // shadow at most half a centimetre at the pass's 16-block range.
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: -16,
                    slope_scale: 0.0,
                    clamp: 0.0,
                },
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let vertex_capacity = 8192;
        let vertex_buffer = make_vertex_buffer(&device, vertex_capacity);
        let shadow_capacity = 2048;
        let shadow_buffer = make_vertex_buffer(&device, shadow_capacity);
        let hand_buffer = make_vertex_buffer(&device, 1024);
        let beam_buffer = make_vertex_buffer(&device, 1024);
        let glow_buffer = make_vertex_buffer(&device, 1024);
        let portal_buffer = make_vertex_buffer(&device, 256);
        // The beam pipelines share the entity vertex format and atlas bind group (group 1)
        // but skip the lightmap (vanilla's beam shader has no lighting); the portal pipeline
        // is position-only over its own two-texture bind group.
        let beam_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("beam"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../beam.wgsl").into()),
        });
        let beam_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("beam-pipeline-layout"),
            bind_group_layouts: &[Some(camera_layout), Some(&layout)],
            immediate_size: 0,
        });
        let beam_state = |blend: Option<wgpu::BlendState>, depth_write: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(if blend.is_some() {
                    "beam-glow"
                } else {
                    "beam-opaque"
                }),
                layout: Some(&beam_layout),
                vertex: wgpu::VertexState {
                    module: &beam_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(EntityVertex::LAYOUT)],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &beam_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth24Plus,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let beam_opaque = beam_state(None, true);
        let beam_glow = beam_state(Some(wgpu::BlendState::ALPHA_BLENDING), false);
        // The end portal: position-only vertices, its own two samplers (the End sky and the
        // portal starfield from the jar), opaque with depth write (vanilla END_PORTAL).
        let portal_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("endportal"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../endportal.wgsl").into()),
        });
        let mut put_texture = |id: &str| -> wgpu::TextureView {
            let img = assets
                .texture(&haste_core::Identifier::minecraft(id))
                .unwrap_or_else(|e| {
                    tracing::warn!("end portal texture {id}: {e:#}");
                    image::RgbaImage::from_pixel(16, 16, image::Rgba([64, 0, 64, 255]))
                });
            let (w, h) = img.dimensions();
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("portal-texture"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                img.as_raw(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * w),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            texture.create_view(&Default::default())
        };
        let end_sky = put_texture("environment/end_sky");
        let end_portal = put_texture("entity/end_portal/end_portal");
        let portal_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("portal-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let portal_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("portal-bind"),
            layout: &portal_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&end_sky),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&end_portal),
                },
            ],
        });
        let portal_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("portal-pipeline-layout"),
                bind_group_layouts: &[Some(camera_layout), Some(&portal_layout)],
                immediate_size: 0,
            });
        let portal_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("endportal"),
            layout: Some(&portal_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &portal_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    // Position plus the layer count (15 portal, 16 gateway): the count rides
                    // per vertex so one pipeline serves both (vanilla uses two shader
                    // defines; wgpu here has no pipeline constants).
                    array_stride: 16,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                        wgpu::VertexAttribute {
                            offset: 12,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32,
                        },
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &portal_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
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
        // The boats' water mask (vanilla `RenderPipelines.WATER_MASK`): position-only
        // vertices, a colour target whose writes are all masked off, and the depth write
        // kept, so the patch hides the water inside a hull without painting anything.
        let mask_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("water-mask"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../water_mask.wgsl").into()),
        });
        let mask_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("water-mask-layout"),
            bind_group_layouts: &[Some(camera_layout)],
            immediate_size: 0,
        });
        let water_mask_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("water-mask"),
            layout: Some(&mask_layout),
            vertex: wgpu::VertexState {
                module: &mask_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(crate::entity::boat::MaskVertex::LAYOUT)],
            },
            fragment: Some(wgpu::FragmentState {
                module: &mask_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::empty(),
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
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
        let mask_capacity = 256;
        let mask_buffer = crate::entity::boat::make_mask_buffer(&device, mask_capacity);
        EntityRenderer {
            device,
            queue,
            pipeline,
            bind_group,
            atlas,
            shelf: Shelf::default(),
            textures: HashMap::new(),
            vertex_buffer,
            vertex_capacity,
            vertices: Vec::new(),
            shadow_pipeline,
            shadow_buffer,
            shadow_capacity,
            shadow_vertices: Vec::new(),
            nametags: Vec::new(),
            hand_buffer,
            hand_capacity: 1024,
            hand: Vec::new(),
            item_activation: None,
            item_names: Vec::new(),
            item_models: HashMap::new(),
            arm_models: [None, None, None, None, None, None, None, None],
            frame_models: HashMap::new(),
            piston_quads: ahash::AHashMap::new(),
            baked,
            terrain_atlas,
            minecart_quads: ahash::AHashMap::new(),
            beam_vertices: Vec::new(),
            beam_buffer,
            beam_capacity: 1024,
            glow_vertices: Vec::new(),
            glow_buffer,
            glow_capacity: 1024,
            portal_vertices: Vec::new(),
            portal_buffer,
            portal_capacity: 256,
            beam_radius_scale: 1.0,
            view_scale: 1.0,
            mask_vertices: Vec::new(),
            mask_buffer,
            mask_capacity,
            water_mask_pipeline,
            beams: crate::blockentity::BeamPipelines {
                opaque: beam_opaque,
                glow: beam_glow,
                portal: portal_pipeline,
                portal_bind_group,
            },
            font,
            equipment_infos: HashMap::new(),
            maps: Arc::new(HashMap::new()),
            map_textures: HashMap::new(),
            map_decoration_rects: HashMap::new(),
            map_decoration_names: Vec::new(),
            drawn: 0,
        }
    }
}
