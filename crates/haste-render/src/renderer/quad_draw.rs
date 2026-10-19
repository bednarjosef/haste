// Per-frame terrain draw data (`render_world.rs`'s cull fills the draw list;
// this prepares what the draw loop binds): the section origins written into
// the instance-rate buffer (vertex format), the per-draw record bases and
// span kinds (quad format), and the indirect multi-draw arguments for both.

use crate::quad_format::{quad_format_enabled, DrawIndirect, QuadInstance};

use super::{Renderer, SectionDraw};

impl Renderer {
    /// Writes this frame's per-draw data (origins or quad instances) and the
    /// indirect arguments for `draws`, growing the reused buffers when the
    /// list outgrew them.
    pub(super) fn prepare_draw_data(&mut self, draws: &[SectionDraw]) {
        if quad_format_enabled() {
            self.quad_instances.clear();
            self.quad_instances
                .extend(draws.iter().map(|d| QuadInstance {
                    origin: d.origin,
                    first: d.first_index,
                    kind: QuadInstance::kind_of(d.kind),
                }));
            if self.quad_instances.len() > self.quad_instance_capacity {
                self.quad_instance_capacity = self.quad_instances.len().next_power_of_two();
                self.quad_instance_buffer =
                    Self::make_quad_instance_buffer(&self.gpu.device, self.quad_instance_capacity);
            }
            if !self.quad_instances.is_empty() {
                self.gpu.queue.write_buffer(
                    &self.quad_instance_buffer,
                    0,
                    bytemuck::cast_slice(&self.quad_instances),
                );
            }
        } else {
            self.origins.clear();
            self.origins.extend(draws.iter().map(|d| d.origin));
            if self.origins.len() > self.origin_capacity {
                self.origin_capacity = self.origins.len().next_power_of_two();
                self.origin_buffer =
                    Self::make_origin_buffer(&self.gpu.device, self.origin_capacity);
            }
            if !self.origins.is_empty() {
                self.gpu.queue.write_buffer(
                    &self.origin_buffer,
                    0,
                    bytemuck::cast_slice(&self.origins),
                );
            }
        }
        if !self.gpu.multi_draw {
            return;
        }
        self.indirect.clear();
        if quad_format_enabled() {
            for (i, d) in draws.iter().enumerate() {
                let args = DrawIndirect {
                    vertex_count: d.kind.vertex_count(d.index_count),
                    instance_count: 1,
                    first_vertex: 0,
                    first_instance: i as u32,
                };
                self.indirect
                    .extend_from_slice(bytemuck::cast_slice(&[args]));
            }
        } else {
            for (i, d) in draws.iter().enumerate() {
                self.indirect.extend_from_slice(&[
                    d.index_count,
                    1,
                    d.first_index,
                    d.base_vertex,
                    i as u32,
                ]);
            }
        }
        if draws.len() > self.indirect_capacity {
            self.indirect_capacity = draws.len().next_power_of_two();
            self.indirect_buffer =
                Self::make_indirect_buffer(&self.gpu.device, self.indirect_capacity);
        }
        if !self.indirect.is_empty() {
            self.gpu.queue.write_buffer(
                &self.indirect_buffer,
                0,
                bytemuck::cast_slice(&self.indirect),
            );
        }
    }
}
