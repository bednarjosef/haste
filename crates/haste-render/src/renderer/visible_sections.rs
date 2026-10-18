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
}
