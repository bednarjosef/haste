// Block entity drawing: chests, ender chests, trapped chests and shulker boxes posed on the CPU
// into the entity renderer's vertex stream (one draw call for all of them, like entities).
//
// Follows vanilla's renderers (26.3 decompiled source, `net.minecraft.client.renderer.blockentity`):
// `ChestRenderer` (model transformation per facing, the lid angle curve, `Sheets.chooseSprite`'s
// texture per material and chest half, the Christmas rule from `SpecialDates.isExtendedChristmas`)
// and `ShulkerBoxRenderer` (its `createModelTransform` per facing, the lid's rise and spin from
// `ShulkerBoxBlockEntity.getProgress`). The models are vanilla's `ChestModel` /
// `ShulkerModel.createBoxLayer` layer definitions expressed as data.
//
// Haste's `CubeModel` parts are in vanilla *entity* convention (y down, the front at -z); vanilla's
// chest layer is y-up (its pose stack is world-aligned), so the chest cubes below have their y
// negated against the vanilla `addBox` values and the root matrix ends with the y mirror that
// undoes it. The shulker layer is already in entity convention; its root carries vanilla's own
// `scale(1, -1, -1)` flip instead.

use std::sync::OnceLock;

use glam::{Mat4, Vec3};
use haste_core::model::{Cube, CubeModel, ModelPart, PoseOverrides};
use haste_core::{BlockPos, Direction, Identifier};

use crate::entity::{EntityVertex, TickPair};

/// Which half of a double chest a block is (vanilla `ChestType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChestHalf {
    Single,
    Left,
    Right,
}

impl ChestHalf {
    /// The opposite half (vanilla `ChestType.getOpposite`); a single chest has none and keeps
    /// its own value (callers only use this for doubles).
    pub fn other(self) -> ChestHalf {
        match self {
            ChestHalf::Left => ChestHalf::Right,
            ChestHalf::Right => ChestHalf::Left,
            ChestHalf::Single => ChestHalf::Single,
        }
    }
}

/// A chest's texture set (vanilla `ChestRenderState.ChestMaterialType`, minus copper which Haste
/// does not draw yet).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChestMaterial {
    Normal,
    Trapped,
    Christmas,
    Ender,
}

/// One side of a sign's text, vanilla `SignText`: up to 4 lines, a dye colour and the glowing
/// flag. Lines are already the *rendered* strings: the caller applied vanilla's wrap rule
/// (`Font.split` to the sign's max width, first wrapped line only) when parsing the NBT.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignSideText {
    /// The rendered line (vanilla `SignText.getRenderMessages`, empty lines included).
    pub lines: [String; 4],
    /// The dye colour id, vanilla `DyeColor` order (sign NBT's `color`, default black = 15).
    pub color: u8,
    /// `has_glowing_text`.
    pub glowing: bool,
}

/// Both sides of a sign (vanilla `SignBlockEntity.frontText` / `backText`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignTextData {
    pub front: SignSideText,
    pub back: SignSideText,
}

/// One banner pattern layer (vanilla `BannerPatternLayers.Layer`): the pattern's registry
/// id (its texture is `entity/banner/<path>.png`) and the dye it is drawn in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BannerLayer {
    /// The pattern's registry id, e.g. `minecraft:stripe_bottom`.
    pub pattern: Identifier,
    /// The dye colour id, vanilla `DyeColor` order.
    pub color: u8,
}

/// A banner's render data: the base colour (from the block id, vanilla `BannerBlock.getColor`)
/// and up to 16 pattern layers (vanilla draws at most `MAX_PATTERNS`).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct BannerData {
    /// The base dye colour id.
    pub base: u8,
    /// The pattern layers, in order.
    pub layers: Vec<BannerLayer>,
}

/// What to draw for one block entity (the per-type registry's rows; later renderers for beds
/// and heads plug in here).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockEntityKind {
    Chest(ChestMaterial),
    Shulker(Option<u8>),
    /// A sign or hanging sign: the board is the chunk's block model; the renderer draws the
    /// text. `hanging` picks `HangingSignRenderer`'s transform and line metrics.
    Sign {
        hanging: bool,
    },
    /// A banner: `wall` picks `WallBannerBlock`'s transform (the bar without the pole,
    /// offset to the wall) over the standing one.
    Banner {
        wall: bool,
    },
    /// A skull/head on the floor or a wall (vanilla `SkullBlockRenderer`; the wall flag comes
    /// from the block id like the type does).
    Skull {
        kind: SkullKind,
        wall: bool,
    },
}

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

/// One drawable block entity instance handed to the renderer each frame. `open` is the lid
/// animation as the ends of the last tick (vanilla's `ChestLidController.getOpenness` /
/// `ShulkerBoxBlockEntity.getProgress`), already combined over a double chest's halves;
/// `light` is (sky, block) at the block. Signs carry their text as a shared handle (no
/// per-frame allocation: the client caches one per block entity and re-clones the handle).
#[derive(Clone, Debug)]
pub struct BlockEntityInstance {
    pub pos: BlockPos,
    /// The block's `facing` property (horizontal for chests, any for shulker boxes); for
    /// standing signs the `rotation` segment as a horizontal facing, wall signs the wall's
    /// `facing`.
    pub facing: Direction,
    pub half: ChestHalf,
    pub kind: BlockEntityKind,
    pub open: TickPair,
    pub light: (u8, u8),
    /// The sign's yaw in vanilla degrees (standing: `RotationSegment.convertToDegrees` of the
    /// `rotation` property; wall: the wall `facing.toYRot()`), for `BlockEntityKind::Sign`.
    pub sign_angle: f32,
    /// Sign text (both sides), for `BlockEntityKind::Sign`.
    pub text: Option<std::sync::Arc<SignTextData>>,
    /// The banner's wave phase (vanilla `BannerRenderer.extractRenderState`: the position hash
    /// plus game time, over 100), for `BlockEntityKind::Banner`.
    pub banner_phase: f32,
    /// Banner patterns, for `BlockEntityKind::Banner`.
    pub banner: Option<std::sync::Arc<BannerData>>,
    /// The skull's animation tick as the ends of the last tick (vanilla
    /// `SkullBlockEntity.getAnimation`, counting while the block is powered), for
    /// `BlockEntityKind::Skull`.
    pub skull_anim: TickPair,
}

/// Resolves a block entity type name (from the synced `minecraft:block_entity_type` registry)
/// plus the block it is in into what to draw. `None` draws nothing (logged once by the caller).
/// Vanilla `BlockEntityRenderers` maps the type to its renderer; `ChestRenderer.getChestMaterial`
/// picks the material (ender chest by entity type, Christmas by the calendar, trapped by entity
/// type); the shulker colour comes from the block id (`ShulkerBoxBlock.getColor`).
pub fn kind_of(be_type: &Identifier, block: &Identifier, xmas: bool) -> Option<BlockEntityKind> {
    if be_type.namespace() != "minecraft" {
        return None;
    }
    match be_type.path() {
        "chest" => Some(BlockEntityKind::Chest(if xmas {
            ChestMaterial::Christmas
        } else {
            ChestMaterial::Normal
        })),
        "trapped_chest" => Some(BlockEntityKind::Chest(if xmas {
            ChestMaterial::Christmas
        } else {
            ChestMaterial::Trapped
        })),
        "ender_chest" => Some(BlockEntityKind::Chest(ChestMaterial::Ender)),
        "shulker_box" => Some(BlockEntityKind::Shulker(shulker_color(block))),
        "sign" => Some(BlockEntityKind::Sign { hanging: false }),
        "hanging_sign" => Some(BlockEntityKind::Sign { hanging: true }),
        "banner" => Some(BlockEntityKind::Banner {
            wall: block.path().ends_with("_wall_banner"),
        }),
        "skull" => skull_kind_of(block),
        _ => None,
    }
}

/// The head kind and wall flag from a skull block id (vanilla `SkullBlock.Types`'s blocks:
/// `skeleton_skull`, `wither_skeleton_skull`, `zombie_head`, `creeper_head`, `dragon_head`,
/// `piglin_head`, `player_head`; the wall variants insert `_wall_`, e.g.
/// `creeper_wall_head`).
fn skull_kind_of(block: &Identifier) -> Option<BlockEntityKind> {
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

/// Whether `block` is one of the blocks whose `block_event` action 1 drives a lid (used by the
/// client to filter events; copper chests are not drawn yet and are excluded).
pub fn is_lid_block(block: &Identifier) -> bool {
    block.namespace() == "minecraft"
        && matches!(block.path(), "chest" | "trapped_chest" | "ender_chest")
        || (block.namespace() == "minecraft" && block.path().ends_with("shulker_box"))
}

/// The dye colour id of a shulker box block id (`minecraft:white_shulker_box` -> 0), vanilla
/// `DyeColor`'s order; `None` for the plain `minecraft:shulker_box` (vanilla `getColor`'s null).
fn shulker_color(block: &Identifier) -> Option<u8> {
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

/// Vanilla `SpecialDates.isExtendedChristmas`: December 24 to 26, in the local time zone.
/// Evaluated once per process, like vanilla's `ChestRenderer` constructor does.
pub fn xmas_textures() -> bool {
    static XMAS: OnceLock<bool> = OnceLock::new();
    *XMAS.get_or_init(|| {
        let (month, day) = local_month_day(std::time::SystemTime::now());
        month == 12 && (24..=26).contains(&day)
    })
}

/// The local month (1..12) and day of the month, from the epoch time. UTC when the platform has
/// no time zone support (Rust has none in std); the calendar rule is hours-granular, so the
/// difference only matters around local midnight.
fn local_month_day(t: std::time::SystemTime) -> (u32, u32) {
    let days = t
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0) as i64;
    // Civil-from-days (Howard Hinnant's algorithm), no year limits in range.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let _ = y;
    (m as u32, d as u32)
}

/// The texture a chest half is drawn with (vanilla `Sheets.chooseSprite`: `normal`/`trapped`/
/// `christmas` with `_left`/`_right` for the double halves, `ender` single).
pub fn chest_texture(material: ChestMaterial, half: ChestHalf) -> Identifier {
    let prefix = match material {
        ChestMaterial::Normal => "normal",
        ChestMaterial::Trapped => "trapped",
        ChestMaterial::Christmas => "christmas",
        ChestMaterial::Ender => return Identifier::minecraft("entity/chest/ender"),
    };
    let suffix = match half {
        ChestHalf::Single => "",
        ChestHalf::Left => "_left",
        ChestHalf::Right => "_right",
    };
    Identifier::minecraft(&format!("entity/chest/{prefix}{suffix}"))
}

/// The texture a shulker box is drawn with (vanilla `Sheets.getShulkerBoxSprite`:
/// `entity/shulker/shulker_<color>`, the undyed `entity/shulker/shulker` for `None`).
pub fn shulker_texture(color: Option<u8>) -> Identifier {
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
    match color {
        None => Identifier::minecraft("entity/shulker/shulker"),
        Some(i) => Identifier::minecraft(&format!("entity/shulker/shulker_{}", NAMES[i as usize])),
    }
}

/// Vanilla `ChestModel.createSingleBodyLayer` and friends (26.3), one model per chest half.
/// Haste cube y is the vanilla `addBox` y negated (see the module doc); part pivots likewise.
fn chest_model(half: ChestHalf) -> CubeModel {
    // (bottom x, width) and the lock's x span per half: single 1..15, right 1..16 (east hidden),
    // left 0..15 (west hidden). Vanilla hides the touching face; drawing it is invisible (it
    // lies between the two halves) and saves a per-face flag in the shared `Cube` type.
    let (x, w) = match half {
        ChestHalf::Single => (1.0, 14.0),
        ChestHalf::Left => (0.0, 15.0),
        ChestHalf::Right => (1.0, 15.0),
    };
    let lock_x = match half {
        ChestHalf::Single => 7.0,
        ChestHalf::Left => 0.0,
        ChestHalf::Right => 15.0,
    };
    let lock_w = if half == ChestHalf::Single { 2.0 } else { 1.0 };
    let cube = |origin: Vec3, size: Vec3, uv: [u32; 2]| Cube {
        origin,
        size,
        inflate: 0.0,
        uv,
        mirror: false,
    };
    CubeModel {
        // The chest textures are all 64x64.
        texture: Identifier::minecraft("entity/chest/normal"),
        tex_width: 64,
        tex_height: 64,
        root: ModelPart {
            name: "root".into(),
            pivot: Vec3::ZERO,
            rotation: Vec3::ZERO,
            visible: true,
            cubes: Vec::new(),
            children: vec![
                // bottom: addBox(x, 0, 1, w, 10, 14), texOffs(0, 19)
                ModelPart {
                    name: "bottom".into(),
                    pivot: Vec3::ZERO,
                    rotation: Vec3::ZERO,
                    visible: true,
                    cubes: vec![cube(
                        Vec3::new(x, -10.0, 1.0),
                        Vec3::new(w, 10.0, 14.0),
                        [0, 19],
                    )],
                    children: Vec::new(),
                },
                // lid: addBox(x, 0, 0, w, 5, 14) at PartPose.offset(0, 9, 1), texOffs(0, 0)
                ModelPart {
                    name: "lid".into(),
                    pivot: Vec3::new(0.0, -9.0, 1.0),
                    rotation: Vec3::ZERO,
                    visible: true,
                    cubes: vec![cube(
                        Vec3::new(x, -5.0, 0.0),
                        Vec3::new(w, 5.0, 14.0),
                        [0, 0],
                    )],
                    children: Vec::new(),
                },
                // lock: addBox(lock_x, -2, 14, lock_w, 4, 1) at PartPose.offset(0, 9, 1), texOffs(0, 0)
                ModelPart {
                    name: "lock".into(),
                    pivot: Vec3::new(0.0, -9.0, 1.0),
                    rotation: Vec3::ZERO,
                    visible: true,
                    cubes: vec![cube(
                        Vec3::new(lock_x, -2.0, 14.0),
                        Vec3::new(lock_w, 4.0, 1.0),
                        [0, 0],
                    )],
                    children: Vec::new(),
                },
            ],
        },
    }
}

/// The three chest halves (single, left, right), built once.
fn chest_models() -> &'static [CubeModel; 3] {
    static MODELS: OnceLock<[CubeModel; 3]> = OnceLock::new();
    MODELS.get_or_init(|| {
        [
            chest_model(ChestHalf::Single),
            chest_model(ChestHalf::Left),
            chest_model(ChestHalf::Right),
        ]
    })
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
            pivot: Vec3::ZERO,
            rotation: Vec3::ZERO,
            visible: true,
            cubes: Vec::new(),
            children: vec![
                // lid: addBox(-8, -16, -8, 16, 12, 16), texOffs(0, 0)
                ModelPart {
                    name: "lid".into(),
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

/// Vanilla `Direction.toYRot`: south 0, west 90, north 180, east 270.
fn to_y_rot(d: Direction) -> f32 {
    match d {
        Direction::South => 0.0,
        Direction::West => 90.0,
        Direction::North => 180.0,
        Direction::East => 270.0,
        Direction::Down | Direction::Up => 0.0,
    }
}

/// Vanilla `ChestRenderer.createModelTransformation`: rotate by `-toYRot(facing)` about the
/// block's centre column (0.5, 0, 0.5), with the chest model's y-up-to-entity-convention mirror
/// (see the module doc) folded in at the end.
fn chest_root(pos: BlockPos, facing: Direction, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.0, 0.5))
        * Mat4::from_rotation_y((-to_y_rot(facing)).to_radians())
        * Mat4::from_translation(Vec3::new(-0.5, 0.0, -0.5))
        * Mat4::from_scale(Vec3::new(1.0, 1.0, 1.0) / 16.0)
        * Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0))
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

/// Vanilla `ChestRenderer.submit`'s lid angle curve: `1 - (1 - open)^3`.
pub fn chest_open_curve(open: f32) -> f32 {
    1.0 - (1.0 - open).powi(3)
}

/// The entity renderer's cube emitter, as a type (light and shading are its job).
pub(crate) type EmitPart<'a> = &'a dyn Fn(
    &mut Vec<EntityVertex>,
    &CubeModel,
    &ModelPart,
    &PoseOverrides,
    Mat4,
    [u32; 4],
    (u8, u8),
);

/// Vanilla `DyeColor.getTextColor` (the ARGB text colours), by dye id.
const DYE_TEXT_COLOR: [u32; 16] = [
    0xFFFF_FFFF,
    0xFFFF_681F,
    0xFFFF_00FF,
    0xFF9A_C0CD,
    0xFFFF_FF00,
    0xFFBF_FF00,
    0xFFFF_69B4,
    0xFF80_8080,
    0xFFD3_D3D3,
    0xFF00_FFFF,
    0xFFA0_20F0,
    0xFF00_00FF,
    0xFF8B_4513,
    0xFF00_FF00,
    0xFFFF_0000,
    0xFF00_0000,
];

/// The outline colour vanilla uses for black glowing text (`AbstractSignRenderer.BLACK_TEXT_OUTLINE_COLOR`).
const BLACK_TEXT_OUTLINE_COLOR: u32 = 0xFFF0_EBCC;

/// Vanilla `DyeColor.getTextureDiffuseColor` (the dye's map colour), by dye id.
pub const DYE_DIFFUSE_COLOR: [u32; 16] = [
    16383998, 16351261, 13061821, 3847130, 16701501, 8439583, 15961002, 4673362, 10329495, 1481884,
    8991416, 3949738, 8606770, 6192150, 11546150, 1908001,
];

/// Vanilla `AbstractSignRenderer.getDarkColor`: the colour non-glowing text is drawn with —
/// the dye colour scaled by 0.4 (per channel, clamped), except black glowing text which keeps
/// its special outline colour instead of scaling to near-black.
pub fn sign_dark_color(side: &SignSideText) -> u32 {
    let color = DYE_TEXT_COLOR[side.color as usize % 16];
    if side.color == 15 && side.glowing {
        BLACK_TEXT_OUTLINE_COLOR
    } else {
        let scale = |c: u32| ((c * 2) / 5).min(255);
        0xFF00_0000
            | scale((color >> 16) & 0xFF) << 16
            | scale((color >> 8) & 0xFF) << 8
            | scale(color & 0xFF)
    }
}

/// The colour glowing text is drawn with (the dye's own text colour), vanilla
/// `AbstractSignRenderer.submitSignText`.
pub fn sign_glow_color(side: &SignSideText) -> u32 {
    DYE_TEXT_COLOR[side.color as usize % 16]
}

/// Vanilla `StandingSignRenderer.textTransformation` / `HangingSignRenderer.textTransformation`:
/// the matrix a side's text is drawn through. `angle` is the sign's yaw in vanilla degrees
/// (standing: the `rotation` segment, wall: the wall's `facing.toYRot()`); `front` picks the
/// side (the back adds vanilla's extra 180-degree turn).
pub(crate) fn sign_text_transform(
    hanging: bool,
    angle: f32,
    front: bool,
    camera: glam::DVec3,
    pos: BlockPos,
) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    let mut m = if hanging {
        // Hanging: (0.5, 0.9375, 0.5), rotate -angle, translate (0, -0.3125, 0).
        Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.9375, 0.5))
            * Mat4::from_rotation_y((-angle).to_radians())
            * Mat4::from_translation(Vec3::new(0.0, -0.3125, 0.0))
    } else {
        // Standing: (0.5, 0.5, 0.5), rotate -angle, wall signs drop to (0, -0.3125, -0.4375).
        Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.5, 0.5))
            * Mat4::from_rotation_y((-angle).to_radians())
    };
    if !front {
        m *= Mat4::from_rotation_y(180.0f32.to_radians());
    }
    let s = if hanging { 0.0140625 } else { 0.010416667 };
    let offset = if hanging {
        Vec3::new(0.0, -0.32, 0.073)
    } else {
        Vec3::new(0.0, 1.0 / 3.0, 0.046666667)
    };
    m * Mat4::from_translation(offset) * Mat4::from_scale(Vec3::new(s, -s, s))
}

/// Vanilla `SignBlockEntity.getTextLineHeight` (10) and `HangingSignBlockEntity` (9);
/// `getMaxTextLineWidth` is 90 standing, 60 hanging.
pub fn sign_line_metrics_pub(hanging: bool) -> (i32, i32) {
    sign_line_metrics(hanging)
}

pub(crate) fn sign_line_metrics(hanging: bool) -> (i32, i32) {
    if hanging {
        (9, 60)
    } else {
        (10, 90)
    }
}

/// Vanilla `BannerModel.createBodyLayer` + `BannerFlagModel.createFlagLayer` as one model per
/// attachment: standing has the pole (texOffs 44,0) and the bar at y -44; wall banners drop
/// the pole and hang the bar at y -20.5, z 9.5. The flag (texOffs 0,0) hangs from its pivot
/// (standing (0, -44, 0), wall (0, -20.5, 10.5)) and waves by `BannerFlagModel.setupAnim`.
fn banner_model(wall: bool) -> &'static CubeModel {
    static STANDING: OnceLock<CubeModel> = OnceLock::new();
    static WALL: OnceLock<CubeModel> = OnceLock::new();
    let model = if wall { &WALL } else { &STANDING };
    model.get_or_init(|| {
        let mut children = Vec::new();
        if !wall {
            children.push(ModelPart {
                name: "pole".into(),
                pivot: Vec3::ZERO,
                rotation: Vec3::ZERO,
                visible: true,
                // addBox(-1, -42, -1, 2, 42, 2), texOffs(44, 0)
                cubes: vec![Cube {
                    origin: Vec3::new(-1.0, -42.0, -1.0),
                    size: Vec3::new(2.0, 42.0, 2.0),
                    inflate: 0.0,
                    uv: [44, 0],
                    mirror: false,
                }],
                children: Vec::new(),
            });
        }
        let bar_y = if wall { -20.5 } else { -44.0 };
        let bar_z = if wall { 9.5 } else { -1.0 };
        children.push(ModelPart {
            name: "bar".into(),
            pivot: Vec3::ZERO,
            rotation: Vec3::ZERO,
            visible: true,
            // addBox(-10, bar_y, bar_z, 20, 2, 2), texOffs(0, 42)
            cubes: vec![Cube {
                origin: Vec3::new(-10.0, bar_y, bar_z),
                size: Vec3::new(20.0, 2.0, 2.0),
                inflate: 0.0,
                uv: [0, 42],
                mirror: false,
            }],
            children: Vec::new(),
        });
        let flag_pivot = if wall {
            Vec3::new(0.0, -20.5, 10.5)
        } else {
            Vec3::new(0.0, -44.0, 0.0)
        };
        children.push(ModelPart {
            name: "flag".into(),
            pivot: flag_pivot,
            rotation: Vec3::ZERO,
            visible: true,
            // addBox(-10, 0, -2, 20, 40, 1), texOffs(0, 0)
            cubes: vec![Cube {
                origin: Vec3::new(-10.0, 0.0, -2.0),
                size: Vec3::new(20.0, 40.0, 1.0),
                inflate: 0.0,
                uv: [0, 0],
                mirror: false,
            }],
            children: Vec::new(),
        });
        CubeModel {
            texture: Identifier::minecraft("entity/banner/banner_base"),
            tex_width: 64,
            tex_height: 64,
            root: ModelPart {
                name: "root".into(),
                pivot: Vec3::ZERO,
                rotation: Vec3::ZERO,
                visible: true,
                cubes: Vec::new(),
                children,
            },
        }
    })
}

/// Vanilla `BannerRenderer.modelTransformation`: translate (0.5, 0, 0.5), rotate `-angle`
/// about y, scale (2/3, -2/3, -2/3) — the y/z flip maps the model's y-down entity convention
/// onto the world — and the model renderer's 1/16 from pixels to blocks. `angle` is the
/// `rotation` segment (standing) or the wall `facing.toYRot()`.
fn banner_root(pos: BlockPos, angle: f32, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.0, 0.5))
        * Mat4::from_rotation_y((-angle).to_radians())
        * Mat4::from_scale(Vec3::new(2.0 / 3.0, -2.0 / 3.0, -2.0 / 3.0))
        * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}

/// Vanilla `BannerFlagModel.setupAnim`: the flag swings by
/// `(-0.0125 + 0.01 * cos(2 * PI * phase)) * PI` radians about its pivot's x axis.
pub fn banner_flag_rotation(phase: f32) -> f32 {
    (-0.0125 + 0.01 * (std::f32::consts::TAU * phase).cos()) * std::f32::consts::PI
}

/// Vanilla `BannerRenderer.extractRenderState`'s phase:
/// `(floorMod(x*7 + y*9 + z*13 + gameTime, 100) + partial) / 100`.
pub fn banner_wave_phase(pos: BlockPos, game_time: i64, partial: f32) -> f32 {
    let hash = pos.x * 7 + pos.y * 9 + pos.z * 13;
    (hash.wrapping_add(game_time as i32).rem_euclid(100) as f32 + partial) / 100.0
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
fn skull_model(kind: SkullKind) -> &'static CubeModel {
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
fn skull_root(
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

/// Poses one block entity into `out` (camera-relative, the entity stream's convention).
/// `rect` is the texture's atlas rectangle, `light` the lightmap coordinates (block, sky).
pub(crate) fn emit_block_entity(
    out: &mut Vec<EntityVertex>,
    inst: &BlockEntityInstance,
    camera: glam::DVec3,
    rect: [u32; 4],
    emit_part: EmitPart,
) {
    // The instance stores (sky, block); the vertex wants (block, sky).
    let (sky, block) = inst.light;
    let light = (block, sky);
    match inst.kind {
        BlockEntityKind::Chest(_material) => {
            let models = chest_models();
            let model = &models[match inst.half {
                ChestHalf::Single => 0,
                ChestHalf::Left => 1,
                ChestHalf::Right => 2,
            }];
            let open = chest_open_curve(inst.open.end.max(inst.open.start));
            // The lid and lock rotate by the open angle around their shared pivot (vanilla
            // `ChestModel.setupAnim`; the sign is the y mirror's, see the module doc).
            let mut pose = PoseOverrides::default();
            pose.rotations.push((
                "lid".into(),
                Vec3::new(open * std::f32::consts::FRAC_PI_2, 0.0, 0.0),
            ));
            pose.rotations.push((
                "lock".into(),
                Vec3::new(open * std::f32::consts::FRAC_PI_2, 0.0, 0.0),
            ));
            for part in &model.root.children {
                emit_part(
                    out,
                    model,
                    part,
                    &pose,
                    chest_root(inst.pos, inst.facing, camera),
                    rect,
                    light,
                );
            }
        }
        BlockEntityKind::Shulker(_color) => {
            let model = shulker_model();
            let progress = inst.open.end.max(inst.open.start);
            let mut pose = PoseOverrides::default();
            // Vanilla `ShulkerBoxModel.setupAnim`: the lid spins by 270 degrees * progress...
            pose.rotations.push((
                "lid".into(),
                Vec3::new(0.0, 270.0_f32.to_radians() * progress, 0.0),
            ));
            let root = shulker_root(inst.pos, inst.facing, camera);
            for part in &model.root.children {
                let parent = if part.name == "lid" {
                    // ...and rises by `progress * 0.5 * 16` model units (the pivot moves from
                    // 24 down to 16).
                    root * Mat4::from_translation(Vec3::new(0.0, -8.0 * progress, 0.0))
                } else {
                    root
                };
                emit_part(out, model, part, &pose, parent, rect, light);
            }
        }
        // Signs draw no cubes here: the board is the chunk's block model and the text goes
        // through the entity renderer's glyph pass (`emit_sign_text`), which needs the font.
        BlockEntityKind::Sign { .. } => {}
        // Banners likewise draw per-pattern layers with a tint, handled by
        // `EntityRenderer::emit_banner`.
        BlockEntityKind::Banner { .. } => {}
        BlockEntityKind::Skull { kind, wall } => {
            let model = skull_model(kind);
            let mut pose = PoseOverrides::default();
            // The animation value: vanilla lerps `animationTickCount + partial`; the
            // tick's end value is used here (the chest lids' precedent).
            let anim = inst.skull_anim.end;
            match kind {
                SkullKind::Dragon => {
                    // The jaw swings about its pivot.
                    pose.rotations.push((
                        "jaw".into(),
                        Vec3::new(skull_dragon_jaw_rotation(anim), 0.0, 0.0),
                    ));
                }
                SkullKind::Piglin => {
                    let (left, right) = skull_piglin_ear_rotations(anim);
                    pose.rotations
                        .push(("left_ear".into(), Vec3::new(0.0, 0.0, left)));
                    pose.rotations
                        .push(("right_ear".into(), Vec3::new(0.0, 0.0, right)));
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
    }
}

/// Poses one banner's layers, following vanilla `BannerRenderer.submitBanner`: the model
/// (pole, bar, flag) through `modelTransformation`, the flag waving by the phase, then one
/// pass per pattern layer (the base sprite untinted, the `base` pattern in the banner's
/// colour, then up to 16 patterns in their own), each through the same geometry with its
/// texture and diffuse tint. Draw order is back to front: bar and pole first, then the flag.
pub(crate) fn emit_banner(
    out: &mut Vec<EntityVertex>,
    inst: &BlockEntityInstance,
    camera: glam::DVec3,
    angle: f32,
    rect_of: &mut dyn FnMut(&Identifier) -> [u32; 4],
) {
    use crate::entity::emit_part_tinted;
    let wall = matches!(inst.kind, BlockEntityKind::Banner { wall: true });
    let model = banner_model(wall);
    let (sky, block) = inst.light;
    let light = (block, sky);
    let mut pose = PoseOverrides::default();
    pose.rotations.push((
        "flag".into(),
        Vec3::new(banner_flag_rotation(inst.banner_phase), 0.0, 0.0),
    ));
    let root = banner_root(inst.pos, angle, camera);
    let Some(data) = inst.banner.clone() else {
        return;
    };
    // (texture, tint as linear rgb) per layer, vanilla's submitBanner then submitPatterns.
    let mut layers: Vec<(Identifier, [f32; 3])> = Vec::new();
    let diffuse = |c: u32| {
        [
            ((c >> 16) & 0xFF) as f32 / 255.0,
            ((c >> 8) & 0xFF) as f32 / 255.0,
            (c & 0xFF) as f32 / 255.0,
        ]
    };
    // The base sprite (banner_base) is drawn white; the `base` pattern in the banner colour.
    layers.push((model.texture.clone(), [1.0; 3]));
    layers.push((
        Identifier::minecraft("entity/banner/base"),
        diffuse(DYE_DIFFUSE_COLOR[data.base as usize % 16]),
    ));
    for layer in data.layers.iter().take(16) {
        // The pattern's texture is `entity/banner/<asset id>`; the registry id's path is the
        // asset id for every vanilla pattern.
        let tex = Identifier::minecraft(&format!("entity/banner/{}", layer.pattern.path()));
        layers.push((tex, diffuse(DYE_DIFFUSE_COLOR[layer.color as usize % 16])));
    }
    for (texture, tint) in &layers {
        let rect = rect_of(texture);
        for part in &model.root.children {
            emit_part_tinted(out, model, part, &pose, root, rect, light, 0.0, *tint);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::emit_part;

    fn emit(inst: &BlockEntityInstance) -> Vec<EntityVertex> {
        let mut out = Vec::new();
        let f = |out: &mut Vec<EntityVertex>,
                 model: &CubeModel,
                 part: &ModelPart,
                 pose: &PoseOverrides,
                 m: Mat4,
                 rect: [u32; 4],
                 light: (u8, u8)| {
            emit_part(out, model, part, pose, m, rect, light, 0.0);
        };
        emit_block_entity(&mut out, inst, glam::DVec3::ZERO, [0, 0, 64, 64], &f);
        out
    }

    fn chest(facing: Direction, half: ChestHalf, open: f32) -> BlockEntityInstance {
        BlockEntityInstance {
            pos: BlockPos::new(0, 0, 0),
            facing,
            half,
            kind: BlockEntityKind::Chest(ChestMaterial::Normal),
            open: TickPair::fixed(open),
            light: (15, 0),
            sign_angle: 0.0,
            text: None,
            banner_phase: 0.0,
            banner: None,
            skull_anim: TickPair::fixed(0.0),
        }
    }

    fn shulker(facing: Direction, progress: f32) -> BlockEntityInstance {
        BlockEntityInstance {
            pos: BlockPos::new(0, 0, 0),
            facing,
            half: ChestHalf::Single,
            kind: BlockEntityKind::Shulker(None),
            open: TickPair::fixed(progress),
            light: (15, 0),
            sign_angle: 0.0,
            text: None,
            banner_phase: 0.0,
            banner: None,
            skull_anim: TickPair::fixed(0.0),
        }
    }

    /// Closed single chest: three cubes, six faces each, inside the block (the bottom spans
    /// x/z 1..15, y 0..10; the lid adds up to y 14; the lock reaches the block's front edge).
    #[test]
    fn closed_chest_fits_its_block() {
        let out = emit(&chest(Direction::South, ChestHalf::Single, 0.0));
        assert_eq!(out.len(), 3 * 6 * 6);
        for v in &out {
            let [x, y, z] = v.pos;
            assert!((0.99 / 16.0..=15.01 / 16.0).contains(&x), "x: {x}");
            assert!((-0.01 / 16.0..=14.01 / 16.0).contains(&y), "y: {y}");
            assert!((0.99 / 16.0..=16.01 / 16.0).contains(&z), "z: {z}");
        }
    }

    /// The lid swings up and over the hinge at the back: fully open, the lid's front edge
    /// sits above the block and behind the hinge line (vanilla's `-PI/2` x rotation).
    #[test]
    fn open_chest_lid_swings_back() {
        let out = emit(&chest(Direction::South, ChestHalf::Single, 1.0));
        let max_y = out.iter().map(|v| v.pos[1]).fold(0.0f32, f32::max);
        assert!(max_y > 1.0, "lid does not rise above the block: {max_y}");
        // The lock (front, z 14..15 closed) ends up behind the block's centre.
        // Every vertex of the moved lid stays behind its hinge z = 1/16 + 14/16.
        let hinge = 15.0 / 16.0;
        let front_z = out.iter().map(|v| v.pos[2]).fold(0.0f32, f32::max);
        assert!(front_z <= hinge + 1e-4, "lid front z {front_z} > {hinge}");
    }

    /// A north-facing chest turns its front (the lock, at the block's front edge) from the
    /// south side to the north side: the rotation is vanilla's `-toYRot(north) = -180
    /// degrees` about the block centre, so the lock's face that touched z = 1 touches z = 0.
    #[test]
    fn north_facing_chest_turns_its_front() {
        let south = emit(&chest(Direction::South, ChestHalf::Single, 0.0));
        let north = emit(&chest(Direction::North, ChestHalf::Single, 0.0));
        // The lock protrudes past the box to the block edge: the chest's extreme z is its
        // front. Facing south it is z = 16/16; facing north the chest is rotated within the
        // block, so the extreme front is z = 0 and the box's back fills up to 15/16.
        let max_z = |v: &Vec<EntityVertex>| v.iter().map(|v| v.pos[2]).fold(0.0f32, f32::max);
        let min_z = |v: &Vec<EntityVertex>| v.iter().map(|v| v.pos[2]).fold(f32::MAX, f32::min);
        assert!(max_z(&south) > 15.5 / 16.0);
        assert!(
            min_z(&north) < 0.5 / 16.0,
            "front stays south: {}",
            min_z(&north)
        );
        assert!(max_z(&north) > 14.5 / 16.0, "chest lost its body");
    }

    /// The lid curve: vanilla `1 - (1 - open)^3`.
    #[test]
    fn chest_open_curve_matches_vanilla() {
        assert_eq!(chest_open_curve(0.0), 0.0);
        assert_eq!(chest_open_curve(1.0), 1.0);
        assert!((chest_open_curve(0.5) - 0.875).abs() < 1e-6);
        assert!((chest_open_curve(0.1) - 0.271).abs() < 1e-6);
    }

    /// Closed shulker box: base in the lower half, lid up to the block top; the 0.9995
    /// shrink keeps everything just inside the block (0..1 on every axis, centred).
    #[test]
    fn closed_shulker_fits_its_block() {
        let out = emit(&shulker(Direction::Up, 0.0));
        assert_eq!(out.len(), 2 * 6 * 6);
        for v in &out {
            let [x, y, z] = v.pos;
            assert!((-1e-4..=1.0).contains(&y), "shulker y out: {y}");
            assert!((-1e-4..=1.0 + 1e-4).contains(&x), "shulker x out: {x}");
            assert!((-1e-4..=1.0 + 1e-4).contains(&z), "shulker z out: {z}");
        }
        let max_y = out.iter().map(|v| v.pos[1]).fold(0.0f32, f32::max);
        assert!(max_y > 0.99, "lid does not reach the block top: {max_y}");
    }

    /// Open shulker box: the lid rises half a block (vanilla `MAX_LID_HEIGHT`) and spins.
    #[test]
    fn open_shulker_lid_rises_half_a_block() {
        let closed = emit(&shulker(Direction::Up, 0.0));
        let open = emit(&shulker(Direction::Up, 1.0));
        let top = |v: &Vec<EntityVertex>| v.iter().map(|v| v.pos[1]).fold(0.0f32, f32::max);
        assert!((top(&open) - top(&closed) - 0.5).abs() < 1e-3);
    }

    /// A sideways shulker box lies in the block, not above it: every facing's rotation is
    /// vanilla's `Direction.getRotation`, and the box keeps the block's bounds.
    #[test]
    fn sideways_shulker_stays_in_the_block() {
        for facing in Direction::ALL {
            let out = emit(&shulker(facing, 0.0));
            for v in &out {
                let [x, y, z] = v.pos;
                assert!((-1e-4..=1.0 + 1e-4).contains(&y), "{facing:?} y out: {y}");
                assert!((-1e-4..=1.0 + 1e-4).contains(&x), "{facing:?} x out: {x}");
                assert!((-1e-4..=1.0 + 1e-4).contains(&z), "{facing:?} z out: {z}");
            }
        }
    }

    /// Type resolution: the four families map, unknown types and copper chests do not.
    #[test]
    fn kinds_resolve() {
        let chest = Identifier::minecraft("chest");
        let block = Identifier::minecraft("chest");
        assert_eq!(
            kind_of(&chest, &block, false),
            Some(BlockEntityKind::Chest(ChestMaterial::Normal))
        );
        assert_eq!(
            kind_of(&Identifier::minecraft("trapped_chest"), &block, false),
            Some(BlockEntityKind::Chest(ChestMaterial::Trapped))
        );
        assert_eq!(
            kind_of(&Identifier::minecraft("ender_chest"), &block, false),
            Some(BlockEntityKind::Chest(ChestMaterial::Ender))
        );
        // Christmas overrides normal and trapped (vanilla `getChestMaterial`).
        assert_eq!(
            kind_of(&chest, &block, true),
            Some(BlockEntityKind::Chest(ChestMaterial::Christmas))
        );
        // Shulker colour from the block id.
        assert_eq!(
            kind_of(
                &Identifier::minecraft("shulker_box"),
                &Identifier::minecraft("lime_shulker_box"),
                false
            ),
            Some(BlockEntityKind::Shulker(Some(5)))
        );
        assert_eq!(
            kind_of(
                &Identifier::minecraft("shulker_box"),
                &Identifier::minecraft("shulker_box"),
                false
            ),
            Some(BlockEntityKind::Shulker(None))
        );
        // Signs and hanging signs.
        assert_eq!(
            kind_of(&Identifier::minecraft("sign"), &block, false),
            Some(BlockEntityKind::Sign { hanging: false })
        );
        assert_eq!(
            kind_of(&Identifier::minecraft("hanging_sign"), &block, false),
            Some(BlockEntityKind::Sign { hanging: true })
        );
        // Banners: standing and wall (the wall from the block id).
        assert_eq!(
            kind_of(
                &Identifier::minecraft("banner"),
                &Identifier::minecraft("red_banner"),
                false
            ),
            Some(BlockEntityKind::Banner { wall: false })
        );
        assert_eq!(
            kind_of(
                &Identifier::minecraft("banner"),
                &Identifier::minecraft("blue_wall_banner"),
                false
            ),
            Some(BlockEntityKind::Banner { wall: true })
        );
        // Skulls: every kind from its block id, standing and wall.
        for (block, kind) in [
            ("skeleton_skull", SkullKind::Skeleton),
            ("wither_skeleton_skull", SkullKind::WitherSkeleton),
            ("zombie_head", SkullKind::Zombie),
            ("creeper_head", SkullKind::Creeper),
            ("dragon_head", SkullKind::Dragon),
            ("piglin_head", SkullKind::Piglin),
            ("player_head", SkullKind::Player),
        ] {
            assert_eq!(
                kind_of(
                    &Identifier::minecraft("skull"),
                    &Identifier::minecraft(block),
                    false
                ),
                Some(BlockEntityKind::Skull { kind, wall: false }),
                "{block}"
            );
            let wall_block = block.replace("_skull", "_wall_skull");
            let wall_block = if block.ends_with("_skull") {
                wall_block
            } else {
                block.replace("_head", "_wall_head")
            };
            assert_eq!(
                kind_of(
                    &Identifier::minecraft("skull"),
                    &Identifier::minecraft(&wall_block),
                    false
                ),
                Some(BlockEntityKind::Skull { kind, wall: true }),
                "{wall_block}"
            );
        }
        // Unknown and out of scope.
        assert_eq!(
            kind_of(&Identifier::minecraft("copper_chest"), &block, false),
            None
        );
        assert_eq!(
            kind_of(&Identifier::new("mod", "chest"), &block, false),
            None
        );
    }

    /// The skull models follow vanilla's layers: the mob head is one cube on 64x32, the
    /// humanoid head adds the hat on 64x64, the piglin head has snout and ears on 64x64, the
    /// dragon head is the 0.75-scaled part set with the jaw on 256x256.
    #[test]
    fn skull_models_match_vanilla_layers() {
        let mob = skull_model(SkullKind::Skeleton);
        assert_eq!((mob.tex_width, mob.tex_height), (64, 32));
        assert_eq!(mob.root.children.len(), 1);
        let head = &mob.root.children[0];
        assert_eq!(head.cubes.len(), 1);
        assert_eq!(head.cubes[0].origin, Vec3::new(-4.0, -8.0, -4.0));
        assert_eq!(head.cubes[0].size, Vec3::new(8.0, 8.0, 8.0));

        let humanoid = skull_model(SkullKind::Player);
        assert_eq!((humanoid.tex_width, humanoid.tex_height), (64, 64));
        // Head + hat.
        assert_eq!(humanoid.root.children[0].cubes.len(), 2);
        assert_eq!(humanoid.root.children[0].cubes[1].uv, [32, 0]);
        assert_eq!(humanoid.root.children[0].cubes[1].inflate, 0.25);

        let piglin = skull_model(SkullKind::Piglin);
        assert_eq!((piglin.tex_width, piglin.tex_height), (64, 64));
        let phead = &piglin.root.children[0];
        // Head, snout and two nostrils.
        assert_eq!(phead.cubes.len(), 4);
        // The ears on their pivots.
        assert_eq!(phead.children.len(), 2);
        assert_eq!(phead.children[0].pivot, Vec3::new(4.5, -6.0, 0.0));

        let dragon = skull_model(SkullKind::Dragon);
        assert_eq!((dragon.tex_width, dragon.tex_height), (256, 256));
        let dhead = &dragon.root.children[0];
        // The pivot scaled 0.75: -7.986666 * 0.75.
        assert!((dhead.pivot.y - -5.99).abs() < 0.01, "{}", dhead.pivot.y);
        assert_eq!(dhead.cubes.len(), 6);
        // The jaw child at (0, 4, -8).
        assert_eq!(dhead.children[0].pivot, Vec3::new(0.0, 4.0, -8.0));
    }

    /// The skull transform: a ground skull sits in the block's bottom half (the head cube's
    /// y -8..0 maps to 0..0.5 blocks); a north-wall skull is offset 0.25 into the block and
    /// turned to look away from the wall.
    #[test]
    fn skull_root_places_the_head() {
        let pos = BlockPos::new(0, 0, 0);
        // Ground, rotation 0 (south): the head's centre (0, -4, 0 in model space) lands at
        // (0.5, 0.25, 0.5) — the block centre, a quarter up.
        let m = skull_root(pos, false, Direction::South, 0, glam::DVec3::ZERO);
        let p = m.transform_point3(Vec3::new(0.0, -4.0, 0.0));
        assert!((p.x - 0.5).abs() < 1e-6, "x: {}", p.x);
        assert!((p.y - 0.25).abs() < 1e-6, "y: {}", p.y);
        assert!((p.z - 0.5).abs() < 1e-6, "z: {}", p.z);
        // The head's top (model y -8) is at 0.5 blocks.
        let top = m.transform_point3(Vec3::new(0.0, -8.0, 0.0));
        assert!((top.y - 0.5).abs() < 1e-6, "top: {}", top.y);
        // A south wall (the skull hangs on the south wall of the block behind it, facing
        // north): the origin is 0.25 off the wall.
        let w = skull_root(pos, true, Direction::South, 0, glam::DVec3::ZERO);
        let q = w.transform_point3(Vec3::new(0.0, -4.0, 0.0));
        // The wall's facing points south (+z): the offset moves the head north
        // (0.5 - 1*0.25 = 0.25), and it sits 0.25 + 0.25 up.
        assert!((q.x - 0.5).abs() < 1e-6, "wx: {}", q.x);
        assert!((q.z - 0.25).abs() < 1e-6, "wz: {}", q.z);
        assert!((q.y - 0.5).abs() < 1e-6, "wy: {}", q.y);
    }

    /// The dragon jaw and piglin ear animations match vanilla's formulas.
    #[test]
    fn skull_animations_match_vanilla() {
        // Jaw at anim 0: (sin(0) + 1) * 0.2 = 0.2.
        assert!((skull_dragon_jaw_rotation(0.0) - 0.2).abs() < 1e-6);
        // Ears at anim 0: left -(cos(0) + 2.5) * 0.2 = -0.7, right (cos(0) + 2.5) * 0.2 = 0.7.
        let (l, r) = skull_piglin_ear_rotations(0.0);
        assert!((l - -0.7).abs() < 1e-6, "left: {l}");
        assert!((r - 0.7).abs() < 1e-6, "right: {r}");
    }

    /// The calendar rule is December 24..26 (vanilla `SpecialDates.isExtendedChristmas`);
    /// the civil-date helper is checked on known epochs since the current date is not
    /// controllable from a test. 2025-01-01 is 1735689600.
    #[test]
    fn xmas_covers_december_24_to_26() {
        let day = |secs: u64| {
            local_month_day(
                std::time::SystemTime::UNIX_EPOCH
                    .checked_add(std::time::Duration::from_secs(secs))
                    .unwrap(),
            )
        };
        assert_eq!(day(1_735_689_600), (1, 1)); // 2025-01-01
        assert_eq!(day(1_734_998_400), (12, 24)); // 2024-12-24
        assert_eq!(day(1_735_084_800), (12, 25)); // 2024-12-25
        assert_eq!(day(1_735_171_200), (12, 26)); // 2024-12-26
        assert_eq!(day(1_735_257_600), (12, 27)); // 2024-12-27
        assert_eq!(day(1_734_912_000), (12, 23)); // 2024-12-23
    }

    fn side(color: u8, glowing: bool) -> SignSideText {
        SignSideText {
            lines: std::array::from_fn(|_| String::new()),
            color,
            glowing,
        }
    }

    /// A standing banner's model spans the block: the pole rises from the ground to y = 42/24
    /// blocks (scale 2/3: 42 pixels * 2/3 / 16 = 1.75), the flag hangs below the bar.
    #[test]
    fn banner_model_standing_geometry() {
        let model = banner_model(false);
        assert_eq!(model.root.children.len(), 3);
        let names: Vec<&str> = model.root.children.iter().map(|p| &*p.name).collect();
        assert_eq!(names, vec!["pole", "bar", "flag"]);
        // The pole: addBox(-1, -42, -1, 2, 42, 2).
        let pole = &model.root.children[0];
        assert_eq!(pole.cubes[0].origin, Vec3::new(-1.0, -42.0, -1.0));
        assert_eq!(pole.cubes[0].size, Vec3::new(2.0, 42.0, 2.0));
        // The bar: addBox(-10, -44, -1, 20, 2, 2).
        let bar = &model.root.children[1];
        assert_eq!(bar.cubes[0].origin, Vec3::new(-10.0, -44.0, -1.0));
        assert_eq!(bar.cubes[0].size, Vec3::new(20.0, 2.0, 2.0));
        // The flag: pivot (0, -44, 0), addBox(-10, 0, -2, 20, 40, 1).
        let flag = &model.root.children[2];
        assert_eq!(flag.pivot, Vec3::new(0.0, -44.0, 0.0));
        assert_eq!(flag.cubes[0].origin, Vec3::new(-10.0, 0.0, -2.0));
        assert_eq!(flag.cubes[0].size, Vec3::new(20.0, 40.0, 1.0));
        // The wall banner drops the pole and hangs the bar and flag on the wall's z.
        let wall = banner_model(true);
        assert_eq!(wall.root.children.len(), 2);
        assert_eq!(wall.root.children[0].cubes[0].origin.y, -20.5);
        assert_eq!(wall.root.children[0].cubes[0].origin.z, 9.5);
        assert_eq!(wall.root.children[1].pivot, Vec3::new(0.0, -20.5, 10.5));
    }

    /// Vanilla `BannerRenderer.modelTransformation`: translate (0.5, 0, 0.5), rotate -angle,
    /// scale (2/3, -2/3, -2/3) then 1/16 (model pixels to blocks). The ground point of the
    /// pole (0, 0, 0 in model space) maps to the block centre at floor level.
    #[test]
    fn banner_root_places_the_block_centre() {
        let pos = BlockPos::new(0, 0, 0);
        let m = banner_root(pos, 0.0, glam::DVec3::ZERO);
        // Model point (0, 0, 0): the pole's ground point.
        let p = m.transform_point3(Vec3::ZERO);
        assert!((p.x - 0.5).abs() < 1e-6 && p.y.abs() < 1e-6 && (p.z - 0.5).abs() < 1e-6);
        // The pole top (model y -42, flipped up by the scale) rises 42 * 2/3 / 16 = 1.75
        // blocks above the block's floor.
        let top = m.transform_point3(Vec3::new(0.0, -42.0, 0.0));
        assert!((top.y - 1.75).abs() < 1e-6, "pole top: {}", top.y);
        // The flag's x extents (±10 pixels) map to ±10 * 2/3 / 16 = ±0.4167.
        let left = m.transform_point3(Vec3::new(-10.0, -44.0, 0.0));
        assert!((left.x - (0.5 - 10.0 * 2.0 / 3.0 / 16.0)).abs() < 1e-6);
    }

    /// Vanilla `BannerFlagModel.setupAnim`'s swing and `BannerRenderer`'s phase:
    /// `(-0.0125 + 0.01 * cos(2 * PI * phase)) * PI` about x, phase over position hash and
    /// game time mod 100.
    #[test]
    fn banner_wave_matches_vanilla() {
        // phase 0: cos(0) = 1 -> (-0.0125 + 0.01) * PI = -0.0025 * PI.
        assert!(
            (banner_flag_rotation(0.0) - (-0.0025 * std::f32::consts::PI)).abs() < 1e-6,
            "{}",
            banner_flag_rotation(0.0)
        );
        // phase 0.5: cos(PI) = -1 -> (-0.0125 - 0.01) * PI = -0.0225 * PI.
        assert!(
            (banner_flag_rotation(0.5) - (-0.0225 * std::f32::consts::PI)).abs() < 1e-6,
            "{}",
            banner_flag_rotation(0.5)
        );
        // The phase: floorMod(x*7 + y*9 + z*13 + gameTime, 100), over 100, plus partial.
        let a = banner_wave_phase(BlockPos::new(0, 0, 0), 0, 0.0);
        assert_eq!(a, 0.0);
        let b = banner_wave_phase(BlockPos::new(1, 0, 0), 0, 0.0);
        assert_eq!(b, 0.07);
        let c = banner_wave_phase(BlockPos::new(0, 0, 0), 50, 0.5);
        assert_eq!(c, 0.505);
        // Negative positions wrap (floorMod).
        let d = banner_wave_phase(BlockPos::new(-1, 0, 0), 0, 0.0);
        assert_eq!(d, 0.93);
    }

    /// Vanilla `AbstractSignRenderer.getDarkColor`: non-glowing text is the dye colour scaled
    /// by 0.4 per channel; black glowing text keeps its special outline colour instead.
    #[test]
    fn sign_dark_color_scales_by_four_tenths() {
        // White (0xFFFFFF) * 0.4 = 0x666666.
        assert_eq!(sign_dark_color(&side(0, false)), 0xFF66_6666);
        // Red (0xFF0000) * 0.4 = 0x660000.
        assert_eq!(sign_dark_color(&side(14, false)), 0xFF66_0000);
        // Black glowing keeps vanilla's outline colour.
        assert_eq!(sign_dark_color(&side(15, true)), BLACK_TEXT_OUTLINE_COLOR);
        // Glowing text is drawn with the dye's own colour.
        assert_eq!(sign_glow_color(&side(0, true)), 0xFFFF_FFFF);
    }

    /// The sign text transform at yaw 0, south-facing front: text space is x right, y *up*
    /// (vanilla's -s y scale), origin near the block centre. The front's text faces +z; the
    /// back's matrix is the front's turned by 180 degrees.
    #[test]
    fn sign_text_transform_places_the_origin() {
        let pos = BlockPos::new(0, 0, 0);
        let front = sign_text_transform(false, 0.0, true, glam::DVec3::ZERO, pos);
        let p = front.transform_point3(Vec3::ZERO);
        // Standing sign: (0.5, 0.5, 0.5) + (0, 1/3, 1/21) — the text origin sits just north
        // of the board's centre, one third up.
        assert!((p.x - 0.5).abs() < 1e-6);
        assert!((p.y - (0.5 + 1.0 / 3.0)).abs() < 1e-6, "y: {}", p.y);
        assert!((p.z - (0.5 + 0.046666667)).abs() < 1e-6, "z: {}", p.z);
        // Text x maps to world +x and text y to world -y (the -s scale).
        let px = front.transform_point3(Vec3::new(1.0, 0.0, 0.0));
        let py = front.transform_point3(Vec3::new(0.0, 1.0, 0.0));
        assert!(px.x > p.x);
        assert!(py.y < p.y, "text y is up: {}", py.y);
        // The back is the front rotated 180 degrees about y.
        let back = sign_text_transform(false, 0.0, false, glam::DVec3::ZERO, pos);
        let pb = back.transform_point3(Vec3::ZERO);
        assert!((pb.x - p.x).abs() < 1e-6 && (pb.y - p.y).abs() < 1e-6);
        let pbx = back.transform_point3(Vec3::new(1.0, 0.0, 0.0));
        assert!(pbx.x < pb.x, "back text x flips");
    }

    /// A hanging sign's text transform starts higher (0.9375 - 0.3125 - 0.32) and uses the
    /// bigger 0.0140625 scale.
    #[test]
    fn hanging_sign_text_transform() {
        let pos = BlockPos::new(0, 0, 0);
        let m = sign_text_transform(true, 0.0, true, glam::DVec3::ZERO, pos);
        let p = m.transform_point3(Vec3::ZERO);
        assert!((p.x - 0.5).abs() < 1e-6);
        assert!((p.y - (0.9375 - 0.3125 - 0.32)).abs() < 1e-6, "y: {}", p.y);
        assert!((p.z - (0.5 + 0.073)).abs() < 1e-6, "z: {}", p.z);
    }

    /// Line metrics: standing 10/90 (`SignBlockEntity`), hanging 9/60
    /// (`HangingSignBlockEntity`).
    #[test]
    fn sign_line_metrics_match_vanilla() {
        assert_eq!(sign_line_metrics(false), (10, 90));
        assert_eq!(sign_line_metrics(true), (9, 60));
    }
}
