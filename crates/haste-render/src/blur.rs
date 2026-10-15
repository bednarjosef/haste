// Menu background blur (vanilla: GameRenderer.processBlurEffect, the `blur` post chain): the
// world (and the GUI strata before the screen's blur marker) is blurred behind an open screen
// that asks for it (Screen.extractBlurredBackground), with the radius from the accessibility
// option `menuBackgroundBlurriness`.
//
// Vanilla's chain (assets/minecraft/post_effect/blur.json) is six passes of the separable box
// blur (post/box_blur.fsh): horizontal, vertical, repeated three times, ping-ponging between
// the main target and one internal `swap` target, each sampling bilinear with a step of two
// texels (the loop from -radius+0.5 to radius plus a half-weight sample at the end, divided
// by radius+0.5). The JSON's `Radius` uniform is 0, which the shader maps to the global
// `MenuBlurRadius` — the option value.
//
// Haste's chain copies the frame's colour target into the first internal texture (the target
// itself cannot always be sampled: a windowed surface lacks TEXTURE_BINDING), then runs the
// same six passes over two internal textures, the last writing back into the target. All
// textures use the non-sRGB view of the target's format, so the stored sRGB bytes move
// through the blur raw like vanilla's RGBA8_UNORM main target. Targets are created once per
// size and reused; nothing allocates per frame.

/// Vanilla's `blur` post chain: three separable box-blur rounds (H then V), 6 passes total
/// (`post_effect/blur.json`).
pub const BLUR_PASSES: usize = 6;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlurUniforms {
    /// `BlurDir` in vanilla's shader: (1,0) horizontal, (0,1) vertical.
    dir: [f32; 2],
    /// `Radius`: vanilla's JSON leaves 0, which the shader replaces with the option's value.
    radius: f32,
    _pad: f32,
    /// `InSize` in vanilla's SamplerInfo: the input texture's size in texels.
    in_size: [f32; 2],
}

/// One uniform block per pass (binding offsets must be 256-byte aligned in min-ABP; pad the
/// struct to one block each so pass `i` binds `uniforms.slice(i * BLOCK..)`).
const BLOCK: u64 = 256;

/// The menu background blur pass. The two internal textures alternate roles as input and
/// output across the six passes (the target itself is only the source of the initial copy
/// and the destination of the last pass).
pub(crate) struct MenuBlur {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    /// The two internal textures at `size` (views for binding and as attachments).
    views: [wgpu::TextureView; 2],
    /// The textures themselves, kept for copying the frame's target in.
    textures: [wgpu::Texture; 2],
    /// One bind group per pass (six): the input alternates A/B and each binds its own
    /// uniform block.
    groups: [wgpu::BindGroup; BLUR_PASSES],
    size: (u32, u32),
}

impl MenuBlur {
    /// `format` must be a non-sRGB view of the main target's format (vanilla blurs the stored
    /// sRGB bytes of its RGBA8_UNORM main target).
    pub(crate) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: (u32, u32),
    ) -> MenuBlur {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blur"),
            source: wgpu::ShaderSource::Wgsl(include_str!("blur.wgsl").into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blur-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blur"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("blur-pipeline-layout"),
                    bind_group_layouts: &[Some(&bind_group_layout)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("blur-uniforms"),
            size: BLOCK * BLUR_PASSES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("blur-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let textures = std::array::from_fn(|i| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(if i == 0 { "blur-a" } else { "blur-b" }),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        });
        let views = std::array::from_fn(|i| textures[i].create_view(&Default::default()));
        // Six groups, one per pass: the input texture alternates A/B/A/B/A/B, and each binds
        // its own uniform slice (binding offsets must be 256-aligned).
        let uniform_size =
            std::num::NonZeroU64::new(std::mem::size_of::<BlurUniforms>() as u64).unwrap();
        let groups = std::array::from_fn(|pass| {
            let input = if pass % 2 == 0 { 0 } else { 1 };
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(if pass % 2 == 0 {
                    "blur-group-a"
                } else {
                    "blur-group-b"
                }),
                layout: &bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&views[input]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &uniforms,
                            offset: BLOCK * pass as u64,
                            size: Some(uniform_size),
                        }),
                    },
                ],
            })
        });
        MenuBlur {
            pipeline,
            bind_group_layout,
            uniforms,
            views,
            textures,
            groups,
            size,
        }
    }

    /// (Re)creates the internal textures when the target size changed; no-op otherwise.
    pub(crate) fn resize(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: (u32, u32),
    ) {
        if self.size == size {
            return;
        }
        let blur = Self::new(device, format, size);
        self.pipeline = blur.pipeline;
        self.bind_group_layout = blur.bind_group_layout;
        self.uniforms = blur.uniforms;
        self.views = blur.views;
        self.textures = blur.textures;
        self.groups = blur.groups;
        self.size = blur.size;
    }

    /// Runs the blur chain (vanilla: `GameRenderer.processBlurEffect` -> the `blur` post
    /// chain's six passes): copies the frame's colour target into the first internal
    /// texture, blurs horizontal/vertical three times between the two internal textures,
    /// and writes the last pass into `target_view`. `radius` is the option value.
    pub(crate) fn run(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        radius: u32,
        target_view: &wgpu::TextureView,
        target_copy_src: wgpu::TexelCopyTextureInfo,
    ) {
        if radius == 0 || self.size.0 == 0 || self.size.1 == 0 {
            return;
        }
        let size = self.size;
        // The frame's colour bytes into the first internal texture ("main" of vanilla's
        // ping-pong; the target itself cannot always be sampled).
        encoder.copy_texture_to_texture(
            target_copy_src,
            wgpu::TexelCopyTextureInfo {
                texture: &self.textures[0],
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
        );
        // One uniform block per pass, all written before the passes are recorded (a
        // mid-recording write_buffer is not ordered between passes).
        for pass_index in 0..BLUR_PASSES {
            let uniforms = BlurUniforms {
                dir: if pass_index % 2 == 0 {
                    [1.0, 0.0]
                } else {
                    [0.0, 1.0]
                },
                radius: radius as f32,
                _pad: 0.0,
                in_size: [size.0 as f32, size.1 as f32],
            };
            // Pad each block to `BLOCK` bytes so pass i binds slice(i * BLOCK..).
            let mut block = [0u8; BLOCK as usize];
            block[..std::mem::size_of::<BlurUniforms>()]
                .copy_from_slice(bytemuck::bytes_of(&uniforms));
            queue.write_buffer(&self.uniforms, BLOCK * pass_index as u64, &block);
        }
        // Vanilla's pass order (blur.json): In=main Out=swap (H), In=swap Out=main (V),
        // three rounds; the last writes back into the target instead of a texture.
        for pass_index in 0..BLUR_PASSES {
            let into_second = pass_index % 2 == 0;
            let output_view = match pass_index {
                _ if pass_index == BLUR_PASSES - 1 => target_view,
                _ if into_second => &self.views[1],
                _ => &self.views[0],
            };
            let input_group = &self.groups[pass_index];
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blur"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: output_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rpass.set_pipeline(&self.pipeline);
            rpass.set_bind_group(0, input_group, &[]);
            rpass.draw(0..3, 0..1);
        }
    }
}

/// The radius the blur runs with: the option value as-is (vanilla's shader takes
/// `MenuBlurRadius` — the option — when the pass uniform is 0, which the JSON always is).
pub fn blur_radius(option: u8) -> u32 {
    u32::from(option.min(10))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radius_maps_the_option_directly() {
        // Vanilla: Screen.extractBlurredBackground only blurs from 1 up; the shader's radius
        // is the option (default 5, IntRange 0..=10).
        assert_eq!(blur_radius(0), 0);
        assert_eq!(blur_radius(1), 1);
        assert_eq!(blur_radius(5), 5);
        assert_eq!(blur_radius(10), 10);
    }

    #[test]
    fn six_passes_three_rounds() {
        // blur.json: 6 passes, alternating H/V (3 separable rounds).
        assert_eq!(BLUR_PASSES, 6);
    }
}
