//! Humanoid models built in code: the armour pieces (vanilla `HumanoidArmorLayer`'s
//! `HumanoidModel.createArmorMeshSet`), the elytra and the missing-item fallback model.

use super::*;

/// The humanoid armour model for one slot (vanilla `HumanoidModel.createArmorMeshSet` +
/// `ArmorModelSet`, the 64x32 armour texture layout). Built once per slot; the renderer draws it
/// with the equipment asset's layer texture and the entity's own pose, so armour animates with
/// the body. The outer deformation is 1.0, the inner (leggings) 0.5, and leg cubes 0.1 less.
pub(crate) fn humanoid_armor(slot: haste_core::model::EquipmentSlot) -> Arc<CubeModel> {
    use haste_core::model::EquipmentSlot;
    static MODELS: std::sync::OnceLock<[Arc<CubeModel>; 4]> = std::sync::OnceLock::new();
    let models = MODELS.get_or_init(|| {
        [
            EquipmentSlot::Head,
            EquipmentSlot::Chest,
            EquipmentSlot::Legs,
            EquipmentSlot::Feet,
        ]
        .map(build_armor_model)
    });
    let i = match slot {
        EquipmentSlot::Head => 0,
        EquipmentSlot::Chest => 1,
        EquipmentSlot::Legs => 2,
        EquipmentSlot::Feet => 3,
        _ => 1,
    };
    models[i].clone()
}

/// The armour model for one slot. `head`/`hat` (outer), `body`/arms (outer), `body`+legs
/// (inner) and the legs (outer), per `HumanoidModel.ADULT_ARMOR_PARTS_PER_SLOT`.
fn build_armor_model(slot: haste_core::model::EquipmentSlot) -> Arc<CubeModel> {
    use haste_core::model::EquipmentSlot;
    use haste_core::model::{Cube, ModelPart};
    let outer = 1.0f32;
    let inner = 0.5f32;
    let cube = |origin: [f32; 3], size: [f32; 3], uv: [u32; 2], inflate: f32, mirror: bool| Cube {
        origin: origin.into(),
        size: size.into(),
        inflate,
        uv,
        mirror,
    };
    let part =
        |name: &str, pivot: [f32; 3], cubes: Vec<Cube>, children: Vec<ModelPart>| ModelPart {
            name: name.to_owned(),
            id: parts::intern(name),
            pivot: pivot.into(),
            rotation: Vec3::ZERO,
            visible: true,
            cubes,
            children,
        };
    let head = || {
        part(
            "head",
            [0.0, -24.0, 0.0],
            vec![cube(
                [-4.0, -8.0, -4.0],
                [8.0, 8.0, 8.0],
                [0, 0],
                outer,
                false,
            )],
            vec![part(
                "hat",
                [0.0, 0.0, 0.0],
                vec![cube(
                    [-4.0, -8.0, -4.0],
                    [8.0, 8.0, 8.0],
                    [32, 0],
                    outer + 0.5,
                    false,
                )],
                Vec::new(),
            )],
        )
    };
    let body = |inflate: f32| {
        part(
            "body",
            [0.0, -24.0, 0.0],
            vec![cube(
                [-4.0, 0.0, -2.0],
                [8.0, 12.0, 4.0],
                [16, 16],
                inflate,
                false,
            )],
            Vec::new(),
        )
    };
    let arms = || {
        vec![
            part(
                "right_arm",
                [-5.0, -22.0, 0.0],
                vec![cube(
                    [-3.0, -2.0, -2.0],
                    [4.0, 12.0, 4.0],
                    [40, 16],
                    outer,
                    false,
                )],
                Vec::new(),
            ),
            part(
                "left_arm",
                [5.0, -22.0, 0.0],
                vec![cube(
                    [-1.0, -2.0, -2.0],
                    [4.0, 12.0, 4.0],
                    [40, 16],
                    outer,
                    true,
                )],
                Vec::new(),
            ),
        ]
    };
    let legs = |inflate: f32| {
        vec![
            part(
                "right_leg",
                [-1.9, -12.0, 0.0],
                vec![cube(
                    [-2.0, 0.0, -2.0],
                    [4.0, 12.0, 4.0],
                    [0, 16],
                    inflate,
                    false,
                )],
                Vec::new(),
            ),
            part(
                "left_leg",
                [1.9, -12.0, 0.0],
                vec![cube(
                    [-2.0, 0.0, -2.0],
                    [4.0, 12.0, 4.0],
                    [0, 16],
                    inflate,
                    true,
                )],
                Vec::new(),
            ),
        ]
    };
    let mut children = Vec::new();
    match slot {
        EquipmentSlot::Head => children.push(head()),
        EquipmentSlot::Chest => {
            children.push(body(outer));
            children.extend(arms());
        }
        EquipmentSlot::Legs => {
            children.push(body(inner));
            children.extend(legs(inner - 0.1));
        }
        EquipmentSlot::Feet => children.extend(legs(outer - 0.1)),
        _ => {}
    }
    Arc::new(CubeModel {
        texture: Identifier::new("haste", "entity/armor"),
        tex_width: 64,
        tex_height: 32,
        root: part("root", [0.0, 0.0, 0.0], Vec::new(), children),
    })
}

/// The elytra model (vanilla `ElytraModel.createLayer`): a left and a right wing, 64x32 texture.
/// In Haste's model space (y down, the head base at y = -24) the wings hang from the shoulders.
/// Built once.
pub(crate) fn elytra_model() -> Arc<CubeModel> {
    static MODEL: std::sync::OnceLock<Arc<CubeModel>> = std::sync::OnceLock::new();
    MODEL
        .get_or_init(|| {
            use haste_core::model::{Cube, ModelPart};
            let cube = |origin: [f32; 3], mirror: bool| Cube {
                origin: origin.into(),
                size: Vec3::new(10.0, 20.0, 2.0),
                inflate: 1.0,
                uv: [22, 0],
                mirror,
            };
            let wing = |name: &str, pivot: [f32; 3], rotation: [f32; 3], mirror: bool| ModelPart {
                name: name.to_owned(),
                id: parts::intern(name),
                pivot: pivot.into(),
                rotation: rotation.into(),
                visible: true,
                cubes: vec![cube(
                    if mirror {
                        [0.0, 0.0, 0.0]
                    } else {
                        [-10.0, 0.0, 0.0]
                    },
                    mirror,
                )],
                children: vec![],
            };
            let pi = std::f32::consts::PI;
            Arc::new(CubeModel {
                texture: Identifier::minecraft("entity/elytra"),
                tex_width: 64,
                tex_height: 32,
                root: ModelPart {
                    name: "root".into(),
                    id: parts::ROOT,
                    pivot: Vec3::ZERO,
                    rotation: Vec3::ZERO,
                    visible: true,
                    cubes: vec![],
                    children: vec![
                        wing(
                            "left_wing",
                            [5.0, -24.0, 0.0],
                            [pi / 12.0, 0.0, -pi / 12.0],
                            false,
                        ),
                        wing(
                            "right_wing",
                            [-5.0, -24.0, 0.0],
                            [pi / 12.0, 0.0, pi / 12.0],
                            true,
                        ),
                    ],
                },
            })
        })
        .clone()
}

/// The missing item model vanilla's `ModelBakery.MissingModels` bakes (`MissingCuboidModel`):
/// a unit cube with the missingno texture and no display transforms, so it draws centred on
/// the origin like any `NO_TRANSFORM` item. Built once; unknown items are rare (modded ids).
pub(super) fn missing_item_model() -> std::sync::Arc<haste_assets::ItemModel> {
    static MISSING: std::sync::OnceLock<std::sync::Arc<haste_assets::ItemModel>> =
        std::sync::OnceLock::new();
    MISSING
        .get_or_init(|| {
            let quads = [
                haste_assets::Direction::Down,
                haste_assets::Direction::Up,
                haste_assets::Direction::North,
                haste_assets::Direction::South,
                haste_assets::Direction::West,
                haste_assets::Direction::East,
            ]
            .into_iter()
            .map(|d| {
                let mut q =
                    haste_assets::BakedQuad::from_element(d, [0.0; 3], [1.0; 3], None, None);
                q.texture = "haste:missing".into();
                q
            })
            .collect();
            std::sync::Arc::new(haste_assets::ItemModel {
                parts: vec![haste_assets::ItemModelPart {
                    geometry: haste_assets::ItemGeometry::Elements(quads),
                    tints: vec![],
                    gui: haste_assets::DisplayTransform::IDENTITY,
                    firstperson: haste_assets::DisplayTransform::IDENTITY,
                    thirdperson: haste_assets::DisplayTransform::IDENTITY,
                    thirdperson_left: haste_assets::DisplayTransform::IDENTITY,
                    head: haste_assets::DisplayTransform::IDENTITY,
                    ground: haste_assets::DisplayTransform::IDENTITY,
                    fixed: haste_assets::DisplayTransform::IDENTITY,
                    on_shelf: haste_assets::DisplayTransform::IDENTITY,
                    gui_light_front: false,
                    transformation: None,
                }],
            })
        })
        .clone()
}

/// Vanilla's missing texture: 16x16 magenta/black checker of 8x8 squares.
pub(super) fn missing_texture() -> image::RgbaImage {
    image::RgbaImage::from_fn(16, 16, |x, y| {
        if (x < 8) ^ (y < 8) {
            image::Rgba([248, 0, 248, 255])
        } else {
            image::Rgba([0, 0, 0, 255])
        }
    })
}

/// `haste:white`: a plain white texel, so a vertex colour draws as itself (the leash rope).
pub(super) fn white_texture() -> image::RgbaImage {
    image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255]))
}

/// `haste:` placeholder texture: light grey with a darker 1 px border per 8 px cell.
pub(super) fn placeholder_texture() -> image::RgbaImage {
    image::RgbaImage::from_fn(64, 64, |x, y| {
        if x % 8 == 0 || y % 8 == 0 {
            image::Rgba([120, 120, 130, 255])
        } else {
            image::Rgba([200, 200, 210, 255])
        }
    })
}
