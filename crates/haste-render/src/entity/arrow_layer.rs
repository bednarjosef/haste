//! `ArrowLayer`: the arrows stuck in a humanoid (`StuckInBodyLayer` with the `IN_CUBE`
//! placement and `ArrowModel`). Each stuck arrow is the flight arrow's quads, placed by the
//! stuck-item transform; the quads are in blocks, so they are scaled back to model units first.

use glam::{Mat4, Vec3};
use haste_core::model::{CubeModel, PoseOverrides};

use super::arrow::push_arrow_model;
use super::stuck_in_body::{for_each_stuck_item, Placement};
use super::vertex::EntityVertex;

/// Pushes `count` stuck arrows on the body described by `body`, through `root` (the entity's
/// transform with the model scale). `rect` is the arrow texture's atlas rectangle.
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_stuck_arrows(
    out: &mut Vec<EntityVertex>,
    root: Mat4,
    body: &CubeModel,
    pose: &PoseOverrides,
    body_parts: &[&str],
    count: u32,
    seed: i64,
    light: (u8, u8),
    rect: [u32; 4],
) {
    let to_model_units = Mat4::from_scale(Vec3::splat(16.0));
    for_each_stuck_item(
        root,
        body,
        pose,
        body_parts,
        count,
        seed,
        Placement::InCube,
        |m| push_arrow_model(out, m * to_model_units, light, rect),
    );
}
