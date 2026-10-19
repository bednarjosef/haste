//! The thrown-item billboard transform, against `ThrownItemRenderer.submit`'s order:
//! translate to the entity, scale by the renderer's `scale`, rotate to the camera.
use super::thrown_item::thrown_item_root;
use super::*;

/// Translation first, then scale: a point of the item model lands at origin + scale * point.
#[test]
fn root_translates_then_scales() {
    let root = thrown_item_root(Vec3::new(1.0, 2.0, 3.0), 2.0, glam::Quat::IDENTITY);
    let p = root * Vec3::new(0.5, 0.0, 0.25).extend(1.0);
    assert!((p.truncate() - Vec3::new(2.0, 2.0, 3.5)).length() < 1e-6);
}

/// The camera orientation turns the sprite after the scale: a quarter turn about y maps the
/// model's x axis onto the world's -z (camera-relative) axis, and the scale still applies.
#[test]
fn root_rotates_after_the_scale() {
    let yaw = glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    let root = thrown_item_root(Vec3::new(1.0, 2.0, 3.0), 0.75, yaw);
    let p = root * Vec3::new(1.0, 0.0, 0.0).extend(1.0);
    assert!(
        (p.truncate() - Vec3::new(1.0, 2.0, 3.0 - 0.75)).length() < 1e-6,
        "x maps to -z at 0.75 scale: {p:?}"
    );
}
