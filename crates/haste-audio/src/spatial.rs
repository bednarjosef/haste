//! Pure spatial-audio math: distance attenuation, stereo panning and pitch clamping.
//!
//! No rodio, no devices; everything here is directly testable.
use glam::DVec3;

/// Gain of a positional sound, per vanilla (minecraft.wiki, "Sound" / "Sounds.json").
///
/// A sound is audible within `attenuation_distance * max(volume, 1.0)` blocks (the wiki's
/// default is 16 when the field is absent). Inside that range the gain falls off LINEARLY
/// from full at distance 0 to 0 at the range; the actual gain is `min(volume, 1.0)` times
/// that falloff. The result is clamped to 0..=1.
///
/// Not confirmed: whether the falloff is exactly linear in vanilla (the wiki describes the
/// audible range and that volume above 1 extends it, but not the precise curve shape).
pub fn attenuation(distance: f64, volume: f32, attenuation_distance: u32) -> f32 {
    let range = attenuation_distance as f64 * volume.max(1.0) as f64;
    if range <= 0.0 {
        return 0.0;
    }
    let falloff = (1.0 - distance / range).clamp(0.0, 1.0);
    (volume.min(1.0) * falloff as f32).clamp(0.0, 1.0)
}

/// Stereo pan of a source in -1 (left) ..= 1 (right) relative to the listener's facing.
///
/// Only the horizontal plane matters. Minecraft yaw: 0 = facing +Z (south), 90 = facing -X
/// (west), increasing clockwise seen from above. A source straight ahead or behind pans 0,
/// directly to the listener's right is +1; a source at the listener's position pans 0.
///
/// Sign convention check with yaw 0 (facing +Z, south): the listener's right hand points
/// west (-X), so a source at -X is +1 (right) and a source at +X is -1 (left).
pub fn pan(listener_pos: DVec3, listener_yaw_deg: f32, source: DVec3) -> f32 {
    let delta = source - listener_pos;
    let horizontal = DVec3::new(delta.x, 0.0, delta.z);
    if horizontal.length_squared() < f64::EPSILON {
        return 0.0;
    }
    let yaw = listener_yaw_deg.to_radians();
    // Forward direction for the given yaw (0 = +Z, 90 = -X).
    let forward = DVec3::new(-(yaw.sin() as f64), 0.0, yaw.cos() as f64);
    // Right = forward x up: at yaw 0 that is (-1, 0, 0), i.e. west, the listener's right
    // when facing south.
    let right = DVec3::new(-forward.z, 0.0, forward.x);
    (horizontal.normalize().dot(right) as f32).clamp(-1.0, 1.0)
}

/// Constant-power stereo gains for a gain/pan pair: `left^2 + right^2 == gain^2`.
///
/// Pan 0 splits the gain evenly (`gain / sqrt(2)` per channel), -1 is all left, +1 all
/// right.
pub fn stereo_gains(gain: f32, pan: f32) -> (f32, f32) {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    (gain * angle.cos(), gain * angle.sin())
}

/// Clamp a pitch multiplier into the range vanilla's protocol allows: 0.5..=2.0.
pub fn clamp_pitch(p: f32) -> f32 {
    p.clamp(0.5, 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attenuation_at_distance_zero_is_full() {
        assert_eq!(attenuation(0.0, 1.0, 16), 1.0);
    }

    #[test]
    fn attenuation_at_half_range_is_half() {
        assert_eq!(attenuation(8.0, 1.0, 16), 0.5);
    }

    #[test]
    fn attenuation_at_range_is_zero() {
        assert_eq!(attenuation(16.0, 1.0, 16), 0.0);
    }

    #[test]
    fn attenuation_beyond_range_is_zero() {
        assert_eq!(attenuation(100.0, 1.0, 16), 0.0);
        assert_eq!(attenuation(f64::INFINITY, 1.0, 16), 0.0);
    }

    #[test]
    fn volume_above_one_doubles_the_range() {
        // volume 2.0 with attenuation_distance 16: audible to 32 blocks, gain <= 1.
        assert_eq!(attenuation(0.0, 2.0, 16), 1.0);
        assert_eq!(attenuation(16.0, 2.0, 16), 0.5);
        assert_eq!(attenuation(32.0, 2.0, 16), 0.0);
        assert_eq!(attenuation(40.0, 2.0, 16), 0.0);
    }

    #[test]
    fn volume_below_one_halves_the_gain_but_keeps_the_range() {
        // volume 0.5: range stays 16 blocks, gain caps at 0.5.
        assert_eq!(attenuation(0.0, 0.5, 16), 0.5);
        assert_eq!(attenuation(8.0, 0.5, 16), 0.25);
        assert_eq!(attenuation(16.0, 0.5, 16), 0.0);
    }

    #[test]
    fn pan_yaw_zero_cardinal_directions() {
        // Yaw 0: facing +Z (south). Right hand points west (-X).
        let listener = DVec3::new(0.0, 0.0, 0.0);
        // Source at +X (east) is to the LEFT: negative pan.
        assert!(pan(listener, 0.0, DVec3::new(10.0, 0.0, 0.0)) < -0.99);
        // Source at -X (west) is to the RIGHT: positive pan.
        assert!(pan(listener, 0.0, DVec3::new(-10.0, 0.0, 0.0)) > 0.99);
        // Straight ahead (+Z) and behind (-Z): centred.
        assert_eq!(pan(listener, 0.0, DVec3::new(0.0, 0.0, 10.0)), 0.0);
        assert_eq!(pan(listener, 0.0, DVec3::new(0.0, 0.0, -10.0)), 0.0);
    }

    #[test]
    fn pan_yaw_90_faces_west() {
        // Yaw 90: facing -X (west). Right hand points north (-Z).
        let listener = DVec3::new(0.0, 0.0, 0.0);
        assert!(pan(listener, 90.0, DVec3::new(0.0, 0.0, -10.0)) > 0.99);
        assert!(pan(listener, 90.0, DVec3::new(0.0, 0.0, 10.0)) < -0.99);
        assert!(pan(listener, 90.0, DVec3::new(-10.0, 0.0, 0.0)).abs() < 1e-6);
    }

    #[test]
    fn pan_at_listener_position_is_zero() {
        assert_eq!(
            pan(
                DVec3::new(1.0, 64.0, 1.0),
                137.0,
                DVec3::new(1.0, 64.0, 1.0)
            ),
            0.0
        );
    }

    #[test]
    fn pan_ignores_height() {
        let listener = DVec3::ZERO;
        assert_eq!(
            pan(listener, 0.0, DVec3::new(10.0, 500.0, 0.0)),
            pan(listener, 0.0, DVec3::new(10.0, 0.0, 0.0)),
        );
    }

    #[test]
    fn stereo_gains_is_constant_power() {
        for (gain, pan) in [
            (1.0, 0.0),
            (1.0, -1.0),
            (1.0, 1.0),
            (0.7, -0.3),
            (0.25, 0.9),
        ] {
            let (l, r) = stereo_gains(gain, pan);
            assert!(
                (l * l + r * r - gain * gain).abs() < 1e-5,
                "gain {gain} pan {pan}"
            );
        }
    }

    #[test]
    fn stereo_gains_extremes() {
        let (l, r) = stereo_gains(1.0, -1.0);
        assert!((l - 1.0).abs() < 1e-6 && r.abs() < 1e-6);
        let (l, r) = stereo_gains(1.0, 1.0);
        assert!(l.abs() < 1e-6 && (r - 1.0).abs() < 1e-6);
        let (l, r) = stereo_gains(1.0, 0.0);
        assert!((l - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((r - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
    }

    #[test]
    fn clamp_pitch_limits() {
        assert_eq!(clamp_pitch(1.0), 1.0);
        assert_eq!(clamp_pitch(0.1), 0.5);
        assert_eq!(clamp_pitch(10.0), 2.0);
        assert_eq!(clamp_pitch(0.5), 0.5);
        assert_eq!(clamp_pitch(2.0), 2.0);
    }
}
