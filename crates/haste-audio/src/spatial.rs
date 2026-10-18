//! Pure spatial-audio math: the listener transform, distance attenuation, stereo panning
//! and pitch clamping.
//!
//! No rodio, no devices; everything here is directly testable.
use glam::DVec3;

/// The listener's position and orientation, vanilla's `ListenerTransform` (what
/// `Listener.setTransform` hands to OpenAL: position, forward, up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListenerTransform {
    pub position: DVec3,
    pub forward: DVec3,
    pub up: DVec3,
}

impl ListenerTransform {
    /// The transform of a camera at `position` looking with Minecraft yaw/pitch (the
    /// vanilla camera: yaw 0 = facing +Z (south), 90 = facing -X (west), increasing
    /// clockwise seen from above; positive pitch looks down). The up vector tilts with
    /// the pitch, so `right` stays defined even looking straight up or down.
    pub fn from_look(position: DVec3, yaw_deg: f32, pitch_deg: f32) -> Self {
        let yaw = (yaw_deg as f64).to_radians();
        let pitch = (pitch_deg as f64).to_radians();
        let forward = DVec3::new(
            -yaw.sin() * pitch.cos(),
            -pitch.sin(),
            yaw.cos() * pitch.cos(),
        );
        let up = DVec3::new(
            -yaw.sin() * pitch.sin(),
            pitch.cos(),
            yaw.cos() * pitch.sin(),
        );
        Self {
            position,
            forward,
            up,
        }
    }

    /// The listener's right-hand direction: `forward x up`. At yaw 0 that is (-1, 0, 0),
    /// west, the listener's right when facing south. Horizontal and pitch-independent, so
    /// it never degenerates.
    pub fn right(&self) -> DVec3 {
        self.forward.cross(self.up)
    }
}

/// Gain of a positional sound: OpenAL's linear distance model as vanilla configures it
/// (`Channel.linearAttenuation`: AL_LINEAR_DISTANCE with reference distance 1.0, rolloff
/// factor 1 and max distance `max(volume, 1.0) * attenuation_distance`, vanilla's
/// `Sound.getAttenuationDistance(volume)`).
///
/// The distance factor is `1 - (distance - 1) / (max - 1)`, clamped to 0..=1: full up to the
/// reference distance of one block, then a straight line to silence at the max distance.
/// The channel's own volume (`SoundEngine.calculateVolume`, `min(volume, 1.0)` times the
/// source volume) multiplies it; the result is clamped to 0..=1.
pub fn attenuation(distance: f64, volume: f32, attenuation_distance: u32) -> f32 {
    let max = volume.max(1.0) * attenuation_distance as f32;
    let falloff = if max > 1.0 {
        (1.0 - (distance - 1.0) / (max as f64 - 1.0)).clamp(0.0, 1.0)
    } else if distance <= 1.0 {
        // No room for a slope: full at (or under) the reference distance, silent beyond it.
        1.0
    } else {
        0.0
    };
    (volume.min(1.0) * falloff as f32).clamp(0.0, 1.0)
}

/// Stereo pan of a source in -1 (left) ..= 1 (right) of the listener, from the listener's
/// forward/up basis like `Listener.setTransform` gives OpenAL: the source's horizontal
/// direction dotted with the listener's right vector, which is the sine of the source's
/// azimuth around the listener. A source straight ahead or behind pans 0, directly to the
/// listener's right is +1; a source at the listener's position pans 0. Elevation does not
/// matter (only the azimuth does).
pub fn pan(listener: &ListenerTransform, source: DVec3) -> f32 {
    let delta = source - listener.position;
    let horizontal = DVec3::new(delta.x, 0.0, delta.z);
    if horizontal.length_squared() < f64::EPSILON {
        return 0.0;
    }
    (horizontal.normalize().dot(listener.right()) as f32).clamp(-1.0, 1.0)
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
    fn attenuation_is_full_up_to_the_reference_distance_of_one_block() {
        // Channel.linearAttenuation sets AL_REFERENCE_DISTANCE 1.0: no falloff until one
        // block out.
        assert_eq!(attenuation(1.0, 1.0, 16), 1.0);
    }

    #[test]
    fn attenuation_at_half_range_is_over_half() {
        // 1 - (8 - 1) / (16 - 1) = 8/15 (the reference distance shifts the line).
        assert!((attenuation(8.0, 1.0, 16) - 8.0 / 15.0).abs() < 1e-6);
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
    fn attenuation_with_no_room_for_a_slope_steps_at_the_reference_distance() {
        // attenuation_distance 0: max = 0 <= reference 1, so the falloff is a step.
        assert_eq!(attenuation(0.0, 1.0, 0), 1.0);
        assert_eq!(attenuation(1.0, 1.0, 0), 1.0);
        assert_eq!(attenuation(1.5, 1.0, 0), 0.0);
    }

    #[test]
    fn volume_above_one_doubles_the_range() {
        // volume 2.0 with attenuation_distance 16: audible to 32 blocks, gain <= 1.
        // Falloff 1 - (d - 1) / 31.
        assert_eq!(attenuation(0.0, 2.0, 16), 1.0);
        assert!((attenuation(16.0, 2.0, 16) - 16.0 / 31.0).abs() < 1e-6);
        assert_eq!(attenuation(32.0, 2.0, 16), 0.0);
        assert_eq!(attenuation(40.0, 2.0, 16), 0.0);
    }

    #[test]
    fn volume_below_one_halves_the_gain_but_keeps_the_range() {
        // volume 0.5: range stays 16 blocks, gain caps at 0.5.
        assert_eq!(attenuation(0.0, 0.5, 16), 0.5);
        assert!((attenuation(8.0, 0.5, 16) - 4.0 / 15.0).abs() < 1e-6);
        assert_eq!(attenuation(16.0, 0.5, 16), 0.0);
    }

    #[test]
    fn pan_yaw_zero_cardinal_directions() {
        // Yaw 0: facing +Z (south). Right hand points west (-X).
        let listener = ListenerTransform::from_look(DVec3::ZERO, 0.0, 0.0);
        // Source at +X (east) is to the LEFT: negative pan.
        assert!(pan(&listener, DVec3::new(10.0, 0.0, 0.0)) < -0.99);
        // Source at -X (west) is to the RIGHT: positive pan.
        assert!(pan(&listener, DVec3::new(-10.0, 0.0, 0.0)) > 0.99);
        // Straight ahead (+Z) and behind (-Z): centred.
        assert_eq!(pan(&listener, DVec3::new(0.0, 0.0, 10.0)), 0.0);
        assert_eq!(pan(&listener, DVec3::new(0.0, 0.0, -10.0)), 0.0);
    }

    #[test]
    fn pan_yaw_90_faces_west() {
        // Yaw 90: facing -X (west). Right hand points north (-Z).
        let listener = ListenerTransform::from_look(DVec3::ZERO, 90.0, 0.0);
        assert!(pan(&listener, DVec3::new(0.0, 0.0, -10.0)) > 0.99);
        assert!(pan(&listener, DVec3::new(0.0, 0.0, 10.0)) < -0.99);
        assert!(pan(&listener, DVec3::new(-10.0, 0.0, 0.0)).abs() < 1e-6);
    }

    #[test]
    fn pan_from_a_source_left_and_right_of_the_listener() {
        // A source left of the listener is louder on the left channel, one on the right
        // louder on the right, by the pan the listener's basis gives.
        let listener = ListenerTransform::from_look(DVec3::ZERO, 0.0, 0.0);
        // Yaw 0 faces +Z (south): west (-X) is the listener's right.
        let (l_right, r_right) = stereo_gains(1.0, pan(&listener, DVec3::new(-10.0, 0.0, 0.0)));
        assert!(r_right > 0.99 && l_right.abs() < 1e-5, "west is right");
        let (l_left, r_left) = stereo_gains(1.0, pan(&listener, DVec3::new(10.0, 0.0, 0.0)));
        assert!(l_left > 0.99 && r_left.abs() < 1e-5, "east is left");
        // The same with the listener turned 90 degrees (facing west): north (-Z) is now
        // the right.
        let listener = ListenerTransform::from_look(DVec3::ZERO, 90.0, 0.0);
        let (l, r) = stereo_gains(1.0, pan(&listener, DVec3::new(0.0, 0.0, -10.0)));
        assert!(r > 0.99 && l.abs() < 1e-5);
    }

    #[test]
    fn pan_survives_looking_straight_down() {
        // The forward/up basis stays defined at pitch +/- 90 (up tilts horizontal), so the
        // right vector does not degenerate.
        let listener = ListenerTransform::from_look(DVec3::ZERO, 30.0, 90.0);
        let right = listener.right();
        assert!(
            right.length_squared() > 1e-12,
            "right {right:?} degenerated"
        );
        // Facing down with yaw 30: right is the horizontal perpendicular of the look,
        // (-cos 30, 0, -sin 30); a source out that way pans hard right.
        assert!(pan(&listener, DVec3::new(-8.66, 5.0, -5.0)) > 0.99);
    }

    #[test]
    fn pan_ignores_height() {
        let listener = ListenerTransform::from_look(DVec3::ZERO, 0.0, 0.0);
        assert_eq!(
            pan(&listener, DVec3::new(10.0, 500.0, 0.0)),
            pan(&listener, DVec3::new(10.0, 0.0, 0.0)),
        );
    }

    #[test]
    fn pan_ignores_listener_pitch() {
        // Only the azimuth around the up axis matters: pitching the listener does not
        // move the left/right axis.
        let level = ListenerTransform::from_look(DVec3::ZERO, 0.0, 0.0);
        let down = ListenerTransform::from_look(DVec3::ZERO, 0.0, 45.0);
        for source in [
            DVec3::new(10.0, 0.0, 0.0),
            DVec3::new(-10.0, 3.0, 7.0),
            DVec3::new(4.0, -2.0, -9.0),
        ] {
            assert_eq!(pan(&level, source), pan(&down, source));
        }
    }

    #[test]
    fn transform_from_look_builds_the_camera_basis() {
        let t = ListenerTransform::from_look(DVec3::new(1.0, 64.0, 2.0), 0.0, 0.0);
        assert_eq!(t.position, DVec3::new(1.0, 64.0, 2.0));
        assert!(
            t.forward.distance(DVec3::new(0.0, 0.0, 1.0)) < 1e-9,
            "south"
        );
        assert!(t.up.distance(DVec3::new(0.0, 1.0, 0.0)) < 1e-9);
        let t = ListenerTransform::from_look(DVec3::ZERO, 90.0, 0.0);
        assert!(
            t.forward.distance(DVec3::new(-1.0, 0.0, 0.0)) < 1e-9,
            "west"
        );
        // Positive pitch looks down: forward gains -Y, up tilts toward +Z.
        let t = ListenerTransform::from_look(DVec3::ZERO, 0.0, 90.0);
        assert!(
            t.forward.distance(DVec3::new(0.0, -1.0, 0.0)) < 1e-9,
            "down"
        );
        assert!(
            t.up.distance(DVec3::new(0.0, 0.0, 1.0)) < 1e-9,
            "up tilts south"
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
