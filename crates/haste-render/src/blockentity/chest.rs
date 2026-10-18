// Chests (vanilla `ChestRenderer`): the halves and material (`ChestType`, `ChestRenderState
// .ChestMaterialType`), the textures (`Sheets.chooseSprite`), the Christmas rule
// (`SpecialDates.isExtendedChristmas`), the model per half (`ChestModel`) and the lid pose
// (`ChestModel.setupAnim` through the lid angle curve `1 - (1 - open)^3`).
//
// Haste cube y is the vanilla `addBox` y negated (see the parent module's doc); the root
// matrix ends with the y mirror that undoes it.

use std::sync::OnceLock;

use glam::{Mat4, Vec3};
use haste_core::model::{Cube, CubeModel, ModelPart, PoseOverrides};
use haste_core::{parts, BlockPos, Direction, Identifier};

use super::{to_y_rot, BlockEntityInstance, EmitPart};
use crate::entity::EntityVertex;

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
pub(super) fn local_month_day(t: std::time::SystemTime) -> (u32, u32) {
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
            id: parts::ROOT,
            pivot: Vec3::ZERO,
            rotation: Vec3::ZERO,
            visible: true,
            cubes: Vec::new(),
            children: vec![
                // bottom: addBox(x, 0, 1, w, 10, 14), texOffs(0, 19)
                ModelPart {
                    name: "bottom".into(),
                    id: parts::intern("bottom"),
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
                    id: parts::LID,
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
                    id: parts::LOCK,
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

/// Vanilla `ChestRenderer.submit`'s lid angle curve: `1 - (1 - open)^3`.
pub fn chest_open_curve(open: f32) -> f32 {
    1.0 - (1.0 - open).powi(3)
}

/// Poses one chest into `out` (the `emit_block_entity` chest arm): the model of its half with
/// the lid and lock swung by the open angle.
pub(super) fn submit(
    out: &mut Vec<EntityVertex>,
    inst: &BlockEntityInstance,
    camera: glam::DVec3,
    rect: [u32; 4],
    light: (u8, u8),
    emit_part: EmitPart,
) {
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
    pose.set_rotation(
        parts::LID,
        Vec3::new(open * std::f32::consts::FRAC_PI_2, 0.0, 0.0),
    );
    pose.set_rotation(
        parts::LOCK,
        Vec3::new(open * std::f32::consts::FRAC_PI_2, 0.0, 0.0),
    );
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
