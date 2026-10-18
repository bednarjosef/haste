//! Water and lava meshing.
//!
//! Behaviour from minecraft.wiki ("Water", "Lava", "Fluid") and screenshots of the real game;
//! the rules implemented:
//!
//! - Fluid per block state: `water`/`lava` with `level` 0 (source, height 8/9), 1..7 (flowing,
//!   height (8 - level)/9), 8..15 (falling, height 8/9); `waterlogged=true` states and the
//!   always-underwater plants (`kelp`, `kelp_plant`, `seagrass`, `tall_seagrass`,
//!   `bubble_column`) hold a water source.
//! - Column height for corner averaging: 1.0 when the same fluid is also above it, else its own
//!   height; a non-fluid block counts as 0 (it pulls the corner down) unless it is "solid", in
//!   which case it is ignored. Corner height: 1.0 if either side column is full; otherwise a
//!   weighted average of the block, the two side columns and (when a side holds fluid) the
//!   diagonal column, where columns of height >= 0.8 weigh 10 and lower ones 1.
//!   APPROXIMATE: "solid" is "full opaque cube, or any block with a model outside the cutout
//!   layers" (vanilla uses the block's blocks-motion flag).
//! - Faces: the top is drawn unless the same fluid is above (and also drawn reversed, so it is
//!   visible from inside the fluid); sides are culled against the same fluid and full opaque
//!   blocks and drawn on both sides; the bottom is drawn unless the same fluid or a full
//!   opaque block is below. APPROXIMATE: partial blocks (slabs, stairs) never cull a fluid
//!   face.
//! - Textures: still sprite on the bottom and on tops without flow; flowing tops use the flow
//!   sprite rotated so the texture runs along the flow (the downhill gradient of the corner
//!   heights), sampling the middle half of the 32px flow sprite; sides use the flow sprite's
//!   left half, cut at the surface height. Water sides next to glass or leaves use
//!   `water_overlay` (single-sided). Animated sprites show their first frame (the atlas has no
//!   animation yet).
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
