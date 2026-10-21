//! `LlamaSpitRenderer.submit`'s transform for the spit, expressed for the generic model path
//! (`minecraft:llama_spit#main`, one part): the lift before the yaw, then `rotY(yRot - 90)`
//! and `rotZ(xRot)`. The generic path flips the model with `rotZ(pi)` and turns it by
//! `180 - yaw`, so the spit's body op cancels the flip and the instance yaw is `270 - yRot`.
//! The layer is drawn about the entity's origin, so the loader keeps it unshifted
//! (`model_dump::UNSHIFTED_LAYERS`). `LlamaSpitRenderer` sets no shadow (radius 0).

use haste_core::math::Vec3;
use haste_core::model::{BodyOp, BodyTransform};

/// `translate(0, 0.15, 0)` before the spit's rotations, in blocks.
pub const LIFT: Vec3 = Vec3::new(0.0, 0.15, 0.0);

/// Whether a tracked entity is a `minecraft:llama_spit`.
pub fn is_llama_spit(kind: haste_core::RegistryIndex) -> bool {
    crate::registry::entity_type_name(kind) == Some("minecraft:llama_spit")
}

/// The instance yaw that makes the generic path turn the spit by `rotY(yRot - 90)`:
/// the path turns by `180 - yaw`, so `180 - (270 - yRot) = yRot - 90`.
pub fn instance_yaw(y_rot_deg: f32) -> f32 {
    270.0 - y_rot_deg
}

/// The body ops: `rotZ(xRot)` (degrees in vanilla) plus `pi`, which the generic path's flip
/// (`rotZ(pi)`, applied before the body ops) takes back out.
pub fn body(x_rot_deg: f32) -> BodyTransform {
    let mut b = BodyTransform::default();
    b.push(BodyOp::RotateZ(
        x_rot_deg.to_radians() + std::f32::consts::PI,
    ));
    b.pre_translate = LIFT;
    b
}
