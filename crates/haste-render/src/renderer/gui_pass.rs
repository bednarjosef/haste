//! The GUI pass of a frame (vanilla `GuiRenderer.render`): the frame's commands over the
//! world, with the open screen's blurred background between them.
use super::{FrameInput, Renderer};

impl Renderer {
    /// GUI on top, in its own pass over the non-sRGB view (no depth). When the open screen
    /// asks for a blurred background (vanilla: Screen.extractBlurredBackground marking the
    /// strata split), the commands before the marker draw, the blur post chain runs over
    /// the target (GameRenderer.processBlurEffect), then the rest draws over the result.
    ///
    /// Returns the pass's draw calls, `None` when the frame has no GUI to draw. `target` is the
    /// presented surface texture of this frame, `capture_color` the capture's own target.
    pub(super) fn encode_gui(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &FrameInput,
        gui_view: &wgpu::TextureView,
        target: Option<&wgpu::SurfaceTexture>,
        capture_color: Option<&wgpu::Texture>,
    ) -> Option<u32> {
        if frame.gui.commands.is_empty() && self.nametags.is_empty() {
            return None;
        }
        let mut draw_calls = 0u32;
        let blur_after = frame.gui.blur_after as usize;
        let blur = frame.gui.blur_after >= 0
            && blur_after < frame.gui.commands.len()
            && self.menu_blur_radius > 0;
        let split = if blur {
            blur_after
        } else {
            frame.gui.commands.len()
        };
        self.gui.prepare_range(
            frame.gui,
            frame.entities,
            &self.nametags,
            self.size,
            &mut self.assets,
            0,
            split,
            true,
        );
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: gui_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: self.timer.as_ref().and_then(|t| t.writes(2)),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.gui.draw(&mut pass, self.size);
        }
        let first_batch_after_blur = self.gui.batch_count();
        draw_calls += self.gui.stats.draw_calls;
        if blur {
            // The blur's input is the frame's colour target itself: copy it in (the
            // surface's texture, or the offscreen/capture target), run the chain, the
            // last pass lands back in the target the rest of the GUI draws over.
            let copy_src = match (target, capture_color, &self.offscreen) {
                (Some(tex), _, _) => Some(tex.texture.as_image_copy()),
                (None, Some(color), _) => Some(color.as_image_copy()),
                (None, None, Some(off)) => Some(off.color.as_image_copy()),
                (None, None, None) => None,
            };
            // A windowed surface without COPY_SRC cannot feed the blur; leave the world
            // sharp rather than dropping the screen (logged once). Vulkan has COPY_SRC.
            let can_blur = copy_src.is_some()
                && (self.offscreen.is_some() || capture_color.is_some() || self.surface_copy);
            if can_blur {
                let copy_src = copy_src.unwrap();
                self.blur.resize(
                    &self.gpu.device,
                    Self::gui_format_for(&self.gpu, self.surface_format),
                    self.size,
                );
                self.blur.run(
                    encoder,
                    &self.gpu.queue,
                    self.menu_blur_radius,
                    gui_view,
                    copy_src,
                );
                // The rest of the frame's commands, over the blurred target.
                self.gui.prepare_range(
                    frame.gui,
                    frame.entities,
                    &self.nametags,
                    self.size,
                    &mut self.assets,
                    blur_after,
                    frame.gui.commands.len(),
                    false,
                );
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("gui-after-blur"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: gui_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                // Only the batches built after the blur: the before-blur ones (HUD
                // included, like vanilla's firstStratumAfterBlur split) are already
                // on the target underneath it and must not be redrawn sharp.
                self.gui
                    .draw_from(&mut pass, self.size, first_batch_after_blur);
                draw_calls += self.gui.stats.draw_calls - first_batch_after_blur as u32;
            } else if !self.blur_unavailable_logged {
                self.blur_unavailable_logged = true;
                tracing::warn!("surface cannot be copied: menu background blur unavailable");
            }
        }
        Some(draw_calls)
    }
}
