//! Water and lava meshing, ported from the client's `FluidRenderer.tesselate` and its
//! helpers (26.3), plus `FlowingFluid.getFlow` for the surface flow:
//!
//! - Fluid per block state: `water`/`lava` with `level` 0 (source, height 8/9), 1..7 (flowing,
//!   height (8 - level)/9), 8..15 (falling, height 8/9); `waterlogged=true` states and the
//!   always-underwater plants (`kelp`, `kelp_plant`, `seagrass`, `tall_seagrass`,
//!   `bubble_column`) hold a water source.
//! - Corner heights: `FluidRenderer.getHeight` (1.0 under a same-fluid column, own height, 0
//!   for open non-fluid blocks, ignored for solid ones) and
//!   `FluidRenderer.calculateAverageHeight` (columns of height >= 0.8 weigh 10, lower ones 1).
//!   APPROXIMATE: vanilla's blocks-motion flag is the mesher's `baked_solid` (full opaque
//!   cube, or a non-cutout model).
//! - Faces: the top unless the same fluid is above (`isNeighborSameFluid`), the bottom and
//!   sides through `FluidRenderer.shouldRenderFace` (which adds `isFaceOccludedBySelf`:
//!   faces the fluid block's own shape covers, as inside a waterlogged slab) and then
//!   `isFaceOccludedByNeighbor` with the neighbour's occlusion shape
//!   (`fluid_occlusion.rs`): the bottom at `MAX_FLUID_HEIGHT`, the top at its lowest corner,
//!   each side at its higher corner. The top's underside returns only for
//!   `FluidState.shouldRenderBackwardUpFace`. Every face is pulled 0.001 into the block
//!   (`tesselate`'s `offs`). APPROXIMATE: the mesher's `FluidWorld::face_occluder` answers
//!   full cubes only so far; slabs and stairs need its shape data wired in.
//! - Flow: `FlowingFluid.getFlow` (horizontal part; the FALLING branch only moves the y
//!   component, which `tesselate` never reads) picks the still or the flowing sprite and
//!   rotates the flow sprite's middle-half window along the flow. APPROXIMATE: the
//!   `blocks_fluid_flow` tag (`#blocks_motion` + `#all_signs`) is the blocks-motion
//!   approximation above, so signs slip through.
//! - Textures: sides use the flow sprite's left half, cut at the surface height; water sides
//!   next to glass or leaves use `water_overlay` (single-sided). Animated sprites show their
//!   first frame (the atlas has no animation yet).
//! - Light: the brighter of the block's own cell and the cell above per channel (bottom: the
//!   cell below and its own), no ambient occlusion; directional face shade like blocks.
//! - Water is tinted by the blended biome water colour and goes to the translucent layer;
//!   lava is untinted and solid.

pub use crate::fluid_occlusion::{is_face_occluded, Occluder};
use haste_assets::{BakedBlockModels, RenderLayer};
use haste_core::BlockStateId;

mod geometry;
mod quads;
mod registry;
#[cfg(test)]
mod tests;

pub use geometry::{
    corner_heights, fluid_flow, should_render_backward_up_face, INSET, MAX_FLUID_HEIGHT,
};
pub use quads::{fluid_quads, FluidQuad};
pub use registry::{
    Fluid, FluidDef, FluidRules, FluidSprites, FluidState, FluidTable, FluidTint, FluidWorld,
    Fluids,
};

/// Whether a baked block counts as solid for corner averaging (see the module docs).
pub fn baked_solid(baked: &BakedBlockModels, state: BlockStateId) -> bool {
    state != BlockStateId::AIR
        && (baked.is_full_opaque(state)
            || (!baked.quads(state).is_empty()
                && !matches!(
                    baked.render_layer(state),
                    RenderLayer::Cutout | RenderLayer::CutoutMipped
                )))
}

/// Render layer of a fluid.
pub fn fluid_layer(defs: &Fluids, fluid: Fluid) -> RenderLayer {
    defs.def(fluid).layer
}
