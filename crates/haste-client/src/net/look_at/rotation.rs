//! `Entity.lookAt`'s angles: the yaw and pitch that face a point from an anchor position.

use glam::DVec3;

use super::atan2::atan2;

/// `Mth.wrapDegrees(float)`: the angle in [-180, 180).
pub(super) fn wrap_degrees(angle: f32) -> f32 {
    let mut n = angle % 360.0;
    if n >= 180.0 {
        n -= 360.0;
    }
    if n < -180.0 {
        n += 360.0;
    }
    n
}

/// `Entity.lookAt`'s (yRot, xRot) for facing `to` from `from`, both wrapped to [-180, 180).
pub(super) fn look_angles(from: DVec3, to: DVec3) -> (f32, f32) {
    let d = to - from;
    let sd = (d.x * d.x + d.z * d.z).sqrt();
    let pi = std::f32::consts::PI;
    let pitch = wrap_degrees((-(atan2(d.y, sd) * 180.0 / f64::from(pi))) as f32);
    let yaw = wrap_degrees((atan2(d.z, d.x) * 180.0 / f64::from(pi)) as f32 - 90.0);
    (yaw, pitch)
}
