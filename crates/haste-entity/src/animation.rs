//! Per-instance pose overrides: limb swing, head aim, pose adjustments.
//!
//! Limb swing (publicly documented vanilla behaviour): limbs oscillate with
//! `cos(limb_swing * 0.6662)` where `limb_swing` accumulates the smoothed horizontal speed and
//! `limb_speed` (the amplitude) is the per-tick distance x 4, smoothed by 0.4 per tick. At rest
//! the amplitude is zero, so the overrides are empty apart from the head. Sneaking tips the body
//! forward by 0.5 rad and compensates the head; swimming/elytra poses tip it 90 degrees.
//!
//! The renderer (WS3) applies `EntityInstance.yaw` to the whole model and these additive
//! rotations on top of each part's base rotation; the "head" entry carries the look direction
//! (`head_yaw` relative to body yaw, `pitch`), clamped to +-75 degrees like vanilla's body/head
//! split. If the renderer prefers to aim the head itself from `EntityInstance.head_yaw`, it
//! should skip the "head" entries here - they are the same information expressed once.
use haste_core::entity::Pose;
use haste_core::math::Vec3;
use haste_core::model::PoseOverrides;

use crate::store::Entity;

/// Vanilla's head/body split clamp, in radians (75 degrees).
const HEAD_CLAMP: f32 = 75.0_f32.to_radians();

pub(crate) fn pose_overrides(e: &Entity, partial_tick: f32) -> PoseOverrides {
    let mut o = PoseOverrides::default();
    let swing = e.limb_swing + e.limb_speed * partial_tick;
    let amp = e.limb_speed.clamp(0.0, 1.0);
    if amp > 1e-4 {
        let arm = 1.0_f32 * amp; // vanilla: 2.0 * amp * 0.5
        let leg = 1.4 * amp;
        o.rotations.push((
            "right_arm".into(),
            Vec3::new(
                -(swing * 0.6662 + std::f32::consts::PI).cos() * arm,
                0.0,
                0.0,
            ),
        ));
        o.rotations.push((
            "left_arm".into(),
            Vec3::new(-(swing * 0.6662).cos() * arm, 0.0, 0.0),
        ));
        o.rotations.push((
            "right_leg".into(),
            Vec3::new((swing * 0.6662).cos() * leg, 0.0, 0.0),
        ));
        o.rotations.push((
            "left_leg".into(),
            Vec3::new(
                (swing * 0.6662 + std::f32::consts::PI).cos() * leg,
                0.0,
                0.0,
            ),
        ));
    }
    match e.pose {
        Pose::Crouching => {
            o.rotations.push(("body".into(), Vec3::new(0.5, 0.0, 0.0)));
            o.rotations.push(("head".into(), Vec3::new(-0.5, 0.0, 0.0)));
        }
        Pose::Swimming | Pose::FallFlying | Pose::SpinAttack => {
            o.rotations.push((
                "body".into(),
                Vec3::new(std::f32::consts::FRAC_PI_2, 0.0, 0.0),
            ));
            o.rotations.push((
                "right_arm".into(),
                Vec3::new(-std::f32::consts::FRAC_PI_2, 0.0, 0.4),
            ));
            o.rotations.push((
                "left_arm".into(),
                Vec3::new(-std::f32::consts::FRAC_PI_2, 0.0, -0.4),
            ));
        }
        _ => {}
    }
    // Head aim, unless a pose already rotated it (swimming keeps the head up; vanilla tilts it
    // with the body there, which we approximate by skipping the aim).
    if !matches!(
        e.pose,
        Pose::Crouching | Pose::Swimming | Pose::FallFlying | Pose::SpinAttack
    ) {
        let mut rel = (e.head_yaw - e.yaw).to_radians();
        while rel > std::f32::consts::PI {
            rel -= std::f32::consts::TAU;
        }
        while rel < -std::f32::consts::PI {
            rel += std::f32::consts::TAU;
        }
        o.rotations.push((
            "head".into(),
            Vec3::new(
                e.pitch.to_radians(),
                rel.clamp(-HEAD_CLAMP, HEAD_CLAMP),
                0.0,
            ),
        ));
    }
    o
}
