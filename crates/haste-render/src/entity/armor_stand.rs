//! The armour stand's armour models (vanilla `ArmorStandArmorModel.createArmorLayerSet`,
//! the `ARMOR_STAND_ARMOR` / `ARMOR_STAND_SMALL_ARMOR` layers): the humanoid armour parts per
//! slot (`HumanoidModel.ADULT_ARMOR_PARTS_PER_SLOT`) on the stand's own base mesh — head
//! pivot y = 1, leg pivots y = 11 — with the outer 1.0 / inner 0.5 deformations and the
//! legs' extra -0.1. The small set is the same through `HumanoidModel.BABY_TRANSFORMER`
//! (heads translate (0, 16, 0) and scale 3/4, the rest (0, 24, 0) and 1/2, around the ground
//! plane y = 24; the model carries no per-part scale, so it is baked into the geometry).
//!
//! Like the humanoid armour (`humanoid.rs`), these are renderer-side: vanilla's
//! `HumanoidArmorLayer` owns its model set, separate from the body model the store picked.
//! Small-ness is read off the body model (the store already chose it from the synced flag):
//! the small layer halves the base plate's pivot height.
use std::sync::{Arc, OnceLock};

use haste_core::math::Vec3;
use haste_core::model::{Cube, CubeModel, EquipmentSlot, ModelPart};
use haste_core::parts;
use haste_core::Identifier;

/// Vanilla's ground plane in model units (see `haste-entity`'s model space).
const GROUND_Y: f32 = 24.0;
/// The body model's base-plate pivot height in Haste space: -12 big, -6 small.
const BIG_BASE_PLATE_Y: f32 = -12.0;

/// A part with one cube (`CubeListBuilder.create().texOffs(uv).addBox(...)`, optional
/// mirror and `CubeDeformation`), at a vanilla `PartPose` pivot.
fn part(
    name: &str,
    pivot: [f32; 3],
    origin: [f32; 3],
    size: [f32; 3],
    uv: [u32; 2],
    mirror: bool,
    inflate: f32,
) -> ModelPart {
    ModelPart {
        name: name.to_owned(),
        id: parts::intern(name),
        pivot: pivot.into(),
        rotation: Vec3::ZERO,
        visible: true,
        cubes: vec![Cube {
            origin: origin.into(),
            size: size.into(),
            inflate,
            uv,
            mirror,
        }],
        children: Vec::new(),
    }
}

/// Shifts a root child's pivot from vanilla model space (ground at y = 24) into Haste's
/// (ground at y = 0); child pivots are parent-relative and stay.
fn grounded(mut p: ModelPart) -> ModelPart {
    p.pivot.y -= GROUND_Y;
    p
}

/// One root child of the small layer: the pivot through `PartPose.translated(0, lift, 0)
/// .scaled(s)` around the ground plane, the subtree geometry scaled by `s` (UVs untouched,
/// like vanilla's part scale).
fn small_layer_part(mut p: ModelPart) -> ModelPart {
    let (s, lift) = if p.id == parts::HEAD {
        (0.75, 16.0)
    } else {
        (0.5, 24.0)
    };
    p.pivot = Vec3::new(
        p.pivot.x * s,
        (p.pivot.y + GROUND_Y + lift) * s - GROUND_Y,
        p.pivot.z * s,
    );
    scale_subtree(&mut p, s);
    p
}

/// Scales a part's cubes and its children's pivots and cubes by `s` around the part's pivot.
fn scale_subtree(p: &mut ModelPart, s: f32) {
    for c in &mut p.cubes {
        c.origin *= s;
        c.size *= s;
        c.inflate *= s;
    }
    for child in &mut p.children {
        child.pivot *= s;
        scale_subtree(child, s);
    }
}

/// Whether `e`'s body model is an armour stand's, and whether it is the small one (`None`
/// for every other entity: players and mobs use the humanoid armour). The size follows the
/// body model, like vanilla's `ArmorStandRenderer.submit` picking its model pair. Called
/// once per entity per frame, off the hot per-slot path.
pub(super) fn small_of(body: &CubeModel) -> Option<bool> {
    if body.texture != Identifier::minecraft("entity/armorstand/armorstand") {
        return None;
    }
    Some(base_plate(&body.root).is_some_and(|p| p.pivot.y > BIG_BASE_PLATE_Y))
}

/// The armour shape for `slot` (`armor_for(slot, small_of(&e.model)?)`).
pub(super) fn armor_for(slot: EquipmentSlot, small: bool) -> Arc<CubeModel> {
    model_for(slot, small)
}

/// Finds the body model's base plate (both sizes carry one; only its height differs). The
/// part id is interned once, not per frame.
fn base_plate(part: &ModelPart) -> Option<&ModelPart> {
    fn find(part: &ModelPart, id: haste_core::parts::PartId) -> Option<&ModelPart> {
        if part.id == id {
            return Some(part);
        }
        part.children.iter().find_map(|c| find(c, id))
    }
    static ID: OnceLock<haste_core::parts::PartId> = OnceLock::new();
    find(part, *ID.get_or_init(|| parts::intern("base_plate")))
}

/// The shared armour shapes: one per slot, big and small (`ArmorModelSet`).
fn model_for(slot: EquipmentSlot, small: bool) -> Arc<CubeModel> {
    static MODELS: OnceLock<[[Arc<CubeModel>; 4]; 2]> = OnceLock::new();
    let models = MODELS.get_or_init(|| {
        [false, true].map(|small| {
            [
                EquipmentSlot::Head,
                EquipmentSlot::Chest,
                EquipmentSlot::Legs,
                EquipmentSlot::Feet,
            ]
            .map(|slot| build(slot, small))
        })
    });
    let i = match slot {
        EquipmentSlot::Head => 0,
        EquipmentSlot::Legs => 2,
        EquipmentSlot::Feet => 3,
        EquipmentSlot::Chest => 1,
        _ => 1,
    };
    models[usize::from(small)][i].clone()
}

/// One armour shape (`HumanoidModel.ADULT_ARMOR_PARTS_PER_SLOT` picks the parts per slot).
fn build(slot: EquipmentSlot, small: bool) -> Arc<CubeModel> {
    // `LayerDefinitions`: outer 1.0, inner (leggings) 0.5, legs always -0.1 less.
    let (outer, inner) = (1.0f32, 0.5f32);
    let head = || {
        let mut head = part(
            "head",
            [0.0, 1.0, 0.0],
            [-4.0, -8.0, -4.0],
            [8.0, 8.0, 8.0],
            [0, 0],
            false,
            outer,
        );
        let hat = part(
            "hat",
            [0.0, 0.0, 0.0],
            [-4.0, -8.0, -4.0],
            [8.0, 8.0, 8.0],
            [32, 0],
            false,
            outer + 0.5,
        );
        head.children.push(hat);
        head
    };
    let body = |inflate: f32| {
        part(
            "body",
            [0.0, 0.0, 0.0],
            [-4.0, 0.0, -2.0],
            [8.0, 12.0, 4.0],
            [16, 16],
            false,
            inflate,
        )
    };
    let arm = |name: &str, pivot: [f32; 3], origin: [f32; 3], mirror: bool| {
        part(
            name,
            pivot,
            origin,
            [4.0, 12.0, 4.0],
            [40, 16],
            mirror,
            outer,
        )
    };
    let leg = |name: &str, pivot: [f32; 3], mirror: bool, inflate: f32| {
        part(
            name,
            pivot,
            [-2.0, 0.0, -2.0],
            [4.0, 12.0, 4.0],
            [0, 16],
            mirror,
            inflate,
        )
    };
    let legs = |inflate: f32| {
        vec![
            leg("right_leg", [-1.9, 11.0, 0.0], false, inflate),
            leg("left_leg", [1.9, 11.0, 0.0], true, inflate),
        ]
    };
    let children: Vec<ModelPart> = match slot {
        EquipmentSlot::Head => vec![head()],
        EquipmentSlot::Chest => vec![
            body(outer),
            arm("right_arm", [-5.0, 2.0, 0.0], [-3.0, -2.0, -2.0], false),
            arm("left_arm", [5.0, 2.0, 0.0], [-1.0, -2.0, -2.0], true),
        ],
        EquipmentSlot::Legs => {
            let mut c = vec![body(inner)];
            c.extend(legs(inner - 0.1));
            c
        }
        EquipmentSlot::Feet => legs(outer - 0.1),
        _ => Vec::new(),
    };
    let children = children.into_iter().map(grounded).collect::<Vec<_>>();
    let children = if small {
        children.into_iter().map(small_layer_part).collect()
    } else {
        children
    };
    Arc::new(CubeModel {
        texture: Identifier::new("haste", "entity/armor"),
        tex_width: 64,
        tex_height: 32,
        root: ModelPart {
            name: "root".to_owned(),
            id: parts::ROOT,
            pivot: Vec3::ZERO,
            rotation: Vec3::ZERO,
            visible: true,
            cubes: Vec::new(),
            children,
        },
    })
}

#[cfg(test)]
mod tests;
