// Skulls and heads (vanilla `SkullBlockRenderer`): the head kinds (`SkullBlock.Types`) and
// their blocks, the textures (`SKIN_BY_TYPE`), the layer models, the wall/ground transforms
// and the dragon jaw and piglin ear animations.

use std::sync::OnceLock;

use glam::{Mat4, Vec3};
use haste_core::model::{Cube, CubeModel, ModelPart, PoseOverrides};
use haste_core::BlockPos;
use haste_core::{parts, Direction, Identifier};

use super::{to_y_rot, BlockEntityInstance, BlockEntityKind, EmitPart};
use crate::entity::EntityVertex;

/// The head types vanilla renders (vanilla `SkullBlock.Types`); the block id picks one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkullKind {
    Skeleton,
    WitherSkeleton,
    Zombie,
    Creeper,
    Dragon,
    Piglin,
    Player,
}

/// The head kind and wall flag from a skull block id (vanilla `SkullBlock.Types`'s blocks:
/// `skeleton_skull`, `wither_skeleton_skull`, `zombie_head`, `creeper_head`, `dragon_head`,
/// `piglin_head`, `player_head`; the wall variants insert `_wall_`, e.g.
/// `creeper_wall_head`).
pub(super) fn skull_kind_of(block: &Identifier) -> Option<BlockEntityKind> {
    if block.namespace() != "minecraft" {
        return None;
    }
    let path = block.path();
    let (name, wall) = match path.split_once("_wall_") {
        // `skeleton_wall_skull` -> `skeleton_skull`, `creeper_wall_head` -> `creeper_head`.
        Some((head, tail)) => (format!("{head}_{tail}"), true),
        None => (path.to_string(), false),
    };
    let kind = match name.as_str() {
        "skeleton_skull" => SkullKind::Skeleton,
        "wither_skeleton_skull" => SkullKind::WitherSkeleton,
        "zombie_head" => SkullKind::Zombie,
        "creeper_head" => SkullKind::Creeper,
        "dragon_head" => SkullKind::Dragon,
        "piglin_head" => SkullKind::Piglin,
        "player_head" => SkullKind::Player,
        _ => return None,
    };
    Some(BlockEntityKind::Skull { kind, wall })
}

/// The texture a head kind is drawn with (vanilla `SkullBlockRenderer.SKIN_BY_TYPE`; the
/// player head uses the default skin, custom profiles are not modelled yet).
pub fn skull_texture(kind: SkullKind) -> Identifier {
    match kind {
        SkullKind::Skeleton => Identifier::minecraft("entity/skeleton/skeleton"),
        SkullKind::WitherSkeleton => Identifier::minecraft("entity/skeleton/wither_skeleton"),
        SkullKind::Zombie => Identifier::minecraft("entity/zombie/zombie"),
        SkullKind::Creeper => Identifier::minecraft("entity/creeper/creeper"),
        SkullKind::Dragon => Identifier::minecraft("entity/enderdragon/dragon"),
        SkullKind::Piglin => Identifier::minecraft("entity/piglin/piglin"),
        SkullKind::Player => Identifier::minecraft("entity/player/wide/steve"),
    }
}

/// The head models as data. Skeleton, wither skeleton and creeper share `SkullModel`'s
/// mob head layer (one cube, 64x32 texture); the player and zombie add the hat cube on the
/// humanoid head layer (64x64); the piglin head has its own part set with the ears (64x64);
/// the dragon head is scaled 0.75 with the jaw child (256x256).
pub(super) fn skull_model(kind: SkullKind) -> &'static CubeModel {
    static MOB: OnceLock<CubeModel> = OnceLock::new();
    static HUMANOID: OnceLock<CubeModel> = OnceLock::new();
    static PIGLIN: OnceLock<CubeModel> = OnceLock::new();
    static DRAGON: OnceLock<CubeModel> = OnceLock::new();
    let slot = match kind {
        SkullKind::Skeleton | SkullKind::WitherSkeleton | SkullKind::Creeper => &MOB,
        SkullKind::Zombie | SkullKind::Player => &HUMANOID,
        SkullKind::Piglin => &PIGLIN,
        SkullKind::Dragon => &DRAGON,
    };
    slot.get_or_init(|| {
        let part =
            |name: &str, pivot: Vec3, cubes: Vec<Cube>, children: Vec<ModelPart>| ModelPart {
                name: name.into(),
                id: parts::intern(name),
                pivot,
                rotation: Vec3::ZERO,
                visible: true,
                cubes,
                children,
            };
        let cube = |origin: Vec3, size: Vec3, uv: [u32; 2], inflate: f32| Cube {
            origin,
            size,
            inflate,
            uv,
            mirror: false,
        };
        match kind {
            // SkullModel.createMobHeadLayer: head only, 64x32.
            SkullKind::Skeleton | SkullKind::WitherSkeleton | SkullKind::Creeper => CubeModel {
                texture: Identifier::minecraft("entity/skeleton/skeleton"),
                tex_width: 64,
                tex_height: 32,
                root: part(
                    "root",
                    Vec3::ZERO,
                    Vec::new(),
                    vec![part(
                        "head",
                        Vec3::ZERO,
                        vec![cube(
                            Vec3::new(-4.0, -8.0, -4.0),
                            Vec3::new(8.0, 8.0, 8.0),
                            [0, 0],
                            0.0,
                        )],
                        Vec::new(),
                    )],
                ),
            },
            // SkullModel.createHumanoidHeadLayer: head + hat, 64x64.
            SkullKind::Zombie | SkullKind::Player => CubeModel {
                texture: Identifier::minecraft("entity/player/wide/steve"),
                tex_width: 64,
                tex_height: 64,
                root: part(
                    "root",
                    Vec3::ZERO,
                    Vec::new(),
                    vec![part(
                        "head",
                        Vec3::ZERO,
                        vec![
                            cube(
                                Vec3::new(-4.0, -8.0, -4.0),
                                Vec3::new(8.0, 8.0, 8.0),
                                [0, 0],
                                0.0,
                            ),
                            cube(
                                Vec3::new(-4.0, -8.0, -4.0),
                                Vec3::new(8.0, 8.0, 8.0),
                                [32, 0],
                                0.25,
                            ),
                        ],
                        Vec::new(),
                    )],
                ),
            },
            // AbstractPiglinModel.addHead: the head with snout and nostrils, the ears on
            // their pivots (their z rotation is `PiglinHeadModel.setupAnim`'s, applied per
            // instance — it replaces the layer's ±30-degree base), 64x64.
            SkullKind::Piglin => CubeModel {
                texture: Identifier::minecraft("entity/piglin/piglin"),
                tex_width: 64,
                tex_height: 64,
                root: part(
                    "root",
                    Vec3::ZERO,
                    Vec::new(),
                    vec![part(
                        "head",
                        Vec3::ZERO,
                        vec![
                            cube(
                                Vec3::new(-5.0, -8.0, -4.0),
                                Vec3::new(10.0, 8.0, 8.0),
                                [0, 0],
                                0.0,
                            ),
                            cube(
                                Vec3::new(-2.0, -4.0, -5.0),
                                Vec3::new(4.0, 4.0, 1.0),
                                [31, 1],
                                0.0,
                            ),
                            cube(
                                Vec3::new(2.0, -2.0, -5.0),
                                Vec3::new(1.0, 2.0, 1.0),
                                [2, 4],
                                0.0,
                            ),
                            cube(
                                Vec3::new(-3.0, -2.0, -5.0),
                                Vec3::new(1.0, 2.0, 1.0),
                                [2, 0],
                                0.0,
                            ),
                        ],
                        vec![
                            part(
                                "left_ear",
                                Vec3::new(4.5, -6.0, 0.0),
                                vec![cube(
                                    Vec3::new(0.0, 0.0, -2.0),
                                    Vec3::new(1.0, 5.0, 4.0),
                                    [51, 6],
                                    0.0,
                                )],
                                Vec::new(),
                            ),
                            part(
                                "right_ear",
                                Vec3::new(-4.5, -6.0, 0.0),
                                vec![cube(
                                    Vec3::new(-1.0, 0.0, -2.0),
                                    Vec3::new(1.0, 5.0, 4.0),
                                    [39, 6],
                                    0.0,
                                )],
                                Vec::new(),
                            ),
                        ],
                    )],
                ),
            },
            // DragonHeadModel.createHeadLayer: head at PartPose.offset(0, -7.986666, 0) scaled
            // 0.75, the jaw a child at (0, 4, -8), 256x256. The scale cubes are mirrored in
            // vanilla (mirror(true) before the first scale/nostril pair); Haste's Cube mirror
            // swaps the side UVs, which for these single-face boxes is the same drawing.
            SkullKind::Dragon => {
                let head_cubes = vec![
                    cube(
                        Vec3::new(-6.0, -1.0, -24.0),
                        Vec3::new(12.0, 5.0, 16.0),
                        [176, 44],
                        0.0,
                    ),
                    cube(
                        Vec3::new(-8.0, -8.0, -10.0),
                        Vec3::new(16.0, 16.0, 16.0),
                        [112, 30],
                        0.0,
                    ),
                    cube(
                        Vec3::new(-5.0, -12.0, -4.0),
                        Vec3::new(2.0, 4.0, 6.0),
                        [0, 0],
                        0.0,
                    ),
                    cube(
                        Vec3::new(-5.0, -3.0, -22.0),
                        Vec3::new(2.0, 2.0, 4.0),
                        [112, 0],
                        0.0,
                    ),
                    cube(
                        Vec3::new(3.0, -12.0, -4.0),
                        Vec3::new(2.0, 4.0, 6.0),
                        [0, 0],
                        0.0,
                    ),
                    cube(
                        Vec3::new(3.0, -3.0, -22.0),
                        Vec3::new(2.0, 2.0, 4.0),
                        [112, 0],
                        0.0,
                    ),
                ];
                CubeModel {
                    texture: Identifier::minecraft("entity/enderdragon/dragon"),
                    tex_width: 256,
                    tex_height: 256,
                    root: part(
                        "root",
                        Vec3::ZERO,
                        Vec::new(),
                        vec![part(
                            "head",
                            // `PartPose.offset(0, -7.986666, 0).scaled(0.75)`.
                            Vec3::new(0.0, -7.986_666 * 0.75, 0.0),
                            head_cubes,
                            vec![part(
                                "jaw",
                                Vec3::new(0.0, 4.0, -8.0),
                                vec![cube(
                                    Vec3::new(-6.0, 0.0, -16.0),
                                    Vec3::new(12.0, 4.0, 16.0),
                                    [176, 65],
                                    0.0,
                                )],
                                Vec::new(),
                            )],
                        )],
                    ),
                }
            }
        }
    })
}

/// Vanilla `SkullBlockRenderer.createWallTransformation` / `createGroundTransformation`:
/// wall: translate `(0.5 - stepX*0.25, 0.25, 0.5 - stepZ*0.25)`, rotate
/// `-opposite(facing).toYRot()`, scale (-1, -1, 1); ground: translate (0.5, 0, 0.5), rotate
/// `-RotationSegment.convertToDegrees(rotation)`, the same scale. The model renderer's 1/16
/// (pixels to blocks) is folded in after, like the other block entity models.
pub(super) fn skull_root(
    pos: BlockPos,
    kind_wall: bool,
    facing: Direction,
    rotation_segment: i32,
    camera: glam::DVec3,
) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    let flip = Mat4::from_scale(Vec3::new(-1.0 / 16.0, -1.0 / 16.0, 1.0 / 16.0));
    if kind_wall {
        // `facing` is the wall `facing` property; the head looks out of the wall, i.e. the
        // opposite direction's yaw.
        let opposite = facing.opposite();
        let (sx, sz) = (facing.step_x(), facing.step_z());
        Mat4::from_translation(
            p.as_vec3() + Vec3::new(0.5 - sx as f32 * 0.25, 0.25, 0.5 - sz as f32 * 0.25),
        ) * Mat4::from_rotation_y((-to_y_rot(opposite)).to_radians())
            * flip
    } else {
        // `RotationSegment.convertToDegrees`, wrapped from 180 to -180.
        let d = rotation_segment.rem_euclid(16) as f32 * 22.5;
        let d = if d >= 180.0 { d - 360.0 } else { d };
        Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.0, 0.5))
            * Mat4::from_rotation_y((-d).to_radians())
            * flip
    }
}

/// Vanilla `DragonHeadModel.setupAnim`'s jaw: `(sin(anim * PI * 0.2) + 1) * 0.2` radians
/// about x.
pub fn skull_dragon_jaw_rotation(anim: f32) -> f32 {
    ((anim * std::f32::consts::PI * 0.2).sin() + 1.0) * 0.2
}

/// Vanilla `PiglinHeadModel.setupAnim`'s ears: the left
/// `-(cos(anim * PI * 0.2 * 1.2) + 2.5) * 0.2` and the right
/// `(cos(anim * PI * 0.2) + 2.5) * 0.2` radians about z (replacing the layer's base ±30
/// degrees).
pub fn skull_piglin_ear_rotations(anim: f32) -> (f32, f32) {
    let left = -((anim * std::f32::consts::PI * 0.2 * 1.2).cos() + 2.5) * 0.2;
    let right = ((anim * std::f32::consts::PI * 0.2).cos() + 2.5) * 0.2;
    (left, right)
}
/// Poses one skull into `out` (the `emit_block_entity` skull arm): the model of its kind with
/// the dragon jaw or piglin ears animated.
pub(super) fn submit(
    out: &mut Vec<EntityVertex>,
    inst: &BlockEntityInstance,
    camera: glam::DVec3,
    rect: [u32; 4],
    light: (u8, u8),
    emit_part: EmitPart,
) {
    let BlockEntityKind::Skull { kind, wall } = inst.kind else {
        return;
    };
    let model = skull_model(kind);
    let mut pose = PoseOverrides::default();
    // The animation value: vanilla lerps `animationTickCount + partial`; the
    // tick's end value is used here (the chest lids' precedent).
    let anim = inst.skull_anim.end;
    match kind {
        SkullKind::Dragon => {
            // The jaw swings about its pivot.
            pose.set_rotation(
                parts::intern("jaw"),
                Vec3::new(skull_dragon_jaw_rotation(anim), 0.0, 0.0),
            );
        }
        SkullKind::Piglin => {
            let (left, right) = skull_piglin_ear_rotations(anim);
            pose.set_rotation(parts::intern("left_ear"), Vec3::new(0.0, 0.0, left));
            pose.set_rotation(parts::intern("right_ear"), Vec3::new(0.0, 0.0, right));
        }
        _ => {}
    }
    // The ground rotation angle rides in `sign_angle` (degrees, vanilla's
    // `RotationSegment.convertToDegrees`).
    let segment = (inst.sign_angle / 22.5).round() as i32;
    let root = skull_root(inst.pos, wall, inst.facing, segment, camera);
    for part in &model.root.children {
        emit_part(out, model, part, &pose, root, rect, light);
    }
}
