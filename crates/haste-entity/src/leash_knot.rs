//! `LeashKnotModel`: the fence knot's single cube (`LeashKnotModel.createBodyLayer`), code-built
//! like the armour stand's models. Its render origin is the knot's box centre (`crate::leash`).

use std::sync::{Arc, OnceLock};

use haste_core::model::{Cube, CubeModel, ModelPart};
use haste_core::{parts, Identifier, Vec3};

/// The knot's model: a 6 x 8 x 6 cube on a 32 x 32 texture, its origin at the box centre
/// (`PartPose.ZERO`, the cube runs from -3 to 3 across and from -8 to 0 up in model units).
pub(crate) fn model() -> Arc<CubeModel> {
    static MODEL: OnceLock<Arc<CubeModel>> = OnceLock::new();
    MODEL
        .get_or_init(|| {
            Arc::new(CubeModel {
                texture: Identifier::minecraft("entity/lead_knot/lead_knot"),
                tex_width: 32,
                tex_height: 32,
                root: ModelPart {
                    name: "root".to_owned(),
                    id: parts::ROOT,
                    pivot: Vec3::ZERO,
                    rotation: Vec3::ZERO,
                    visible: true,
                    cubes: vec![Cube {
                        origin: Vec3::new(-3.0, -8.0, -3.0),
                        size: Vec3::new(6.0, 8.0, 6.0),
                        inflate: 0.0,
                        uv: [0, 0],
                        mirror: false,
                    }],
                    children: Vec::new(),
                },
            })
        })
        .clone()
}
