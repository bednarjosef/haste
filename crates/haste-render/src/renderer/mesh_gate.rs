//! The mesh gate: which dirty sections may be submitted for meshing. Vanilla's
//! `LevelExtractor` compiles a section only when it is in `visibleSections`, which
//! `SectionOcclusionGraph.addSectionsInFrustum` collects with a frustum widened by the camera
//! cube.

use haste_core::{ChunkPos, SectionPos};

use super::{Camera, Renderer};

impl Renderer {
    /// Records this frame's frustum for the mesh gate and bumps `want_version` when the look
    /// crosses a 2-degree step, then feeds the pre-mesh turn prediction (`update_premesh`)
    /// with the same camera.
    pub(super) fn update_mesh_gate(
        &mut self,
        planes: [glam::Vec4; 6],
        camera: &Camera,
        aspect: f32,
    ) {
        // The mesh gate's frustum (`wants_section` reads it before the next frame's
        // feed): the section cubes are tested expanded by the camera cube, like
        // vanilla's `Frustum.offsetToFullyIncludeCameraCube(8)` in
        // `SectionOcclusionGraph.addSectionsInFrustum`. A look crossing a 2-degree step
        // bumps `want_version` (vanilla rebuilds `visibleSections` then: the mesh gate's
        // held-back sections are re-checked).
        self.want_frustum = Some((planes, camera.pos));
        let look_step = (
            (camera.yaw / 2.0).floor() as i32,
            (camera.pitch / 2.0).floor() as i32,
        );
        if Some(look_step) != self.want_look {
            self.want_look = Some(look_step);
            self.want_version += 1;
        }
        self.update_premesh(camera, aspect);
    }

    /// Whether a dirty section may be submitted for meshing this frame: vanilla's
    /// `LevelExtractor` compiles a section only when it is in `visibleSections` —
    /// the occlusion walk's set (plus the bonus list, and everything when smart
    /// culling is off) intersected with the frustum. Held-back sections keep their
    /// dirty state and mesh when they become visible; one that was meshed before
    /// keeps its stale mesh until then, as vanilla's does.
    pub fn wants_section(&self, pos: SectionPos) -> bool {
        if self.cull_sections {
            let si = pos.y - self.min_section;
            let in_mask = (0..32).contains(&si)
                && self.visible_mask.column(ChunkPos::new(pos.x, pos.z)) & (1u32 << si) != 0;
            let in_bonus = self.sections.get(pos).is_some_and(|s| s.bonus);
            if !in_mask && !in_bonus {
                return false;
            }
        }
        let Some((planes, cam)) = &self.want_frustum else {
            // No frame has rendered yet: mesh everything until a frustum exists.
            return true;
        };
        // The section cube expanded by the camera cube: vanilla widens the frustum
        // by `offsetToFullyIncludeCameraCube(8)` when it collects `visibleSections`,
        // so terrain just outside the view meshes too and turning pops less.
        let lo = [
            ((pos.x << 4) as f64 - cam.x - 8.0) as f32,
            ((pos.y << 4) as f64 - cam.y - 8.0) as f32,
            ((pos.z << 4) as f64 - cam.z - 8.0) as f32,
        ];
        let hi = [
            ((pos.x << 4) as f64 - cam.x + 24.0) as f32,
            ((pos.y << 4) as f64 - cam.y + 24.0) as f32,
            ((pos.z << 4) as f64 - cam.z + 24.0) as f32,
        ];
        Self::aabb_visible(planes, lo, hi)
    }
}
