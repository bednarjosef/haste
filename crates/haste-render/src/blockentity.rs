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

/// What to draw for one block entity (the per-type registry's rows; later renderers for signs,
/// beds, banners and heads plug in here).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockEntityKind {
    Chest(ChestMaterial),
    Shulker(Option<u8>),
}

/// One drawable block entity instance handed to the renderer each frame. `open` is the lid
/// animation as the ends of the last tick (vanilla's `ChestLidController.getOpenness` /
/// `ShulkerBoxBlockEntity.getProgress`), already combined over a double chest's halves;
/// `light` is (sky, block) at the block.
#[derive(Clone, Copy, Debug)]
pub struct BlockEntityInstance {
    pub pos: BlockPos,
    /// The block's `facing` property (horizontal for chests, any for shulker boxes).
    pub facing: Direction,
    pub half: ChestHalf,
    pub kind: BlockEntityKind,
    pub open: TickPair,
    pub light: (u8, u8),
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
        _ => None,
    }
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
        // Unknown and out of scope.
        assert_eq!(kind_of(&Identifier::minecraft("sign"), &block, false), None);
        assert_eq!(
            kind_of(&Identifier::minecraft("copper_chest"), &block, false),
            None
        );
        assert_eq!(
            kind_of(&Identifier::new("mod", "chest"), &block, false),
            None
        );
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
}
