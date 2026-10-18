//! Entity models as data (`haste_core::model::CubeModel`).
//!
//! Model-space convention for everything this crate emits (renderer renders it):
//! - units are 1/16 block (vanilla model units);
//! - y increases downward (vanilla `PartPose` convention);
//! - the feet sit on the plane y = 0, so a player's head top is at y = -32;
//! - a child part's pivot is relative to its parent's (vanilla `PartPose`), so overlay layers
//!   (hat, jacket, sleeves, pants) sit at [0, 0, 0] under their base part;
//! - parts rotate around their pivot in x, y, z order; `PoseOverrides` rotations are ADDITIVE to
//!   the part's base rotation. The renderer rotates the part named "head" by
//!   (head_yaw - yaw) around Y and pitch around X; the rest of the body follows `yaw`.
//!
//! The humanoid is the standard 64x64 skin layout (public knowledge; the UV offsets are in the
//! entities brief). The player model is authored 32 units tall and rendered at scale 0.9375 (vanilla's
//! player-model scale), i.e. 1.875 blocks against the 1.8 hitbox. Every other entity type gets a
//! generic box built from its dimensions, textured with the `haste:entity_box` placeholder.
use std::sync::{Arc, Mutex, OnceLock};

use ahash::AHashMap;
use haste_core::model::{Cube, CubeModel, ModelPart};
use haste_core::Identifier;

fn part(name: &str, pivot: [f32; 3], cubes: Vec<Cube>, children: Vec<ModelPart>) -> ModelPart {
    ModelPart {
        name: name.to_owned(),
        id: haste_core::parts::intern(name),
        pivot: pivot.into(),
        rotation: glam::Vec3::ZERO,
        visible: true,
        cubes,
        children,
    }
}

fn cube(origin: [f32; 3], size: [f32; 3], uv: [u32; 2], inflate: f32) -> Cube {
    Cube {
        origin: origin.into(),
        size: size.into(),
        inflate,
        uv,
        mirror: false,
    }
}

/// The humanoid/player model. `slim` selects the 3-pixel-wide arm layout (Alex-style skins);
/// otherwise the classic 4-pixel arms (Steve layout).
pub fn humanoid(slim: bool) -> Arc<CubeModel> {
    static MODELS: OnceLock<(Arc<CubeModel>, Arc<CubeModel>)> = OnceLock::new();
    let (classic, slim_model) = MODELS.get_or_init(|| {
        let build = move |slim: bool| {
            // Arm width and right-arm x origin: classic arms are 4 wide at -3, slim are 3 at -2
            // (the left arm mirrors around x = 0, so its origin is symmetric).
            let (aw, rao) = if slim { (3.0, -2.0) } else { (4.0, -3.0) };
            let uv_arm_r: [u32; 2] = [40, 16];
            let uv_arm_l: [u32; 2] = [32, 48];
            let uv_sleeve_r: [u32; 2] = [40, 32];
            let uv_sleeve_l: [u32; 2] = [48, 48];
            let root = part(
                "root",
                [0.0, 0.0, 0.0],
                Vec::new(),
                vec![
                    part(
                        "head",
                        [0.0, -24.0, 0.0],
                        vec![cube([-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], [0, 0], 0.0)],
                        vec![part(
                            "hat",
                            [0.0, 0.0, 0.0],
                            vec![cube([-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], [32, 0], 0.5)],
                            Vec::new(),
                        )],
                    ),
                    part(
                        "body",
                        [0.0, -24.0, 0.0],
                        vec![cube([-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], [16, 16], 0.0)],
                        vec![part(
                            "jacket",
                            [0.0, 0.0, 0.0],
                            vec![cube([-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], [16, 32], 0.25)],
                            Vec::new(),
                        )],
                    ),
                    part(
                        "right_arm",
                        [-5.0, -22.0, 0.0],
                        vec![cube([rao, -2.0, -2.0], [aw, 12.0, 4.0], uv_arm_r, 0.0)],
                        vec![part(
                            "right_sleeve",
                            [0.0, 0.0, 0.0],
                            vec![cube([rao, -2.0, -2.0], [aw, 12.0, 4.0], uv_sleeve_r, 0.25)],
                            Vec::new(),
                        )],
                    ),
                    part(
                        "left_arm",
                        [5.0, -22.0, 0.0],
                        vec![cube([-1.0, -2.0, -2.0], [aw, 12.0, 4.0], uv_arm_l, 0.0)],
                        vec![part(
                            "left_sleeve",
                            [0.0, 0.0, 0.0],
                            vec![cube([-1.0, -2.0, -2.0], [aw, 12.0, 4.0], uv_sleeve_l, 0.25)],
                            Vec::new(),
                        )],
                    ),
                    part(
                        "right_leg",
                        [-1.9, -12.0, 0.0],
                        vec![cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0, 16], 0.0)],
                        vec![part(
                            "right_pants",
                            [0.0, 0.0, 0.0],
                            vec![cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0, 32], 0.25)],
                            Vec::new(),
                        )],
                    ),
                    part(
                        "left_leg",
                        [1.9, -12.0, 0.0],
                        vec![cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [16, 48], 0.0)],
                        vec![part(
                            "left_pants",
                            [0.0, 0.0, 0.0],
                            vec![cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0, 48], 0.25)],
                            Vec::new(),
                        )],
                    ),
                ],
            );
            Arc::new(CubeModel {
                texture: Identifier::new("haste", "entity/humanoid"),
                tex_width: 64,
                tex_height: 64,
                root,
            })
        };
        let _ = build;
        (build(false), build(true))
    });
    if slim {
        slim_model.clone()
    } else {
        classic.clone()
    }
}

/// A generic box model from entity dimensions (blocks), cached per quantized size (1/16 units).
/// Cold path: called on spawn and dimension (pose) changes, never per frame.
type BoxCache = Mutex<AHashMap<(u16, u16), Arc<CubeModel>>>;
pub fn box_for(width: f32, height: f32) -> Arc<CubeModel> {
    static CACHE: OnceLock<BoxCache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(AHashMap::new()));
    let w = ((width * 16.0).round() as i16).max(1) as u16;
    let h = ((height * 16.0).round() as i16).max(1) as u16;
    let mut guard = cache.lock().unwrap();
    guard
        .entry((w, h))
        .or_insert_with(|| {
            let hw = f32::from(w) * 0.5;
            let (wf, hf) = (f32::from(w), f32::from(h));
            Arc::new(CubeModel {
                texture: Identifier::new("haste", "entity_box"),
                tex_width: 64,
                tex_height: 64,
                root: part(
                    "root",
                    [0.0, 0.0, 0.0],
                    vec![cube([-hw, -hf, -hw], [wf, hf, wf], [0, 0], 0.0)],
                    Vec::new(),
                ),
            })
        })
        .clone()
}

/// Whether a model has a part with this id (humanoid models carry `right_arm`, which only the
/// armed/armoured layers need).
pub(crate) fn model_has_part(
    part: &haste_core::model::ModelPart,
    id: haste_core::parts::PartId,
) -> bool {
    part.id == id || part.children.iter().any(|c| model_has_part(c, id))
}
