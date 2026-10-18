//! The crouch branch of vanilla `HumanoidModel.setupAnim` (`state.isCrouching`): the body
//! leans forward, the arms follow it, and the head, body and arms drop while the legs move
//! back, so the bent-over body stays joined to the legs.

use haste_core::math::Vec3;
use haste_core::model::PoseOverrides;
use haste_core::parts;

/// Leans the body and arms (`body`, `ra`, `la` are the setupAnim rotations so far) and sets the
/// crouch's pivot offsets, in model units (y down).
pub(super) fn pose(o: &mut PoseOverrides, body: &mut Vec3, ra: &mut Vec3, la: &mut Vec3) {
    body.x = 0.5;
    ra.x += 0.4;
    la.x += 0.4;
    o.set_offset(parts::RIGHT_LEG, Vec3::new(0.0, 0.0, 4.0));
    o.set_offset(parts::LEFT_LEG, Vec3::new(0.0, 0.0, 4.0));
    o.set_offset(parts::HEAD, Vec3::new(0.0, 4.2, 0.0));
    o.set_offset(parts::BODY, Vec3::new(0.0, 3.2, 0.0));
    o.set_offset(parts::LEFT_ARM, Vec3::new(0.0, 3.2, 0.0));
    o.set_offset(parts::RIGHT_ARM, Vec3::new(0.0, 3.2, 0.0));
}
