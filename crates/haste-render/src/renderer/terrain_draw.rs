// The terrain layers of the main pass (vanilla `LevelRenderer.renderLevel` drawing the
// chunk layers through `ChunkSectionsToRender`), split out of `render_world.rs`: the
// per-layer, per-arena multi-draws over the frame's draw list, with the entities drawn
// after the opaque layers and before translucent.

use super::{Renderer, SectionDraw};

impl Renderer {
    /// Draws `draws` (layer-major, see `draw_list.rs`) into `pass`, the entities included
    /// before the first translucent draw (or after the last draw without translucent).
    /// Returns the draw calls issued.
    pub(super) fn draw_terrain<'p>(
        &'p self,
        pass: &mut wgpu::RenderPass<'p>,
        draws: &[SectionDraw],
        brackets: &mut crate::timing::SubpassBrackets,
    ) -> u32 {
        let timer = self.timer.as_ref();
        let mut draw_calls = 0u32;
        // Layered draw order (RenderLayer Ord: Solid < Cutout < CutoutMipped
        // < Translucent). Section origins come from the instance-rate origin buffer, one
        // instance per draw; buffers are bound once per arena. In the quad format
        // the instance buffer also carries each draw's span kind, the arena's
        // record storage buffers are bound as group 3, and the static index
        // pattern replaces the arena's index buffer — the draw's base_vertex
        // carries the record base.
        let quad = crate::quad_format::quad_format_enabled();
        if quad {
            pass.set_vertex_buffer(0, self.quad_instance_buffer.slice(..));
            pass.set_index_buffer(self.quad_pattern_index.slice(..), wgpu::IndexFormat::Uint32);
        } else {
            pass.set_vertex_buffer(1, self.origin_buffer.slice(..));
        }
        let mut entities_drawn = false;
        let mut bound_arena = u32::MAX;
        let mut bound_layer = None;
        let mut i = 0;
        while i < draws.len() {
            let d = &draws[i];
            let instance = i;
            // Entities after the opaque and cutout terrain, before translucent.
            if !entities_drawn && d.layer == haste_assets::RenderLayer::Translucent {
                brackets.begin_entities(pass, timer);
                // Vanilla draws the entity shadows (a translucent, depth-tested pass) with
                // the entities, then the block entities' special passes: the end
                // portal/gateway starfield and the beacon beams (inner opaque, glow
                // translucent without depth write). The special passes use their own bind
                // groups at index 1, so the terrain's must be restored after them (the
                // loop below sets the pipeline but not the bind groups again).
                draw_calls += self.entity.draw_shadows(pass);
                draw_calls += self.entity.draw(pass);
                pass.set_bind_group(1, &self.atlas_bind_group, &[]);
                pass.set_bind_group(2, &self.lightmap_bind_group, &[]);
                draw_calls += self.entity.draw_portals(pass);
                draw_calls += self.entity.draw_beams(pass);
                pass.set_bind_group(1, &self.atlas_bind_group, &[]);
                // Close the entities bracket right after the entity draws, so the
                // translucent terrain below is not timed as "entities".
                brackets.end(pass, timer, crate::timing::ENTITIES);
                entities_drawn = true;
                bound_layer = None;
                bound_arena = u32::MAX;
            }
            if bound_layer != Some(d.layer) {
                // CutoutMipped (leaves, vines) shares the solid *pipeline* (same shader
                // state, mips differ in the sampler) but must keep the alpha discard:
                // its sprites have transparent texels by vanilla's per-sprite layer rule
                // (NativeImage.computeTransparency -> ChunkSectionLayer.byTransparency).
                let (pipeline, sub) = match (d.layer, quad) {
                    (haste_assets::RenderLayer::Solid, false) => (&self.pipeline_solid, 1),
                    (haste_assets::RenderLayer::Cutout, false) => (&self.pipeline_cutout, 2),
                    (haste_assets::RenderLayer::CutoutMipped, false) => (&self.pipeline_cutout, 3),
                    (haste_assets::RenderLayer::Translucent, false) => {
                        (&self.pipeline_translucent, 4)
                    }
                    (haste_assets::RenderLayer::Solid, true) => (&self.quad_pipeline_solid, 1),
                    (haste_assets::RenderLayer::Cutout, true) => (&self.quad_pipeline_cutout, 2),
                    (haste_assets::RenderLayer::CutoutMipped, true) => {
                        (&self.quad_pipeline_cutout, 3)
                    }
                    (haste_assets::RenderLayer::Translucent, true) => {
                        (&self.quad_pipeline_translucent, 4)
                    }
                };
                brackets.begin_terrain_layer(pass, timer, sub);
                pass.set_pipeline(pipeline);
                bound_layer = Some(d.layer);
            }
            if bound_arena != d.arena {
                let a = &self.arenas.arenas[d.arena as usize];
                if quad {
                    if let Some(bind) = &a.quad_bind {
                        pass.set_bind_group(3, bind, &[]);
                    }
                } else {
                    pass.set_vertex_buffer(0, a.vertex.as_ref().unwrap().slice(..));
                    pass.set_vertex_buffer(1, self.origin_buffer.slice(..));
                    pass.set_index_buffer(
                        a.index.as_ref().unwrap().slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                }
                bound_arena = d.arena;
            }
            let (quad_count, quad_base) = if quad {
                d.kind.indexed_draw(d.first_index, d.index_count)
            } else {
                (0, 0)
            };
            if self.gpu.multi_draw {
                // One call for the run of draws sharing this layer and arena.
                let end = draws[i..]
                    .iter()
                    .position(|e| e.layer != d.layer || e.arena != d.arena)
                    .map_or(draws.len(), |n| i + n);
                let stride = std::mem::size_of::<wgpu::util::DrawIndexedIndirectArgs>();
                pass.multi_draw_indexed_indirect(
                    &self.indirect_buffer,
                    (i * stride) as u64,
                    (end - i) as u32,
                );
                draw_calls += 1;
                i = end;
            } else if quad {
                pass.draw_indexed(
                    0..quad_count,
                    quad_base as i32,
                    instance as u32..instance as u32 + 1,
                );
                draw_calls += 1;
                i += 1;
            } else {
                pass.draw_indexed(
                    d.first_index..d.first_index + d.index_count,
                    d.base_vertex as i32,
                    instance as u32..instance as u32 + 1,
                );
                draw_calls += 1;
                i += 1;
            }
        }
        if !entities_drawn {
            brackets.begin_entities(pass, timer);
            draw_calls += self.entity.draw_shadows(pass);
            draw_calls += self.entity.draw(pass);
            pass.set_bind_group(1, &self.atlas_bind_group, &[]);
            pass.set_bind_group(2, &self.lightmap_bind_group, &[]);
            draw_calls += self.entity.draw_portals(pass);
            draw_calls += self.entity.draw_beams(pass);
        }
        draw_calls
    }
}
