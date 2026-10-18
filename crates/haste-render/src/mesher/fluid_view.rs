//! The mesh snapshot as the fluid mesher sees it: the `FluidWorld` adapter the section
//! mesher hands to `crate::fluid::fluid_quads`, answering from the captured snapshot and
//! the baked models. Pure glue, split out of `mesher.rs` for file size.

use haste_core::{BlockPos, BlockStateId};

use crate::fluid_occlusion::Occluder;

use super::MeshSnapshot;

pub(super) struct FluidView<'a> {
    pub(super) snapshot: &'a MeshSnapshot,
    pub(super) baked: &'a haste_assets::BakedBlockModels,
    /// Per-state occlusion shapes (`fluid/occlusion.rs`).
    pub(super) occluders: &'a crate::fluid::FluidOccluders,
}

impl crate::fluid::FluidWorld for FluidView<'_> {
    fn state(&self, p: BlockPos) -> BlockStateId {
        self.snapshot.block_state(p)
    }
    fn opaque(&self, p: BlockPos) -> bool {
        self.baked.is_full_opaque(self.snapshot.block_state(p))
    }
    fn solid(&self, p: BlockPos) -> bool {
        crate::fluid::baked_solid(self.baked, self.snapshot.block_state(p))
    }
    fn face_occluder(&self, p: BlockPos, _dir: haste_assets::Direction) -> Occluder<'_> {
        let state = self.snapshot.block_state(p);
        self.occluders
            .occluder(state, || self.baked.is_full_opaque(state))
    }
}
