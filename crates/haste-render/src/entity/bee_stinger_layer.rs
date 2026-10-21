//! `BeeStingerLayer`: the bee stingers stuck in a humanoid (`StuckInBodyLayer` with the
//! `ON_SURFACE` placement and `BeeStingerModel`). Each stinger is the stinger model's cross
//! parts, drawn through the stuck-item transform.

use glam::Mat4;
use haste_core::model::{CubeModel, PoseOverrides};

use super::model_parts::emit_part;
use super::stuck_in_body::{for_each_stuck_item, Placement};
use super::vertex::EntityVertex;

/// Pushes `count` stuck stingers on the body described by `body`, through `root` (the entity's
/// transform with the model scale). `stinger` is the `BeeStingerModel`, `rect` its texture's
/// atlas rectangle.
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_stuck_stingers(
    out: &mut Vec<EntityVertex>,
    root: Mat4,
    body: &CubeModel,
    pose: &PoseOverrides,
    body_parts: &[&str],
    count: u32,
    seed: i64,
    stinger: &CubeModel,
    light: (u8, u8),
    rect: [u32; 4],
) {
    let still = PoseOverrides::default();
    for_each_stuck_item(
        root,
        body,
        pose,
        body_parts,
        count,
        seed,
        Placement::OnSurface,
        |m| emit_part(out, stinger, &stinger.root, &still, m, rect, light, 0.0),
    );
}
