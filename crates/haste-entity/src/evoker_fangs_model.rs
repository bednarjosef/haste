//! `EvokerFangsModel.setupAnim` and `EvokerFangsRenderer.submit`'s transform for the bite,
//! expressed for the generic model path: the jaws and the base's rise are pose overrides, the
//! root's scale is the body scale, and the root's height is a body op. The layer itself
//! (`minecraft:evoker_fangs#main`) comes from the model data.

use haste_core::math::Vec3;
use haste_core::model::{BodyOp, BodyTransform, PoseOverrides};
use haste_core::parts;

/// `EvokerFangsModel.setupAnim`'s values for one bite progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FangsPose {
    /// `upperJaw.zRot`, in radians.
    pub upper_jaw_z: f32,
    /// `lowerJaw.zRot`, in radians.
    pub lower_jaw_z: f32,
    /// The base's rise, added to its pivot's y in model pixels (y down, so it is negative).
    pub base_rise: f32,
    /// `preScale`: the root's uniform scale; it shrinks over the last tenth of the bite.
    pub pre_scale: f32,
}

/// `EvokerFangsModel.setupAnim(state)` for `progress` (`EvokerFangsRenderState.biteProgress`).
pub fn pose(progress: f32) -> FangsPose {
    let bite = 1.0 - (progress * 2.0).min(1.0).powi(3);
    let mut pre_scale = 1.0;
    if progress > 0.9 {
        pre_scale *= (1.0 - progress) / 0.1;
    }
    FangsPose {
        upper_jaw_z: std::f32::consts::PI - bite * 0.35 * std::f32::consts::PI,
        lower_jaw_z: std::f32::consts::PI + bite * 0.35 * std::f32::consts::PI,
        base_rise: -(progress + (progress * 2.7).sin()) * 7.2,
        pre_scale,
    }
}

/// The instance yaw that makes the generic path turn the fangs by `rotY(90 - yRot)`: the path
/// turns by `180 - yaw`, so `180 - (90 + yRot) = 90 - yRot`.
pub fn instance_yaw(y_rot_deg: f32) -> f32 {
    y_rot_deg + 90.0
}

/// The root's height in model pixels: `root.y = 24 - 20 * preScale`.
pub fn root_y(pre_scale: f32) -> f32 {
    24.0 - 20.0 * pre_scale
}

/// The pose overrides for the jaws and the base.
pub fn overrides(p: FangsPose) -> PoseOverrides {
    let mut o = PoseOverrides::default();
    o.set_rotation(
        parts::intern("upper_jaw"),
        Vec3::new(0.0, 0.0, p.upper_jaw_z),
    );
    o.set_rotation(
        parts::intern("lower_jaw"),
        Vec3::new(0.0, 0.0, p.lower_jaw_z),
    );
    o.set_offset(parts::intern("base"), Vec3::new(0.0, p.base_rise, 0.0));
    o
}

/// The whole-model transform. `EvokerFangsRenderer.submit` translates by -1.501 blocks before
/// its flip, which leaves the model 1.501 blocks up after it; the root's height (`rootY / 16`
/// blocks, pixels scaled) brings it back down, so the net lift is `1.501 - rootY / 16`.
pub fn body(p: FangsPose) -> BodyTransform {
    let mut b = BodyTransform::default();
    b.push(BodyOp::Translate(Vec3::new(
        0.0,
        1.501 - root_y(p.pre_scale) / 16.0,
        0.0,
    )));
    b
}
