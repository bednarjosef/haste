//! `HumanoidModel.setupSwimAnimation`: the crawl stroke (moved out of `animation.rs`
//! unchanged; see there for the rest of `HumanoidModel.setupAnim`).
use std::f32::consts::{FRAC_PI_2, PI};

use haste_core::math::Vec3;

use crate::store::Entity;

use super::{lerp, rot_lerp_rad};

/// `HumanoidModel.setupSwimAnimation`: the crawl stroke. `swim` is the smoothed swim amount; the
/// arm of the swinging hand keeps its pose (`state.currentSwing`).
pub(super) fn setup_swim(
    ra: &mut Vec3,
    la: &mut Vec3,
    rl: &mut Vec3,
    ll: &mut Vec3,
    walk_pos: f32,
    swim: f32,
    e: &Entity,
) {
    let quadratic = |x: f32| -65.0 * x + x * x;
    let swim_pos = walk_pos % 26.0;
    // Main hand is right: hand 0 = right, 1 = left. No swing means both arms swim.
    let (right_amount, left_amount) = if e.swing.is_swinging() {
        if e.swing.hand() == 0 {
            (0.0, swim)
        } else {
            (swim, 0.0)
        }
    } else {
        (swim, swim)
    };
    if swim_pos < 14.0 {
        la.x = rot_lerp_rad(left_amount, la.x, 0.0);
        ra.x = lerp(right_amount, ra.x, 0.0);
        la.y = rot_lerp_rad(left_amount, la.y, PI);
        ra.y = lerp(right_amount, ra.y, PI);
        la.z = rot_lerp_rad(
            left_amount,
            la.z,
            PI + 1.8707964 * quadratic(swim_pos) / quadratic(14.0),
        );
        ra.z = lerp(
            right_amount,
            ra.z,
            PI - 1.8707964 * quadratic(swim_pos) / quadratic(14.0),
        );
    } else if swim_pos < 22.0 {
        let p = (swim_pos - 14.0) / 8.0;
        la.x = rot_lerp_rad(left_amount, la.x, FRAC_PI_2 * p);
        ra.x = lerp(right_amount, ra.x, FRAC_PI_2 * p);
        la.y = rot_lerp_rad(left_amount, la.y, PI);
        ra.y = lerp(right_amount, ra.y, PI);
        la.z = rot_lerp_rad(left_amount, la.z, 5.012389 - 1.8707964 * p);
        ra.z = lerp(right_amount, ra.z, 1.2707963 + 1.8707964 * p);
    } else if swim_pos < 26.0 {
        let p = (swim_pos - 22.0) / 4.0;
        la.x = rot_lerp_rad(left_amount, la.x, FRAC_PI_2 - FRAC_PI_2 * p);
        ra.x = lerp(right_amount, ra.x, FRAC_PI_2 - FRAC_PI_2 * p);
        la.y = rot_lerp_rad(left_amount, la.y, PI);
        ra.y = lerp(right_amount, ra.y, PI);
        la.z = rot_lerp_rad(left_amount, la.z, PI);
        ra.z = lerp(right_amount, ra.z, PI);
    }
    ll.x = lerp(swim, ll.x, 0.3 * (walk_pos * 0.33333334 + PI).cos());
    rl.x = lerp(swim, rl.x, 0.3 * (walk_pos * 0.33333334).cos());
}
