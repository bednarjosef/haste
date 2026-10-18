//! The sections a frame draws, in draw order: vanilla `LevelRenderer.visibleSections`, filled
//! from the occlusion walk (`SectionOcclusionGraph.addSectionsInFrustum`) or, with `smartCull`
//! off, from every loaded section.

use haste_core::SectionPos;

use super::Renderer;

impl Renderer {
    /// Calls `f` with each section to draw this frame. With the cull on that is the occlusion
    /// walk's visit (BFS) order, then the bonus sections (newly uploaded next to visible ones);
    /// with `smartCull` off (a spectator camera inside a solid block) every section draws, and
    /// the caller's frustum test is the only cull.
    pub(super) fn for_each_visible_section(&self, mut f: impl FnMut(SectionPos)) {
        if self.cull_sections {
            for (c, si) in self.visible_mask.ordered() {
                f(SectionPos::new(c.x, si + self.min_section, c.z));
            }
            for &pos in &self.bonus_list {
                if self.sections.get(pos).is_some_and(|s| s.bonus) {
                    f(pos);
                }
            }
        } else {
            for s in self.sections.iter() {
                f(s.pos);
            }
        }
    }

    /// Collects a finished occlusion walk and schedules the next one: vanilla's
    /// `SectionOcclusionGraph.update`.
    pub(super) fn update_occlusion_walk(&mut self, cam: glam::DVec3) {
        // Cave culling: the walk runs on a worker thread (vanilla's
        // SectionOcclusionGraph full update runs on a background executor).
        // Collect a finished mask first, then schedule a new walk when the
        // camera moved or new sightline data arrived and none is in flight.
        // Until the new mask arrives the previous one is drawn, plus newly
        // uploaded sections next to visible ones (`bonus`).
        if let Some(mask) = self.walk.poll() {
            self.visible_mask = mask;
            self.visible_for = self.scheduled_for.take();
            self.walk_in_flight = false;
            self.clear_bonus();
            self.want_version += 1;
        }
        let cam_section = SectionPos::new(
            (cam.x.floor() as i32) >> 4,
            (cam.y.floor() as i32) >> 4,
            (cam.z.floor() as i32) >> 4,
        );
        let need_walk =
            self.visible_for != Some((cam_section, self.view_distance)) || self.vis_dirty;
        if need_walk && !self.walk_in_flight {
            self.walk.schedule(
                cam_section,
                self.view_distance,
                self.section_count,
                self.min_section,
                cam,
            );
            self.scheduled_for = Some((cam_section, self.view_distance));
            self.walk_in_flight = true;
            self.vis_dirty = false;
        }
    }
}
