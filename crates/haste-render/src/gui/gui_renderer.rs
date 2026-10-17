//! The GUI renderer's setup, per-frame preparation (command list in, one vertex buffer up)
//! and draw passes (vanilla `GuiRenderer`).

use std::sync::Arc;

use haste_assets::{Assets, BitmapFont};
use haste_core::gui::{GuiBlend, GuiFrame};
use haste_core::Identifier;

use super::*;

impl GuiRenderer {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        target_format: wgpu::TextureFormat,
        font: Arc<BitmapFont>,
    ) -> GuiRenderer {
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gui-atlas"),
            size: wgpu::Extent3d {
                width: GUI_ATLAS_SIZE,
                height: GUI_ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let atlas_view = atlas.create_view(&Default::default());
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gui-uniforms"),
            size: std::mem::size_of::<GuiUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gui-layout"),
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
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gui-bind"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gui"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../gui.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gui-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        // One pipeline per blend mode: vanilla's GUI pipelines (RenderPipelines.GUI_TEXTURED and
        // friends blend normally; VIGNETTE multiplies; CROSSHAIR inverts the destination;
        // GUI_TEXT_HIGHLIGHT adds to it). Each has its own fragment entry point so only the
        // normal blend converts to linear on an sRGB-encoded target (`gui_color` in gui.wgsl).
        let make_pipeline = |label, blend: Option<wgpu::BlendState>, fs: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(GuiVertex::LAYOUT)],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        // The vignette texture is an opaque grayscale radial gradient (black centre, bright
        // edges), so its blend multiplies the destination by it: unchanged in the centre,
        // darkened towards the edges.
        let pipelines = [
            make_pipeline("gui-normal", Some(blend_state(GuiBlend::Normal)), "fs_main"),
            make_pipeline(
                "gui-multiply",
                Some(blend_state(GuiBlend::Multiply)),
                "fs_main_multiply",
            ),
            make_pipeline(
                "gui-invert",
                Some(blend_state(GuiBlend::Invert)),
                "fs_main_invert",
            ),
            make_pipeline(
                "gui-additive",
                Some(blend_state(GuiBlend::Additive)),
                "fs_main_additive",
            ),
        ];
        let vertex_capacity = 16 * 1024;
        let vertex_buffer = Self::make_vertex_buffer(&device, vertex_capacity);
        // An sRGB target (no non-sRGB view on this backend) forces the shader to emit linear
        // colour; warn with the concrete deviation that causes for translucent GUI pixels.
        let linear_output = gui_writes_linear(target_format);
        if linear_output {
            let src = [128u8, 128, 128, 140];
            let dst = [200u8, 80, 40];
            let vanilla = blend_gui_pixel(src, dst);
            let fallback = blend_gui_pixel_linear(src, dst);
            tracing::warn!(
                "no non-sRGB view of the GUI target {:?}: translucent GUI pixels blend in linear space \
                 (a 55% grey over rgb({},{},{}) lands on {:?} instead of vanilla's {:?})",
                target_format,
                dst[0],
                dst[1],
                dst[2],
                fallback,
                vanilla
            );
        }
        let mut gui = GuiRenderer {
            device,
            queue,
            pipelines,
            bind_group,
            uniforms,
            atlas,
            vertex_buffer,
            vertex_capacity,
            keys: HashMap::new(),
            entries: Vec::new(),
            shelf: Shelf::default(),
            atlas_full: false,
            font,
            item_names: Vec::new(),
            items: HashMap::new(),
            vertices: Vec::new(),
            batches: Vec::new(),
            entity_scratch: Vec::new(),
            rng: 0x9E37_79B9,
            linear_output,
            animation_tick: 0,
            stats: GuiStats::default(),
        };
        // The missing texture is entry 0 so lookups can always fall back to it.
        gui.insert_entry(
            TexKey::Missing,
            Arc::new(missing_image()),
            SpriteMeta::Stretch,
            None,
        );
        gui
    }

    /// The game tick the animated GUI textures (fire, nether portal) show. The next `prepare`
    /// re-uploads every changed frame.
    pub fn set_animation_tick(&mut self, tick: u64) {
        self.animation_tick = tick;
    }

    pub(super) fn make_vertex_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gui-vertices"),
            size: (capacity * std::mem::size_of::<GuiVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn font(&self) -> &Arc<BitmapFont> {
        &self.font
    }

    /// Item registry names by protocol id (`minecraft:item`), for `GuiCommand::Item`.
    pub fn set_item_names(&mut self, names: Vec<Identifier>) {
        self.item_names = names;
        self.items.clear();
    }

    // ---- textures ----

    /// Turns the frame into vertices and batches and uploads them. `target` is the framebuffer
    /// size in pixels.
    pub fn prepare(
        &mut self,
        frame: &GuiFrame,
        entities: &[haste_core::model::EntityInstance],
        nametags: &[ScreenNametag],
        target: (u32, u32),
        assets: &mut Assets,
    ) {
        self.prepare_range(
            frame,
            entities,
            nametags,
            target,
            assets,
            0,
            frame.commands.len(),
            true,
        );
    }

    /// [`Self::prepare`] over `commands[from..to]`, either resetting the buffers or appending
    /// to what an earlier call this frame built (the screen's blurred background splits the GUI
    /// in two: the part before the blur marker draws, the blur runs, then this appends the
    /// rest). Name tags belong to the first (before-blur) part only.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_range(
        &mut self,
        frame: &GuiFrame,
        entities: &[haste_core::model::EntityInstance],
        nametags: &[ScreenNametag],
        target: (u32, u32),
        assets: &mut Assets,
        from: usize,
        to: usize,
        reset: bool,
    ) {
        if reset {
            self.stats = GuiStats::default();
        }
        if self.entries[0].rect.is_none() {
            self.resident(0);
        }
        for attempt in 0..2 {
            self.build(frame, entities, nametags, target, assets, from, to, reset);
            if !self.atlas_full {
                break;
            }
            // The atlas filled up mid-frame: repack with only what this frame needs.
            self.stats.atlas_resets += 1;
            self.reset_atlas();
            if attempt == 1 {
                tracing::warn!("GUI atlas too small for one frame's textures");
            }
        }
        // Animated textures (fire, nether portal): bring the resident frames up to date.
        self.animate();
        let scale = frame.scale.max(1) as f32;
        let uniforms = GuiUniforms {
            size: [target.0 as f32 / scale, target.1 as f32 / scale],
            flags: [if self.linear_output { 1.0 } else { 0.0 }, 0.0],
        };
        self.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        if self.vertices.len() > self.vertex_capacity {
            self.vertex_capacity = self.vertices.len().next_power_of_two();
            self.vertex_buffer = Self::make_vertex_buffer(&self.device, self.vertex_capacity);
        }
        if !self.vertices.is_empty() {
            self.queue
                .write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.vertices));
        }
        self.stats.commands = frame.commands.len() as u32;
        self.stats.vertices = self.vertices.len() as u32;
        self.stats.draw_calls = self.batches.len() as u32;
    }

    /// Records the prepared frame into a render pass on the target.
    pub fn draw<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>, target: (u32, u32)) {
        self.draw_from(pass, target, 0);
    }

    /// [`Self::draw`] over the batches from `first` on: after the blur ran (vanilla draws
    /// only the after-blur strata then — the before-blur ones are already on the target,
    /// under the blur), the second half must not re-draw them sharp.
    pub fn draw_from<'p>(
        &'p self,
        pass: &mut wgpu::RenderPass<'p>,
        target: (u32, u32),
        first: usize,
    ) {
        if self.batches.is_empty() {
            return;
        }
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        for b in &self.batches[first.min(self.batches.len())..] {
            pass.set_pipeline(&self.pipelines[b.blend as usize]);
            match b.scissor {
                Some([x, y, w, h]) => pass.set_scissor_rect(x, y, w, h),
                None => pass.set_scissor_rect(0, 0, target.0, target.1),
            }
            pass.draw(b.start..b.end, 0..1);
        }
    }

    /// The number of batches built so far this frame (the blur splits the frame: batches
    /// built before the blur must not be drawn again after it).
    pub fn batch_count(&self) -> usize {
        self.batches.len()
    }
}

/// Scissor rectangle (GUI pixels, x1 y1 x2 y2) to framebuffer x, y, w, h, clamped.
pub(super) fn to_framebuffer(r: [f32; 4], scale: f32, target: (u32, u32)) -> [u32; 4] {
    let x1 = (r[0] * scale).round().clamp(0.0, target.0 as f32) as u32;
    let y1 = (r[1] * scale).round().clamp(0.0, target.1 as f32) as u32;
    let x2 = (r[2] * scale).round().clamp(0.0, target.0 as f32) as u32;
    let y2 = (r[3] * scale).round().clamp(0.0, target.1 as f32) as u32;
    [x1, y1, x2.saturating_sub(x1), y2.saturating_sub(y1)]
}
