//! The armour stand: its body model, the synced pose and the hit wobble (vanilla
//! `ArmorStandRenderer`, `ArmorStandModel`, 26.3 client).
//!
//! The stand is a code-built model like the humanoid (vanilla bakes
//! `ArmorStandModel.createBodyLayer`, not a JSON layer), so both sizes (the small one is the
//! body layer through `HumanoidModel.BABY_TRANSFORMER`) are built here once and shared as
//! `Arc`s. Parts are authored with vanilla `PartPose` pivots (the literals match the vanilla
//! source); the root children are then shifted into Haste model space (`crate::models`: the
//! ground at y = 0 instead of vanilla's y = 24, y still growing downward).
//!
//! The pose (head/body/arms/legs rotations in degrees, the small/show-arms/no-base-plate
//! flags) is synced entity data (`ArmorStand.DATA_CLIENT_FLAGS` at index 15, the six
//! `Rotations` at 16..=21); `pose` turns it into `PoseOverrides` for the renderer, and
//! `body_transform` is `ArmorStandRenderer.setupRotations`' hit wobble (the stand never gets
//! vanilla's death tilt or sleeping rotations: the override does not call super).
use std::sync::{Arc, OnceLock};

use haste_core::entity::MetadataValue;
use haste_core::math::Vec3;
use haste_core::model::{BodyOp, BodyTransform, Cube, CubeModel, ModelPart};
use haste_core::parts::{self, PartId};
use haste_core::Identifier;

use crate::store::Entity;

/// `ArmorStand.DATA_CLIENT_FLAGS` (index 15: `Entity`'s eight slots, `LivingEntity`'s seven).
const CLIENT_FLAGS_INDEX: u8 = 15;
/// The head/body/arm/leg poses start right after the flags byte (`ArmorStand.defineSynchedData`).
const POSE_INDICES: [u8; 6] = [16, 17, 18, 19, 20, 21];

/// Vanilla's ground plane in model units (see `crate::models`); the small layer scales around
/// it (`HumanoidModel.BABY_TRANSFORMER`).
const GROUND_Y: f32 = 24.0;

/// The flags byte (vanilla `ArmorStand.isSmall`/`showArms`/`showBasePlate`: bits 0x01/0x04/
/// 0x08 inverted). The marker bit (0x10) needs no reading: vanilla 26.3's
/// `ArmorStandRenderer.getRenderType` still draws a visible marker stand's body (it only
/// picks a plainer render type), so Haste's opaque pipeline draws it like any other.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ClientFlags {
    pub small: bool,
    pub show_arms: bool,
    pub show_base_plate: bool,
}

/// The six synced poses in radians (vanilla `Rotations` are degrees; the models take radians).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SyncedPose {
    pub head: Vec3,
    pub body: Vec3,
    pub left_arm: Vec3,
    pub right_arm: Vec3,
    pub left_leg: Vec3,
    pub right_leg: Vec3,
}

fn flags_from(metadata: &crate::metadata::Metadata) -> ClientFlags {
    let b = match metadata.get(CLIENT_FLAGS_INDEX).map(|x| &x.value) {
        Some(MetadataValue::Byte(b)) => *b as u8,
        _ => 0,
    };
    ClientFlags {
        small: b & 0x01 != 0,
        show_arms: b & 0x04 != 0,
        show_base_plate: b & 0x08 == 0,
    }
}

fn flags(e: &Entity) -> ClientFlags {
    flags_from(&e.metadata)
}

/// The small flag (`ArmorStand.isSmall`): picks the small model pair.
pub(crate) fn is_small(metadata: &crate::metadata::Metadata) -> bool {
    flags_from(metadata).small
}

/// Degrees to radians, one axis triple at a time (vanilla multiplies each pose component by
/// `PI / 180` in `ArmorStandArmorModel.setupAnim`).
fn deg([x, y, z]: [f32; 3]) -> Vec3 {
    Vec3::new(x.to_radians(), y.to_radians(), z.to_radians())
}

/// The synced pose with vanilla's defaults for slots the server has not sent
/// (`ArmorStand.DEFAULT_*_POSE`).
fn pose_values(e: &Entity) -> SyncedPose {
    let rotations = |i: u8, default: [f32; 3]| -> Vec3 {
        match e.metadata.get(i).map(|x| &x.value) {
            Some(MetadataValue::Rotations(r)) => deg(*r),
            _ => deg(default),
        }
    };
    SyncedPose {
        head: rotations(POSE_INDICES[0], [0.0, 0.0, 0.0]),
        body: rotations(POSE_INDICES[1], [0.0, 0.0, 0.0]),
        left_arm: rotations(POSE_INDICES[2], [-10.0, 0.0, -10.0]),
        right_arm: rotations(POSE_INDICES[3], [-15.0, 0.0, 10.0]),
        left_leg: rotations(POSE_INDICES[4], [-1.0, 0.0, -1.0]),
        right_leg: rotations(POSE_INDICES[5], [1.0, 0.0, 1.0]),
    }
}

/// The part ids the stand's own model carries beyond the humanoid six, interned once (the
/// frame path matches by id, never by string).
struct StandParts {
    right_body_stick: PartId,
    left_body_stick: PartId,
    shoulder_stick: PartId,
    base_plate: PartId,
}

fn stand_parts() -> &'static StandParts {
    static IDS: OnceLock<StandParts> = OnceLock::new();
    IDS.get_or_init(|| StandParts {
        right_body_stick: parts::intern("right_body_stick"),
        left_body_stick: parts::intern("left_body_stick"),
        shoulder_stick: parts::intern("shoulder_stick"),
        base_plate: parts::intern("base_plate"),
    })
}

/// A part with one cube (`CubeListBuilder.create().texOffs(uv).addBox(...)`, optional mirror
/// and `CubeDeformation`), at a vanilla `PartPose` pivot.
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
/// .scaled(s)` around the ground plane (heads translate (0, 16, 0) and scale 3/4, the rest
/// (0, 24, 0) and 1/2; `HumanoidModel.BABY_TRANSFORMER`), the subtree geometry scaled by `s`
/// — the model carries no per-part scale, so it is baked in; UVs are untouched, like
/// vanilla's part scale.
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

/// A model from root children already in Haste space.
fn model(texture: Identifier, tex_height: u32, children: Vec<ModelPart>) -> Arc<CubeModel> {
    Arc::new(CubeModel {
        texture,
        tex_width: 64,
        tex_height,
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

/// The body model (`ArmorStandRenderer.submit` picks `ARMOR_STAND` or `ARMOR_STAND_SMALL` by
/// the small flag). The hat part vanilla hides in the `ArmorStandModel` constructor is left
/// out entirely, so the helmet's hat (the armour model's own part) is unaffected by nothing
/// here hiding it.
pub(crate) fn body_model(small: bool) -> Arc<CubeModel> {
    static MODELS: OnceLock<(Arc<CubeModel>, Arc<CubeModel>)> = OnceLock::new();
    let (big, small_model) = MODELS.get_or_init(|| {
        let stick = |name: &str, origin: [f32; 3], uv: [u32; 2]| {
            part(
                name,
                [0.0, 0.0, 0.0],
                origin,
                [2.0, 7.0, 2.0],
                uv,
                false,
                0.0,
            )
        };
        let children = vec![
            part(
                "head",
                [0.0, 1.0, 0.0],
                [-1.0, -7.0, -1.0],
                [2.0, 7.0, 2.0],
                [0, 0],
                false,
                0.0,
            ),
            part(
                "body",
                [0.0, 0.0, 0.0],
                [-6.0, 0.0, -1.5],
                [12.0, 3.0, 3.0],
                [0, 26],
                false,
                0.0,
            ),
            part(
                "right_arm",
                [-5.0, 2.0, 0.0],
                [-2.0, -2.0, -1.0],
                [2.0, 12.0, 2.0],
                [24, 0],
                false,
                0.0,
            ),
            part(
                "left_arm",
                [5.0, 2.0, 0.0],
                [0.0, -2.0, -1.0],
                [2.0, 12.0, 2.0],
                [32, 16],
                true,
                0.0,
            ),
            part(
                "right_leg",
                [-1.9, 12.0, 0.0],
                [-1.0, 0.0, -1.0],
                [2.0, 11.0, 2.0],
                [8, 0],
                false,
                0.0,
            ),
            part(
                "left_leg",
                [1.9, 12.0, 0.0],
                [-1.0, 0.0, -1.0],
                [2.0, 11.0, 2.0],
                [40, 16],
                true,
                0.0,
            ),
            stick("right_body_stick", [-3.0, 3.0, -1.0], [16, 0]),
            stick("left_body_stick", [1.0, 3.0, -1.0], [48, 16]),
            part(
                "shoulder_stick",
                [0.0, 0.0, 0.0],
                [-4.0, 10.0, -1.0],
                [8.0, 2.0, 2.0],
                [0, 48],
                false,
                0.0,
            ),
            part(
                "base_plate",
                [0.0, 12.0, 0.0],
                [-6.0, 11.0, -6.0],
                [12.0, 1.0, 12.0],
                [0, 32],
                false,
                0.0,
            ),
        ]
        .into_iter()
        .map(grounded)
        .collect::<Vec<_>>();
        let texture = Identifier::minecraft("entity/armorstand/armorstand");
        let big = model(texture.clone(), 64, children);
        let small = model(
            texture,
            64,
            big.root
                .children
                .iter()
                .map(|c| small_layer_part(c.clone()))
                .collect(),
        );
        (big, small)
    });
    if small {
        small_model.clone()
    } else {
        big.clone()
    }
}

// The armour stand's armour shapes live in haste-render next to the humanoid armour
// (`HumanoidArmorLayer`'s model sets are renderer-side in vanilla too): see
// `crates/haste-render/src/entity/armor_stand.rs`.

/// `ArmorStandArmorModel.setupAnim` + `ArmorStandModel.setupAnim`: every part's rotation from
/// the synced pose, the sticks following the body, the base plate counter-rotating the
/// entity's yaw (`(PI / 180) * -state.yRot`, `state.yRot` being the entity yaw in degrees),
/// and the arms/base plate hidden per the flags. The head aim never runs: the pose replaces
/// it, like the vanilla `setupAnim` that overwrites the humanoid head rotation.
pub(crate) fn pose(e: &Entity, yaw_deg: f32) -> haste_core::model::PoseOverrides {
    let flags = flags(e);
    let values = pose_values(e);
    let ids = stand_parts();
    let mut o = haste_core::model::PoseOverrides::default();
    o.set_rotation(parts::HEAD, values.head);
    o.set_rotation(parts::BODY, values.body);
    o.set_rotation(parts::RIGHT_ARM, values.right_arm);
    o.set_rotation(parts::LEFT_ARM, values.left_arm);
    o.set_rotation(parts::RIGHT_LEG, values.right_leg);
    o.set_rotation(parts::LEFT_LEG, values.left_leg);
    // The three sticks carry the body pose (`ArmorStandModel.setupAnim`).
    o.set_rotation(ids.right_body_stick, values.body);
    o.set_rotation(ids.left_body_stick, values.body);
    o.set_rotation(ids.shoulder_stick, values.body);
    // The plate's own yaw: vanilla `ArmorStandModel.setupAnim`.
    o.set_rotation(ids.base_plate, Vec3::new(0.0, -yaw_deg.to_radians(), 0.0));
    if !flags.show_arms {
        o.hide(parts::RIGHT_ARM);
        o.hide(parts::LEFT_ARM);
    }
    if !flags.show_base_plate {
        o.hide(ids.base_plate);
    }
    o
}

/// `ArmorStandRenderer.setupRotations`: the hit wobble — `sin(wiggle / 1.5 * PI) * 3` degrees
/// of yaw while `wiggle = gameTime - lastHit + partial` is under 5 — and nothing else (no
/// death tilt, sleeping or upside-down: the vanilla override does not call super). `last_hit`
/// is the entity's tick count when entity event 32 arrived (`ArmorStand.handleEntityEvent`).
pub(crate) fn body_transform(e: &Entity, partial: f32) -> BodyTransform {
    let mut b = BodyTransform::default();
    let wiggle = (e.tick_count as i64 - e.last_hit) as f32 + partial;
    if wiggle < 5.0 {
        let wobble = (wiggle / 1.5 * std::f32::consts::PI).sin() * 3.0;
        b.push(BodyOp::RotateY(wobble.to_radians()));
    }
    b
}
