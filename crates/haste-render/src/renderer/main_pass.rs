// The world's main render pass (vanilla `LevelRenderer.renderLevel` into the main
// target): its attachments, recorded as one render pass, or as two around the GPU
// occlusion culling's compute work (`gpu_cull.rs`).

use super::Renderer;

impl Renderer {
    /// Begins (a part of) the main pass. The first part clears the target to the fog
    /// colour and the depth to far; a later part loads both. The pass timestamps go to
    /// the first part's beginning and the last part's end.
    pub(super) fn begin_main_pass<'e>(
        &self,
        encoder: &'e mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        clear: Option<[f32; 3]>,
        last: bool,
    ) -> wgpu::RenderPass<'e> {
        let first = clear.is_some();
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("main"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    // The clear is the fog colour's sRGB-encoded bytes: the view is
                    // non-sRGB, so no hardware conversion happens and the raw value is
                    // what lands in the target (an sRGB view would have encoded the
                    // linear colour itself).
                    load: match clear {
                        Some(fog) => wgpu::LoadOp::Clear(wgpu::Color {
                            r: fog[0] as f64,
                            g: fog[1] as f64,
                            b: fog[2] as f64,
                            a: 1.0,
                        }),
                        None => wgpu::LoadOp::Load,
                    },
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: if first {
                        wgpu::LoadOp::Clear(1.0)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: self
                .timer
                .as_ref()
                .and_then(|t| t.writes_part(0, first, last)),
            occlusion_query_set: None,
            multiview_mask: None,
        })
    }

    /// Binds the camera, atlas and lightmap groups every world draw expects.
    pub(super) fn bind_world_groups(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        pass.set_bind_group(1, &self.atlas_bind_group, &[]);
        pass.set_bind_group(2, &self.lightmap_bind_group, &[]);
    }
}
