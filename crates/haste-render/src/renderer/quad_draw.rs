// Per-frame terrain draw data (`render_world.rs`'s cull fills the draw list;
// this prepares what the draw loop binds): the section origins written into
// the instance-rate buffer (vertex format), the per-draw span kinds (quad
// format; the record base rides each draw's base_vertex), and the indirect
// multi-draw arguments — one five-u32 record per draw in both formats, so
// the draw loop issues the same `multi_draw_indexed_indirect`.

use crate::quad_format::{quad_format_enabled, QuadInstance};

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
        // One DrawIndexedIndirectArgs per draw: index count, instance count,
        // first index, base vertex, first instance. In the quad format the
        // static pattern or sequential buffer is bound, so first index is 0
        // and base vertex is the span's record base (four records per quad
        // before it, so the shader's vertex_index >> 2 is the record).
        self.indirect.clear();
        for (i, d) in draws.iter().enumerate() {
            let (count, first, base) = if quad_format_enabled() {
                let (count, base) = d.kind.indexed_draw(d.first_index, d.index_count);
                (count, 0, base)
            } else {
                (d.index_count, d.first_index, d.base_vertex)
            };
            self.indirect
                .extend_from_slice(&[count, 1, first, base, i as u32]);
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
