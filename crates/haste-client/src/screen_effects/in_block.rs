//! The block the camera is inside: `LevelExtractor.getViewBlockingState` and the sprite
//! `ScreenEffectRenderer.submitBlockSprite` draws.
use haste_protocol::haste_core::Identifier;
/// Vanilla's `ScreenEffectRenderer.submitBlockSprite` tint (`-15132391` as ARGB).
pub(super) const BLOCK_OVERLAY_TINT: u32 = 0xFF19_1919;

/// The view-blocking sample points: the eight corners of a box around the eye, as vanilla's
/// `getViewBlockingState` walks them (`0.8 * bbWidth` wide, `0.1` tall).
pub fn view_probe_points(eye: glam::DVec3, bb_width: f64, scale: f64) -> [glam::DVec3; 8] {
    std::array::from_fn(|i| {
        glam::DVec3::new(
            eye.x + ((i % 2) as f64 - 0.5) * bb_width * 0.8,
            eye.y + (((i >> 1) % 2) as f64 - 0.5) * 0.1 * scale,
            eye.z + (((i >> 2) % 2) as f64 - 0.5) * bb_width * 0.8,
        )
    })
}

/// Vanilla's `causes_suffocation` block tag, which with a full collision shape is what makes a
/// block view-blocking (`BlockBehaviour`'s `isSuffocating` default).
fn causes_suffocation() -> Identifier {
    Identifier::minecraft("causes_suffocation")
}

/// The block's particle texture (`BlockStateModelSet.getParticleMaterial`), which is what the
/// in-block overlay draws. Every block's is `textures/block/<name>.png`.
pub(super) fn particle_texture(block: &Identifier) -> Identifier {
    Identifier::minecraft(&format!("block/{}", block.path()))
}

/// Whether a state blocks the view: in the `causes_suffocation` tag and with a full collision
/// shape (`BlockBehaviour.BlockStateBase.isSuffocating`).
pub(super) fn is_view_blocking(
    state: haste_core::BlockStateId,
    blocks: &haste_data::BlockStates,
    shapes: &dyn haste_core::view::CollisionShapes,
    tags: Option<&haste_physics::mining::BlockTags>,
) -> bool {
    let block = blocks.block_of(state);
    let Some(tags) = tags else { return false };
    let Some(index) = blocks.block_index(block) else {
        return false;
    };
    if !tags.contains(&causes_suffocation(), index.0) {
        return false;
    }
    // `isCollisionShapeFullBlock`: the collision shape covers the whole unit cube.
    let boxes = shapes.collision_boxes(state);
    boxes.len() == 1
        && boxes.iter().all(|b| {
            b.min.abs_diff_eq(glam::DVec3::ZERO, 1e-9)
                && b.max.abs_diff_eq(glam::DVec3::splat(1.0), 1e-9)
        })
}
