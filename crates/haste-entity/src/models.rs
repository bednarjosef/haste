//! Entity models as data (`haste_core::model::CubeModel`). Task 2 adds the humanoid; every other
//! type gets the generic placeholder box textured with `haste:entity_box`.
use std::sync::Arc;

use haste_core::model::{Cube, CubeModel, ModelPart};
use haste_core::Identifier;

/// The generic placeholder: one 1x1x1-block cube, scaled per entity by height at draw time.
pub fn generic_box() -> Arc<CubeModel> {
    static BOX: std::sync::OnceLock<Arc<CubeModel>> = std::sync::OnceLock::new();
    BOX.get_or_init(|| {
        Arc::new(CubeModel {
            texture: Identifier::new("haste", "entity_box"),
            tex_width: 64,
            tex_height: 64,
            root: ModelPart {
                name: "root".into(),
                pivot: glam::Vec3::ZERO,
                rotation: glam::Vec3::ZERO,
                visible: true,
                cubes: vec![Cube {
                    origin: glam::Vec3::new(-8.0, 0.0, -8.0),
                    size: glam::Vec3::new(16.0, 16.0, 16.0),
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
