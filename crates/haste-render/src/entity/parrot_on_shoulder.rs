//! `ParrotOnShoulderLayer`: the parrots a player carries on its shoulders. Each parrot is the
//! parrot model, moved to its shoulder and posed with `ParrotModel.setupAnim`'s ON_SHOULDER
//! pose: the head follows the player's head, the wings are folded, and the tail sways with the
//! walk. The pose has no flap and no bob (the flap angle stays zero on a shoulder).

use glam::Vec3;
use haste_core::model::{CubeModel, PoseOverrides};

use super::stuck_in_body::find_part;

/// `ParrotOnShoulderLayer.submitOnShoulder`'s offset, in blocks, from the player's origin:
/// 0.4 to the side (left parrot +x, right -x), 1.5 down (1.3 while crouching), and no
/// forward offset. Returned in model units (1/16 block).
pub(crate) fn shoulder_offset(left: bool, crouching: bool) -> Vec3 {
    let side = if left { 0.4 } else { -0.4 };
    let drop = if crouching { -1.3 } else { -1.5 };
    Vec3::new(side, drop, 0.0) * 16.0
}

/// `ParrotRenderState` inputs for the ON_SHOULDER pose: the head angles (radians, the player's
/// head as the layer passes them), and the walk cycle's position and speed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ShoulderPose {
    pub head_pitch: f32,
    pub head_yaw: f32,
    pub walk_pos: f32,
    pub walk_speed: f32,
}

/// The wings' folded roll on the shoulder (`ParrotModel.setupAnim`), in radians.
const WING_ROLL: f32 = 0.0873;

/// `ParrotModel.setupAnim` for ON_SHOULDER, as pose overrides. Overrides add to each part's
/// base rotation, so the absolute angles (head, wings) are stored as the difference from the
/// base; the tail's walk sway is an added term.
pub(crate) fn shoulder_pose(model: &CubeModel, p: ShoulderPose) -> PoseOverrides {
    let mut pose = PoseOverrides::default();
    let base = |name: &str| find_part(&model.root, name).map(|part| (part.id, part.rotation));
    if let Some((id, rot)) = base("head") {
        pose.set_rotation(id, Vec3::new(p.head_pitch - rot.x, p.head_yaw - rot.y, 0.0));
    }
    if let Some((id, rot)) = base("left_wing") {
        pose.set_rotation(id, Vec3::new(0.0, 0.0, -WING_ROLL - rot.z));
    }
    if let Some((id, rot)) = base("right_wing") {
        pose.set_rotation(id, Vec3::new(0.0, 0.0, WING_ROLL - rot.z));
    }
    if let Some((id, _)) = base("tail") {
        let sway = (p.walk_pos * 0.6662).cos() * 0.3 * p.walk_speed;
        pose.set_rotation(id, Vec3::new(sway, 0.0, 0.0));
    }
    pose
}
