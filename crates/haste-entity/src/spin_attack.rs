//! `SpinAttackEffectLayer`: the two rotating boxes drawn over a player while
//! `LivingEntity.isAutoSpinAttack` (the riptide spin, living flag 4). Vanilla's
//! `SpinAttackEffectModel` is built here in code rather than taken from the model dump:
//! `model_dump` folds a part's scale into the cube's inflation, and each box here needs its own
//! scale (haste-core's `ModelPart` has none, so the scale is multiplied into the cube's origin
//! and size instead).
//!
//! The layer is an overlay in slot 1 (slot 0 is the cape): [`dress`] adds it and sets the boxes'
//! yaw for the entity's age, so it turns with the player's own ticks.
use std::sync::{Arc, OnceLock};

use glam::Vec3;
use haste_core::model::{Cube, CubeModel, EntityInstance, EntityOverlay, ModelPart, PoseOverrides};
use haste_core::parts::{self, PartId};
use haste_core::Identifier;

use crate::metadata::Metadata;

/// `SpinAttackEffectLayer.TEXTURE`.
pub const TEXTURE: &str = "textures/entity/trident/trident_riptide.png";

/// The number of boxes in `SpinAttackEffectModel`.
const BOXES: usize = 2;

/// `SpinAttackEffectModel.createLayer`: box `i`'s scale, `0.75 * (i + 1)`.
pub fn box_scale(i: usize) -> f32 {
    0.75 * (i + 1) as f32
}

/// `SpinAttackEffectModel.createLayer`: box `i`'s y offset before its scale, `-3.2 + 9.6 * (i + 1)`.
pub fn box_y_offset(i: usize) -> f32 {
    -3.2 + 9.6 * (i + 1) as f32
}

/// `SpinAttackEffectModel.setupAnim`: box `i`'s yaw in radians at `age` (the entity's
/// `ageInTicks`): `wrapDegrees(age * -(45 + (i + 1) * 5))`.
pub fn box_yaw(i: usize, age: f32) -> f32 {
    let angle = age * -(45.0 + (i + 1) as f32 * 5.0);
    crate::store::wrap_degrees(angle).to_radians()
}

/// The part ids of the two boxes, interned once.
fn box_ids() -> &'static [PartId; BOXES] {
    static IDS: OnceLock<[PartId; BOXES]> = OnceLock::new();
    IDS.get_or_init(|| std::array::from_fn(|i| parts::intern(&box_name(i))))
}

fn box_name(i: usize) -> String {
    format!("box{i}")
}

/// The layer model: the root holds the boxes directly, each one cube at vanilla's origin
/// (`-8`, `-16 + yOffset`, `-8`) and size (16, 32, 16), both multiplied by the box's scale.
fn build_layer() -> CubeModel {
    let children = (0..BOXES)
        .map(|i| {
            let s = box_scale(i);
            ModelPart {
                name: box_name(i),
                id: box_ids()[i],
                pivot: Vec3::ZERO,
                rotation: Vec3::ZERO,
                visible: true,
                cubes: vec![Cube {
                    origin: Vec3::new(-8.0, -16.0 + box_y_offset(i), -8.0) * s,
                    size: Vec3::new(16.0, 32.0, 16.0) * s,
                    inflate: 0.0,
                    uv: [0, 0],
                    mirror: false,
                }],
                children: Vec::new(),
            }
        })
        .collect();
    CubeModel {
        texture: Identifier::minecraft(TEXTURE),
        tex_width: 64,
        tex_height: 64,
        root: ModelPart {
            name: "root".to_owned(),
            id: parts::ROOT,
            pivot: Vec3::ZERO,
            rotation: Vec3::ZERO,
            visible: true,
            cubes: Vec::new(),
            children,
        },
    }
}

/// The shared layer model, built on first use; every spinning player draws the same one.
pub fn layer() -> Arc<CubeModel> {
    static LAYER: OnceLock<Arc<CubeModel>> = OnceLock::new();
    LAYER.get_or_init(|| Arc::new(build_layer())).clone()
}

/// `SpinAttackEffectModel.setupAnim`: sets each box's yaw for `age` in the pose.
pub fn pose(pose: &mut PoseOverrides, age: f32) {
    for (i, id) in box_ids().iter().enumerate() {
        pose.set_rotation(*id, Vec3::new(0.0, box_yaw(i, age), 0.0));
    }
}

/// `SpinAttackEffectLayer.submit`: while the player is auto-spinning, adds the layer as overlay
/// slot 1 and sets its boxes' yaw for `age` (`ageInTicks`, the tick count plus the partial tick).
/// Nothing changes otherwise.
pub fn dress(inst: &mut EntityInstance, metadata: &Metadata, age: f32) {
    if !metadata.living.auto_spin_attack {
        return;
    }
    pose(&mut inst.pose, age);
    inst.overlays[1] = Some(EntityOverlay {
        model: layer(),
        tint: [1.0; 3],
    });
}
