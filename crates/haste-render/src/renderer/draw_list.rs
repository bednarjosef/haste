// The terrain draw list (`render_world.rs`'s cull step, split out by the
// file-size rule): frustum-cull the visible sections into per-layer draw
// lists in the occlusion walk's visit order, group the opaque layers by
// arena, sort translucent back to front, and write the per-draw data (see
// `quad_draw.rs`). Vanilla's equivalent is `LevelRenderer` compiling its
// visible sections into per-layer render lists.

use haste_core::SectionPos;

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
        let min_section = self.min_section;
        let sections = &self.sections;
        let emit = |buckets: &mut [Vec<SectionDraw>; 4], pos: SectionPos| {
            let Some(s) = sections.get(pos) else {
                return;
            };
            let si = s.pos.y - min_section;
            if !(0..32).contains(&si) {
                return;
            }
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
                return;
            }
            let d = glam::Vec3::new(
                (lo[0] + hi[0]) * 0.5,
                (lo[1] + hi[1]) * 0.5,
                (lo[2] + hi[2]) * 0.5,
            )
            .length_squared();
            for l in sections.layers(s) {
                // Skip a direction group facing away from the camera.
                if l.dir < 6 && l.back_facing(lo, hi) {
                    continue;
                }
                // Sort key: ascending distance for opaque (early depth),
                // negated for translucent so ascending = far first.
                let translucent = l.layer == haste_assets::RenderLayer::Translucent;
                let i = match l.layer {
                    haste_assets::RenderLayer::Solid => 0,
                    haste_assets::RenderLayer::Cutout => 1,
                    haste_assets::RenderLayer::CutoutMipped => 2,
                    haste_assets::RenderLayer::Translucent => 3,
                };
                buckets[i].push(SectionDraw {
                    layer: l.layer,
                    arena: s.arena,
                    key: if translucent { -d } else { d },
                    first_index: l.first,
                    index_count: l.count,
                    base_vertex: s.base_vertex,
                    kind: l.kind,
                    origin: [s.pos.x << 4, s.pos.y << 4, s.pos.z << 4],
                });
            }
        };
        self.for_each_visible_section(|pos| emit(&mut buckets, pos));
        // Group the opaque buckets by arena: the draw loop issues one multi-draw
        // call per run sharing a layer and an arena, and the visit order
        // interleaves the arenas, so ungrouped the frame pays thousands of draw
        // dispatches (2232 measured at rd 32 over 16 arenas). Depth-tested
        // layers do not depend on order (terrain faces never coincide).
        for b in buckets.iter_mut().take(3) {
            b.sort_unstable_by_key(|d| d.arena);
        }
        // Translucent back to front: the key is the negated distance, so
        // ascending key order is far first.
        if buckets[3].len() > 1 {
            buckets[3].sort_unstable_by(|a, b| a.key.total_cmp(&b.key));
        }
        let mut draws = std::mem::take(&mut self.draw_list);
        draws.clear();
        for b in buckets.iter() {
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
        (sections_total, drawn, indices_drawn)
    }
}
