// The flat cloud cells (vanilla `CloudRenderer.buildFlatCell`): the FAST cloud mode
// draws one down face per cell at the layer's bottom, shaded with the top colour
// (vanilla's `FLAG_USE_TOP_COLOR`, `faceColors[1]` in `clouds.vsh`) and without
// back-face culling (`RenderPipelines.FLAT_CLOUDS` is the clouds pipeline with
// `withCull(false)`), so the plane is visible from below and above alike.

use super::mesh::{pack_face, Dir};

/// Vanilla `buildFlatCell`: one `Direction.DOWN` face carrying `FLAG_USE_TOP_COLOR`.
pub(super) fn build_flat_cell(out: &mut Vec<u32>, x: i32, z: i32) {
    out.push(pack_face(x, z, Dir::Down, true, false));
}
