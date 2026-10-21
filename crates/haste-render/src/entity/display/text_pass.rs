//! The world text passes text displays draw through: vanilla's `TEXT_POLYGON_OFFSET`
//! (translucent, depth tested and written, pulled toward the camera by a depth bias) and
//! `TEXT_SEE_THROUGH` (translucent, no depth test or write, no light texture, no fog). Both
//! share the entity vertex format, atlas and shader; each is one draw per frame.

use super::*;

/// One text pipeline's vertices this frame and their reused GPU buffer.
struct TextList {
    vertices: Vec<EntityVertex>,
    buffer: wgpu::Buffer,
    capacity: usize,
}

impl TextList {
    fn new(device: &wgpu::Device) -> TextList {
        let capacity = 1024;
        TextList {
            vertices: Vec::new(),
            buffer: make_vertex_buffer(device, capacity),
            capacity,
        }
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.vertices.len() > self.capacity {
            self.capacity = self.vertices.len().next_power_of_two();
            self.buffer = make_vertex_buffer(device, self.capacity);
        }
        if !self.vertices.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.vertices));
        }
    }
}

/// The two text pipelines and their vertex lists.
pub(crate) struct TextPass {
    polygon_offset_pipeline: wgpu::RenderPipeline,
    see_through_pipeline: wgpu::RenderPipeline,
    polygon_offset: TextList,
    see_through: TextList,
}

impl TextPass {
    /// The pipelines over the entity renderer's layout (camera, atlas, light texture) and
    /// shader (`vs_main` lit and fogged for the polygon-offset pass, `vs_unlit` for the
    /// see-through one).
    pub(crate) fn new(
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        shader: &wgpu::ShaderModule,
        target_format: wgpu::TextureFormat,
    ) -> TextPass {
        let pipeline = |label, vertex_entry, depth_stencil| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some(vertex_entry),
                    compilation_options: Default::default(),
                    buffers: &[Some(EntityVertex::LAYOUT)],
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                // Text pipelines keep vanilla's default back-face culling: a text display is
                // seen from its front only.
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(depth_stencil),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        // `DepthStencilState(GREATER_THAN_OR_EQUAL, true, 1, 10)` in vanilla's reverse-Z is
        // LessEqual against Haste's depth cleared to 1, the bias negated to pull toward the
        // camera the same way.
        let polygon_offset = wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth24Plus,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: wgpu::DepthBiasState {
                constant: -10,
                slope_scale: -1.0,
                clamp: 0.0,
            },
        };
        // No depth state at all in vanilla: always passes, writes nothing.
        let see_through = wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth24Plus,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: Default::default(),
            bias: Default::default(),
        };
        TextPass {
            polygon_offset_pipeline: pipeline("text-polygon-offset", "vs_main", polygon_offset),
            see_through_pipeline: pipeline("text-see-through", "vs_unlit", see_through),
            polygon_offset: TextList::new(device),
            see_through: TextList::new(device),
        }
    }

    fn list(&mut self, see_through: bool) -> &mut TextList {
        if see_through {
            &mut self.see_through
        } else {
            &mut self.polygon_offset
        }
    }

    /// Empties both lists for a new frame (capacity kept).
    pub(crate) fn clear(&mut self) {
        self.polygon_offset.vertices.clear();
        self.see_through.vertices.clear();
    }

    /// Borrows one list's vertices out to append to (give them back with [`Self::put`]).
    pub(crate) fn take(&mut self, see_through: bool) -> Vec<EntityVertex> {
        std::mem::take(&mut self.list(see_through).vertices)
    }

    pub(crate) fn put(&mut self, see_through: bool, vertices: Vec<EntityVertex>) {
        self.list(see_through).vertices = vertices;
    }

    pub(crate) fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.polygon_offset.upload(device, queue);
        self.see_through.upload(device, queue);
    }

    /// The polygon-offset text, then the see-through text over everything (vanilla's
    /// see-through text features come after the rest); returns the draw calls made.
    pub(crate) fn draw<'p>(
        &'p self,
        pass: &mut wgpu::RenderPass<'p>,
        atlas: &'p wgpu::BindGroup,
    ) -> u32 {
        let mut calls = 0;
        for (pipeline, list) in [
            (&self.polygon_offset_pipeline, &self.polygon_offset),
            (&self.see_through_pipeline, &self.see_through),
        ] {
            if list.vertices.is_empty() {
                continue;
            }
            pass.set_pipeline(pipeline);
            pass.set_bind_group(1, atlas, &[]);
            pass.set_vertex_buffer(0, list.buffer.slice(..));
            pass.draw(0..list.vertices.len() as u32, 0..1);
            calls += 1;
        }
        calls
    }
}
