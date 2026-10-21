//! `EntityRenderer.shouldRender`: the distance check (`Entity.shouldRenderAtSqrDistance` and
//! the per-type overrides) and the frustum test of the culling box.

use super::*;

/// Vanilla `Entity.shouldRenderAtSqrDistance`: whether an entity whose model is `size`
/// blocks and has entity `scale` should be drawn at squared distance `dist_sq`, given the
/// `view_scale` (`Entity.viewScale`, from `options.entityDistanceScaling`).
pub fn should_render_at_sqr_distance(
    size: f32,
    entity_scale: f32,
    dist_sq: f64,
    view_scale: f32,
) -> bool {
    let size = if size.is_nan() { 1.0 } else { size };
    let limit = 64.0 * size.max(1.0) * entity_scale * view_scale;
    dist_sq < f64::from(limit * limit)
}

/// Whether `e` is drawn this frame (`EntityRenderDispatcher.shouldRender`): beyond its render
/// distance or outside the camera frustum it is skipped. `rel` is its position relative to
/// the camera.
pub(super) fn should_render(
    e: &EntityInstance,
    rel: glam::DVec3,
    frustum: &[glam::Vec4; 6],
    view_scale: f32,
) -> bool {
    let dist2 = rel.length_squared();
    // `Display` has its own distance (its view range) and culling box.
    if let Some(Decoration::Display(d)) = &e.decoration {
        return display::should_render(d, rel, dist2, frustum, view_scale);
    }
    // Vanilla skips entities beyond `64 * boundingBox.size` blocks or outside the camera
    // frustum (the AABB inflated by 0.5, `Frustum.isVisible`). Same look: those entities
    // would draw nothing visible anyway.
    let (size, half) = model_extents(&e.model.root);
    // `AbstractArrow.shouldRenderAtSqrDistance` overrides the generic limit with the
    // bounding box diagonal times ten, so arrows stay visible much farther out.
    let visible = if matches!(&e.decoration, Some(Decoration::Arrow(_))) {
        arrow::arrow_should_render(dist2, e.width, e.height, view_scale)
    } else {
        should_render_at_sqr_distance(size, e.scale, dist2, view_scale)
    };
    visible && aabb_in_frustum(frustum, rel, half + 0.5)
}

/// `Frustum.isVisible` for an axis-aligned box given by its camera-relative `min` and `max`:
/// outside as soon as the box lies wholly behind one plane.
pub(super) fn box_in_frustum(frustum: &[glam::Vec4; 6], min: Vec3, max: Vec3) -> bool {
    frustum.iter().all(|p| {
        // The box corner furthest along the plane's normal.
        let corner = Vec3::new(
            if p.x >= 0.0 { max.x } else { min.x },
            if p.y >= 0.0 { max.y } else { min.y },
            if p.z >= 0.0 { max.z } else { min.z },
        );
        p.x * corner.x + p.y * corner.y + p.z * corner.z + p.w >= 0.0
    })
}
