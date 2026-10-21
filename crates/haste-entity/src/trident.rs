//! Tridents (`minecraft:trident`): vanilla `ThrownTridentRenderer.submit`'s pose. The trident
//! turns with its interpolated yaw and pitch: `rotateDegrees(YP, yRot - 90)` then
//! `rotateDegrees(ZP, xRot + 90)`, and the model is drawn with no further transform.
//!
//! Haste draws every non-living model through the generic entity path, which also applies the
//! `scale(-1, -1, 1)` flip vanilla's living-entity renderers use. The trident's vanilla pose has
//! no such flip, so [`body_transform`] adds a half turn about Z that cancels it. Nor does it have
//! the living renderers' -1.501 drop, so the model library loads `minecraft:trident#main`
//! without the ground shift (`model_dump::UNSHIFTED_LAYERS`): the model's origin is the entity's
//! position, the tines below it and the shaft above, along the shaft's own axis.

use std::f32::consts::PI;

use haste_core::model::{BodyOp, BodyTransform};

/// Whether `kind` is a trident we render (`minecraft:trident`). The kind id resolves from the
/// registry once (this runs per entity per frame, so it must not look the name up every time).
pub fn is_trident(kind: haste_core::RegistryIndex) -> bool {
    use std::sync::OnceLock;
    static KIND: OnceLock<(usize, Option<usize>)> = OnceLock::new();
    let (len, trident) = *KIND.get_or_init(|| {
        let names = crate::registry::entity_type_names();
        let trident = names.and_then(|n| {
            n.iter()
                .find(|(_, v)| **v == "minecraft:trident")
                .map(|(k, _)| k.0 as usize)
        });
        (names.map_or(0, |n| n.len()), trident)
    });
    let i = kind.0 as usize;
    // Without the registry (no cache) nothing is a trident, like the name lookup would answer.
    i < len && Some(i) == trident
}

/// `ThrownTridentRenderer.submit`'s rotations as a body transform: `yRot - 90` about Y, then
/// `xRot + 90` about Z, in degrees from the interpolated `yaw` and `pitch`. The yaw is not
/// applied again (`suppress_yaw`), and the extra half turn about Z (`PI`) undoes the generic
/// path's `scale(-1, -1, 1)` flip, so the model's vertices land where vanilla puts them.
pub fn body_transform(yaw: f32, pitch: f32) -> BodyTransform {
    let mut b = BodyTransform {
        suppress_yaw: true,
        ..BodyTransform::default()
    };
    b.push(BodyOp::RotateY((yaw - 90.0).to_radians()));
    b.push(BodyOp::RotateZ((pitch + 90.0).to_radians() + PI));
    b
}
