// Shulker boxes (vanilla `ShulkerBoxRenderer`): the colour from the block id
// (`ShulkerBoxBlock.getColor`), the texture (`Sheets.getShulkerBoxSprite`), the model
// (`ShulkerModel.createBoxLayer`, already in entity convention: y down, the ground at 24) and
// the lid's rise and spin from `ShulkerBoxBlockEntity.getProgress`.

use std::sync::OnceLock;

use glam::{Mat4, Vec3};
use haste_core::model::{Cube, CubeModel, ModelPart, PoseOverrides};
use haste_core::{parts, BlockPos, Direction, Identifier};

use super::{BlockEntityInstance, EmitPart};
use crate::entity::EntityVertex;

/// The dye colour id of a shulker box block id (`minecraft:white_shulker_box` -> 0), vanilla
/// `DyeColor`'s order; `None` for the plain `minecraft:shulker_box` (vanilla `getColor`'s null).
pub(super) fn shulker_color(block: &Identifier) -> Option<u8> {
    const NAMES: [&str; 16] = [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ];
    let path = block.path();
    let color = path.strip_suffix("_shulker_box")?;
    NAMES.iter().position(|n| *n == color).map(|i| i as u8)
}

/// The texture a shulker box is drawn with (vanilla `Sheets.getShulkerBoxSprite`:
/// `entity/shulker/shulker_<color>`, the undyed `entity/shulker/shulker` for `None`).
pub fn shulker_texture(color: Option<u8>) -> &'static Identifier {
    const NAMES: [&str; 16] = [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ];
    static TABLE: OnceLock<[Identifier; 17]> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        std::array::from_fn(|idx| {
            if idx == 0 {
                Identifier::minecraft("entity/shulker/shulker")
            } else {
                Identifier::minecraft(&format!("entity/shulker/shulker_{}", NAMES[idx - 1]))
            }
        })
    });
    match color {
        None => &table[0],
        Some(i) => &table[i as usize + 1],
    }
}

/// Vanilla `ShulkerModel.createBoxLayer`: lid and base as separate parts at pivot (0, 24, 0),
/// entity convention (y down, ground at 24). The lid's rise is applied per instance (vanilla
/// `ShulkerBoxRenderer.ShulkerBoxModel.setupAnim` moves the pivot to `24 - progress * 8`).
fn shulker_model() -> &'static CubeModel {
    static MODEL: OnceLock<CubeModel> = OnceLock::new();
    MODEL.get_or_init(|| CubeModel {
        texture: Identifier::minecraft("entity/shulker/shulker"),
        tex_width: 64,
        tex_height: 64,
        root: ModelPart {
            name: "root".into(),
            id: parts::ROOT,
            pivot: Vec3::ZERO,
            rotation: Vec3::ZERO,
            visible: true,
            cubes: Vec::new(),
            children: vec![
                // lid: addBox(-8, -16, -8, 16, 12, 16), texOffs(0, 0)
                ModelPart {
                    name: "lid".into(),
                    id: parts::LID,
                    pivot: Vec3::new(0.0, 24.0, 0.0),
                    rotation: Vec3::ZERO,
                    visible: true,
                    cubes: vec![Cube {
                        origin: Vec3::new(-8.0, -16.0, -8.0),
                        size: Vec3::new(16.0, 12.0, 16.0),
                        inflate: 0.0,
                        uv: [0, 0],
                        mirror: false,
                    }],
                    children: Vec::new(),
                },
                // base: addBox(-8, -8, -8, 16, 8, 16), texOffs(0, 28)
                ModelPart {
                    name: "base".into(),
                    id: parts::intern("base"),
                    pivot: Vec3::new(0.0, 24.0, 0.0),
                    rotation: Vec3::ZERO,
                    visible: true,
                    cubes: vec![Cube {
                        origin: Vec3::new(-8.0, -8.0, -8.0),
                        size: Vec3::new(16.0, 8.0, 16.0),
                        inflate: 0.0,
                        uv: [0, 28],
                        mirror: false,
                    }],
                    children: Vec::new(),
                },
            ],
        },
    })
}

/// Vanilla `Direction.getRotation` (the shulker transform's facing rotation): JOML's
/// `rotationXYZ(x, 0, z)` composites X * Z on column vectors.
fn direction_rotation(d: Direction) -> Mat4 {
    let x = |a: f32| Mat4::from_rotation_x(a.to_radians());
    let z = |a: f32| Mat4::from_rotation_z(a.to_radians());
    match d {
        Direction::Up => Mat4::IDENTITY,
        Direction::Down => x(180.0),
        Direction::North => x(90.0) * z(180.0),
        Direction::South => x(90.0),
        Direction::West => x(90.0) * z(90.0),
        Direction::East => x(90.0) * z(-90.0),
    }
}

/// Vanilla `ShulkerBoxRenderer.createModelTransform`:
/// `translation(0.5, 0.5, 0.5) * scale(0.9995) * direction.getRotation() * scale(1, -1, -1)
/// * translate(0, -1, 0)` on the model's block-unit vertices, here acting on model units
/// (the `/16` folded in before the translate, the flip after it).
fn shulker_root(pos: BlockPos, facing: Direction, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3() + Vec3::splat(0.5))
        * Mat4::from_scale(Vec3::splat(0.9995))
        * direction_rotation(facing)
        * Mat4::from_scale(Vec3::new(1.0, -1.0, -1.0))
        * Mat4::from_translation(Vec3::new(0.0, -1.0, 0.0))
        * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}

/// Poses one shulker box into `out` (the `emit_block_entity` shulker arm): the lid spins by
/// 270 degrees * progress and rises with it.
pub(super) fn submit(
    out: &mut Vec<EntityVertex>,
    inst: &BlockEntityInstance,
    camera: glam::DVec3,
    rect: [u32; 4],
    light: (u8, u8),
    emit_part: EmitPart,
) {
    let model = shulker_model();
    let progress = inst.open.end.max(inst.open.start);
    let mut pose = PoseOverrides::default();
    // Vanilla `ShulkerBoxModel.setupAnim`: the lid spins by 270 degrees * progress...
    pose.set_rotation(
        parts::LID,
        Vec3::new(0.0, 270.0_f32.to_radians() * progress, 0.0),
    );
    let root = shulker_root(inst.pos, inst.facing, camera);
    for part in &model.root.children {
        let parent = if part.id == parts::LID {
            // ...and rises by `progress * 0.5 * 16` model units (the pivot moves from
            // 24 down to 16).
            root * Mat4::from_translation(Vec3::new(0.0, -8.0 * progress, 0.0))
        } else {
            root
        };
        emit_part(out, model, part, &pose, parent, rect, light);
    }
}
