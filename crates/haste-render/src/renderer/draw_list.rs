// The terrain draw list (`render_world.rs`'s cull step, split out by the
// file-size rule): frustum-cull the visible sections (`visible_list.rs`, kept
// grouped by arena in the occlusion walk's visit order) into per-layer draw
// lists, sort translucent back to front, and write the per-draw data (see
// `quad_draw.rs`). Vanilla's equivalent is `LevelRenderer` compiling its
// visible sections into per-layer render lists.

use super::{Renderer, SectionDraw};

impl Renderer {
    /// Fills `self.draw_list` from the visible sections and returns
    /// `(sections_total, draws, indices_drawn)` for the frame stats.
    pub(super) fn collect_draws(
        &mut self,
        planes: &[glam::Vec4; 6],
        cam: glam::DVec3,
    ) -> (u32, u32, u32) {
        // Emit draws in the occlusion walk's visit (BFS) order — vanilla draws
        // its sections in `SectionOcclusionGraph`'s node order
        // (`addSectionsInFrustum` appends as the graph is visited), so the
        // opaque layers need no sort; only translucent is sorted back to front
        // (vanilla reverses the list for that layer). Bonus sections (newly
        // uploaded next to visible ones) follow; they are a few per frame. The
        // per-layer buckets replace the old layer-major sort of the whole list.
        let mut buckets = std::mem::take(&mut self.layer_buckets);
        for b in buckets.iter_mut() {
            b.clear();
        }
        self.refresh_visible_list();
        let list = &self.visible_list;
        for s in list.sections() {
            let lo = [
                (s.aabb.0[0] as f64 - cam.x) as f32,
                (s.aabb.0[1] as f64 - cam.y) as f32,
                (s.aabb.0[2] as f64 - cam.z) as f32,
            ];
            let hi = [
                (s.aabb.1[0] as f64 - cam.x) as f32,
                (s.aabb.1[1] as f64 - cam.y) as f32,
                (s.aabb.1[2] as f64 - cam.z) as f32,
            ];
            if !Self::aabb_visible(planes, lo, hi) {
                continue;
            }
            let d = glam::Vec3::new(
                (lo[0] + hi[0]) * 0.5,
                (lo[1] + hi[1]) * 0.5,
                (lo[2] + hi[2]) * 0.5,
            )
            .length_squared();
            let front =
                super::visible_list::front_positions(crate::section_slab::front_groups(lo, hi));
            for g in list.groups(s) {
                // Sort key: ascending distance for opaque (early depth),
                // negated for translucent so ascending = far first.
                let key = if g.bucket == 3 { -d } else { d };
                let bucket = &mut buckets[g.bucket as usize];
                // The groups facing the camera, joined where their ranges touch (see
                // `mesher::GROUP_ORDER`); a run also joins the previous draw of the
                // bucket when it continues it: fewer sub-draws.
                g.for_each_run(front, |first, count| {
                    if let Some(prev) = bucket.last_mut() {
                        if prev.kind == crate::quad_format::SpanKind::Indexed
                            && g.kind == crate::quad_format::SpanKind::Indexed
                            && prev.arena == s.arena
                            && prev.base_vertex == s.base_vertex
                            && prev.first_index + prev.index_count == first
                        {
                            prev.index_count += count;
                            return;
                        }
                    }
                    bucket.push(SectionDraw {
                        layer: g.layer,
                        arena: s.arena,
                        key,
                        first_index: first,
                        index_count: count,
                        base_vertex: s.base_vertex,
                        kind: g.kind,
                        origin: s.origin,
                        slot: s.slot,
                        lo,
                        hi,
                    });
                });
            }
        }
        // Translucent back to front: the key is the negated distance, so
        // ascending key order is far first. Equal keys order by section, then by
        // range, so a section's facing groups stay together in mesh order (an
        // unstable sort: the stable one allocates its buffer every frame).
        if buckets[3].len() > 1 {
            buckets[3].sort_unstable_by(|a, b| {
                (a.key.total_cmp(&b.key))
                    .then(a.slot.cmp(&b.slot))
                    .then(a.first_index.cmp(&b.first_index))
            });
        }
        let mut draws = std::mem::take(&mut self.draw_list);
        draws.clear();
        // The visible list is grouped by arena (see `visible_list.rs`), so each opaque
        // bucket is already in the draw loop's run order.
        for b in &buckets {
            draws.extend_from_slice(b);
        }
        self.layer_buckets = buckets;
        let sections_total = self.sections.len() as u32;
        let drawn = draws.len() as u32;
        // The vertex-fetch load: indices in the vertex format, the same
        // triangles through the static pattern in the quad format.
        let indices_drawn: u32 = if crate::quad_format::quad_format_enabled() {
            draws
                .iter()
                .map(|d| d.kind.indexed_draw(d.first_index, d.index_count).0)
                .sum()
        } else {
            draws.iter().map(|d| d.index_count).sum()
        };
        self.prepare_draw_data(&draws);
        self.gpu_cull.prepare(
            &self.gpu.device,
            &self.gpu.queue,
            &draws,
            self.sections.slot_capacity(),
        );
        self.draw_list = draws;
        (sections_total, drawn, indices_drawn)
    }
}
