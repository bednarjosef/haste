//! The depth pyramid of the GPU occlusion culling: a mip chain of the farthest depth
//! over ever larger screen blocks, built from the depth buffer by compute passes
//! (`pyramid.wgsl`). Level 0 is half the target's size; each level halves again
//! (rounding down, like any mip chain) down to 1x1.

/// The pipelines and their layouts; one per renderer.
pub(super) struct PyramidPipelines {
    first_layout: wgpu::BindGroupLayout,
    down_layout: wgpu::BindGroupLayout,
    first: wgpu::ComputePipeline,
    down: wgpu::ComputePipeline,
}

/// The pyramid for one target size and depth buffer.
pub(super) struct Pyramid {
    /// The depth view the first level reads (rebuilt when the target changes).
    depth: wgpu::TextureView,
    /// Each level's size.
    sizes: Vec<(u32, u32)>,
    /// All levels, for the cull's `textureLoad` with an explicit level.
    pub(super) all_levels: wgpu::TextureView,
    /// Depth to level 0, then level n to n + 1.
    groups: Vec<wgpu::BindGroup>,
}

impl PyramidPipelines {
    pub(super) fn new(device: &wgpu::Device) -> PyramidPipelines {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("depth-pyramid"),
            source: wgpu::ShaderSource::Wgsl(include_str!("pyramid.wgsl").into()),
        });
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: wgpu::TextureFormat::R32Float,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        };
        let sampled = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let first_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("depth-pyramid-first"),
            entries: &[sampled(0, wgpu::TextureSampleType::Depth), storage(1)],
        });
        let down_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("depth-pyramid-down"),
            entries: &[
                storage(1),
                sampled(2, wgpu::TextureSampleType::Float { filterable: false }),
            ],
        });
        let pipeline = |layout: &wgpu::BindGroupLayout, entry: &str| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("depth-pyramid"),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        PyramidPipelines {
            first: pipeline(&first_layout, "reduce_depth"),
            down: pipeline(&down_layout, "reduce_level"),
            first_layout,
            down_layout,
        }
    }

    /// The pyramid for `depth` at `size`, reusing `old` when it was built for the
    /// same depth view.
    pub(super) fn pyramid(
        &self,
        device: &wgpu::Device,
        old: Option<Pyramid>,
        depth: &wgpu::TextureView,
        size: (u32, u32),
    ) -> Pyramid {
        if let Some(p) = old {
            if &p.depth == depth {
                return p;
            }
        }
        let sizes = level_sizes(size);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth-pyramid"),
            size: wgpu::Extent3d {
                width: sizes[0].0,
                height: sizes[0].1,
                depth_or_array_layers: 1,
            },
            mip_level_count: sizes.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let level = |l: u32| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: l,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let mut groups = vec![device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("depth-pyramid-first"),
            layout: &self.first_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&level(0)),
                },
            ],
        })];
        for l in 1..sizes.len() as u32 {
            groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("depth-pyramid-down"),
                layout: &self.down_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&level(l)),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&level(l - 1)),
                    },
                ],
            }));
        }
        Pyramid {
            depth: depth.clone(),
            sizes,
            all_levels: texture.create_view(&Default::default()),
            groups,
        }
    }

    /// Records the pyramid's build from its depth buffer into `pass`.
    pub(super) fn build(&self, pass: &mut wgpu::ComputePass<'_>, p: &Pyramid) {
        for (l, (group, &(w, h))) in p.groups.iter().zip(&p.sizes).enumerate() {
            pass.set_pipeline(if l == 0 { &self.first } else { &self.down });
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
    }
}

/// The pyramid's level sizes for a target of `size`: the texture's own mip sizes
/// (halved, rounded down) from half the target down to 1x1. The last texel of an odd
/// row or column covers the extra source texel (`block` in pyramid.wgsl).
pub(super) fn level_sizes(size: (u32, u32)) -> Vec<(u32, u32)> {
    let mut sizes = vec![((size.0 / 2).max(1), (size.1 / 2).max(1))];
    while *sizes.last().unwrap() != (1, 1) {
        let (w, h) = *sizes.last().unwrap();
        sizes.push(((w / 2).max(1), (h / 2).max(1)));
    }
    sizes
}

impl Pyramid {
    pub(super) fn levels(&self) -> u32 {
        self.sizes.len() as u32
    }
}
