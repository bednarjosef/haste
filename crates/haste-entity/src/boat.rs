//! Boats, chest boats and rafts: the client-side state of vanilla `AbstractBoat` that
//! `AbstractBoatRenderer`/`BoatRenderer`/`RaftRenderer` draw from (`BoatRenderState`), and the
//! per-type model/texture table those renderers are registered with
//! (`EntityRenderers`/`ModelLayers`).
//!
//! The renderer's own transform (vanilla `AbstractBoatRenderer.submit`: the 0.375 lift, the
//! hurt wobble, the bubble-column rocking, the mirrored scale and the trailing 90-degree yaw)
//! is carried by the generic entity path: the lift as `BodyTransform::pre_translate`, the
//! wobble and rocking as body rotations, and the trailing yaw baked into the model's root
//! part. The water patch (`BoatRenderer.submitTypeAdditions`, vanilla `BoatModel`'s
//! `water_patch` part) is handed to the renderer as an overlay model it draws depth-only;
//! `RaftRenderer` (the bamboo raft) has no patch.

use std::sync::{Arc, OnceLock};

use glam::Vec3;
use haste_core::entity::MetadataValue;
use haste_core::model::{
    BodyOp, BodyTransform, Cube, CubeModel, EntityOverlay, ModelPart, PoseOverrides,
};
use haste_core::Identifier;

use crate::item_entity::ItemWorld;
use crate::metadata::Metadata;
use crate::store::Entity;

/// `VehicleEntity.DATA_ID_HURT` (index 8: `Entity`'s eight synched fields 0-7 come first, then
/// `VehicleEntity`'s three).
pub const HURT_METADATA_INDEX: u8 = 8;
/// `VehicleEntity.DATA_ID_HURTDIR` (index 9, an int; 1 or -1).
pub const HURT_DIR_METADATA_INDEX: u8 = 9;
/// `VehicleEntity.DATA_ID_DAMAGE` (index 10, a float).
pub const DAMAGE_METADATA_INDEX: u8 = 10;
/// `AbstractBoat.DATA_ID_PADDLE_LEFT` (index 11, a bool).
pub const PADDLE_LEFT_METADATA_INDEX: u8 = 11;
/// `AbstractBoat.DATA_ID_PADDLE_RIGHT` (index 12, a bool).
pub const PADDLE_RIGHT_METADATA_INDEX: u8 = 12;
/// `AbstractBoat.DATA_ID_BUBBLE_TIME` (index 13, an int).
pub const BUBBLE_TIME_METADATA_INDEX: u8 = 13;

/// `AbstractBoat.PADDLE_SPEED`: the radians a paddle sweeps per tick while its flag is set.
pub const PADDLE_SPEED: f32 = std::f32::consts::FRAC_PI_8;
/// `AbstractBoat.tickBubbleColumn`: the bubble-column rocking amplitude in degrees.
const BUBBLE_AMPLITUDE: f32 = 10.0;

/// One boat family entry: the model layer its renderer bakes and the texture it draws
/// (vanilla `BoatRenderer`/`RaftRenderer` build the texture path from the layer's model path:
/// `textures/entity/<path>.png`).
pub struct BoatEntry {
    /// The dump layer id (`minecraft:boat/oak#main`, `ModelLayers.OAK_BOAT`).
    pub layer: &'static str,
    /// The texture under `assets/minecraft/textures/`, without the prefix.
    pub texture: &'static str,
    /// A chest boat (`chest_boat/<wood>` layers, the chest parts on the model).
    pub chest: bool,
    /// A raft (`RaftRenderer`: no water patch; only the bamboo family).
    pub raft: bool,
}

const fn boat(layer: &'static str, texture: &'static str, chest: bool, raft: bool) -> BoatEntry {
    BoatEntry {
        layer,
        texture,
        chest,
        raft,
    }
}

/// The boat family: every type `EntityRenderers` registers a `BoatRenderer` or `RaftRenderer`
/// for (11 woods; bamboo is the raft). A mod's boat type is not here and draws the generic
/// box, never a panic.
static BOATS: &[(&str, BoatEntry)] = &[
    (
        "minecraft:acacia_boat",
        boat(
            "minecraft:boat/acacia#main",
            "entity/boat/acacia",
            false,
            false,
        ),
    ),
    (
        "minecraft:acacia_chest_boat",
        boat(
            "minecraft:chest_boat/acacia#main",
            "entity/chest_boat/acacia",
            true,
            false,
        ),
    ),
    (
        "minecraft:bamboo_raft",
        boat(
            "minecraft:boat/bamboo#main",
            "entity/boat/bamboo",
            false,
            true,
        ),
    ),
    (
        "minecraft:bamboo_chest_raft",
        boat(
            "minecraft:chest_boat/bamboo#main",
            "entity/chest_boat/bamboo",
            true,
            true,
        ),
    ),
    (
        "minecraft:birch_boat",
        boat(
            "minecraft:boat/birch#main",
            "entity/boat/birch",
            false,
            false,
        ),
    ),
    (
        "minecraft:birch_chest_boat",
        boat(
            "minecraft:chest_boat/birch#main",
            "entity/chest_boat/birch",
            true,
            false,
        ),
    ),
    (
        "minecraft:cherry_boat",
        boat(
            "minecraft:boat/cherry#main",
            "entity/boat/cherry",
            false,
            false,
        ),
    ),
    (
        "minecraft:cherry_chest_boat",
        boat(
            "minecraft:chest_boat/cherry#main",
            "entity/chest_boat/cherry",
            true,
            false,
        ),
    ),
    (
        "minecraft:dark_oak_boat",
        boat(
            "minecraft:boat/dark_oak#main",
            "entity/boat/dark_oak",
            false,
            false,
        ),
    ),
    (
        "minecraft:dark_oak_chest_boat",
        boat(
            "minecraft:chest_boat/dark_oak#main",
            "entity/chest_boat/dark_oak",
            true,
            false,
        ),
    ),
    (
        "minecraft:jungle_boat",
        boat(
            "minecraft:boat/jungle#main",
            "entity/boat/jungle",
            false,
            false,
        ),
    ),
    (
        "minecraft:jungle_chest_boat",
        boat(
            "minecraft:chest_boat/jungle#main",
            "entity/chest_boat/jungle",
            true,
            false,
        ),
    ),
    (
        "minecraft:mangrove_boat",
        boat(
            "minecraft:boat/mangrove#main",
            "entity/boat/mangrove",
            false,
            false,
        ),
    ),
    (
        "minecraft:mangrove_chest_boat",
        boat(
            "minecraft:chest_boat/mangrove#main",
            "entity/chest_boat/mangrove",
            true,
            false,
        ),
    ),
    (
        "minecraft:oak_boat",
        boat("minecraft:boat/oak#main", "entity/boat/oak", false, false),
    ),
    (
        "minecraft:oak_chest_boat",
        boat(
            "minecraft:chest_boat/oak#main",
            "entity/chest_boat/oak",
            true,
            false,
        ),
    ),
    (
        "minecraft:pale_oak_boat",
        boat(
            "minecraft:boat/pale_oak#main",
            "entity/boat/pale_oak",
            false,
            false,
        ),
    ),
    (
        "minecraft:pale_oak_chest_boat",
        boat(
            "minecraft:chest_boat/pale_oak#main",
            "entity/chest_boat/pale_oak",
            true,
            false,
        ),
    ),
    (
        "minecraft:poplar_boat",
        boat(
            "minecraft:boat/poplar#main",
            "entity/boat/poplar",
            false,
            false,
        ),
    ),
    (
        "minecraft:poplar_chest_boat",
        boat(
            "minecraft:chest_boat/poplar#main",
            "entity/chest_boat/poplar",
            true,
            false,
        ),
    ),
    (
        "minecraft:spruce_boat",
        boat(
            "minecraft:boat/spruce#main",
            "entity/boat/spruce",
            false,
            false,
        ),
    ),
    (
        "minecraft:spruce_chest_boat",
        boat(
            "minecraft:chest_boat/spruce#main",
            "entity/chest_boat/spruce",
            true,
            false,
        ),
    ),
];

/// The boat family entry for an entity type name, `None` for every other type (a mod's boat
/// included: it keeps the generic box).
pub fn entry_for_name(name: &str) -> Option<&'static BoatEntry> {
    BOATS.iter().find(|(n, _)| *n == name).map(|(_, e)| e)
}

/// Every boat family entry with its type name, for tests.
pub fn all_entries() -> impl Iterator<Item = (&'static str, &'static BoatEntry)> {
    BOATS.iter().map(|(n, e)| (*n, e))
}

/// Whether a type is the `AbstractBoat` family at all (vanilla or a mod's): the client steers
/// every `*_boat`/`*_raft` type (`crate`-local twin of the client's `is_boat_type`), and the
/// family's synched-data layout (hurt, paddles, bubble time at indices 8-13) is the base
/// class's, so the client-side tick state runs for mods' boats too.
pub fn is_boat_family(path: &str) -> bool {
    path.ends_with("_boat") || path.ends_with("_raft")
}

/// The client-side state of one `AbstractBoat` (vanilla's fields of the same names): the
/// paddle sweep positions, the bubble-column rocking and whether the boat is under water
/// (the water patch's gate, `BoatRenderState.isUnderWater`).
#[derive(Clone, Copy, Debug, Default)]
pub struct BoatState {
    /// `AbstractBoat.paddlePositions`: the swept radians per side, advanced one
    /// [`PADDLE_SPEED`] per tick its flag is set.
    pub(crate) paddle_positions: [f32; 2],
    /// `AbstractBoat.bubbleMultiplier`, eased towards 1 in a bubble column.
    pub(crate) bubble_multiplier: f32,
    /// `AbstractBoat.bubbleAngle` / `bubbleAngleO`: this tick's rocking and the last's.
    pub(crate) bubble_angle: f32,
    pub(crate) bubble_angle_o: f32,
    /// `Entity.isUnderWater()` as of the last tick (the water patch's gate).
    pub underwater: bool,
}

impl BoatState {
    /// `AbstractBoat.getBubbleAngle(partialTicks)`: `Mth.lerp(partial, bubbleAngleO, bubbleAngle)`.
    pub fn bubble_angle(&self, partial: f32) -> f32 {
        self.bubble_angle_o + (self.bubble_angle - self.bubble_angle_o) * partial
    }
}

fn var_int(m: &Metadata, index: u8) -> Option<i32> {
    match m.get(index).map(|e| &e.value) {
        Some(MetadataValue::VarInt(v)) => Some(*v),
        _ => None,
    }
}

fn boolean(m: &Metadata, index: u8) -> bool {
    matches!(
        m.get(index).map(|e| &e.value),
        Some(MetadataValue::Boolean(true))
    )
}

/// `AbstractBoat.tick`'s client-side state, in vanilla's order: the paddle sweeps after
/// `tickBubbleColumn`, the rocking from the synced bubble time. `eye_height` is the type's
/// (`EntityDimensions::eye_height`); the under-water gate stands in vanilla's
/// `wasEyeInWater && isInWater` with the water surface at the hull's centre reaching the eye.
pub fn tick(e: &mut Entity, world: &dyn ItemWorld, eye_height: f32) {
    let BoatState {
        paddle_positions,
        bubble_multiplier,
        bubble_angle,
        bubble_angle_o,
        underwater,
    } = &mut e.boat;
    // `tickBubbleColumn`'s client half: the multiplier eases in over 20 ticks while the
    // synced bubble time counts, out over 10 once it stops.
    let bubble_time = var_int(&e.metadata, BUBBLE_TIME_METADATA_INDEX).unwrap_or(0);
    *bubble_multiplier = if bubble_time > 0 {
        *bubble_multiplier + 0.05
    } else {
        *bubble_multiplier - 0.1
    }
    .clamp(0.0, 1.0);
    *bubble_angle_o = *bubble_angle;
    *bubble_angle = BUBBLE_AMPLITUDE * (0.5 * e.tick_count as f32).sin() * *bubble_multiplier;
    // The paddle sweeps: vanilla requires a controlling passenger for `getPaddleState`; the
    // server only leaves the flags set while one drives, so the flag alone matches (the
    // authoritative side clears them on dismount, `AbstractBoat.tick`).
    for (side, index) in [
        (0, PADDLE_LEFT_METADATA_INDEX),
        (1, PADDLE_RIGHT_METADATA_INDEX),
    ] {
        paddle_positions[side] = if boolean(&e.metadata, index) {
            paddle_positions[side] + PADDLE_SPEED
        } else {
            0.0
        };
    }
    *underwater = world.fluid_height(e.pos, haste_core::view::FluidKind::Water) >= eye_height;
}

/// `AbstractBoat.getRowingTime(side, partialTicks)`: the sweep interpolated through the tick
/// while the paddle is in the water, 0 when it is not (`Mth.clampedLerp(partial, position -
/// PADDLE_SPEED, position)`).
pub fn rowing_time(flag: bool, position: f32, partial: f32) -> f32 {
    if flag {
        clamped_lerp(partial, position - PADDLE_SPEED, position)
    } else {
        0.0
    }
}

/// `AbstractBoatModel.animatePaddle`'s x bounds: `-PI / 3` to `-PI / 12`.
const PADDLE_X_MIN: f32 = -std::f32::consts::FRAC_PI_3;
const PADDLE_X_MAX: f32 = -std::f32::consts::PI / 12.0;
/// `AbstractBoatModel.animatePaddle`'s y bounds: `-PI / 4` to `PI / 4`.
const PADDLE_Y_MIN: f32 = -std::f32::consts::FRAC_PI_4;
const PADDLE_Y_MAX: f32 = std::f32::consts::FRAC_PI_4;

/// `AbstractBoatModel.animatePaddle`: the paddle's absolute x and y rotation for a rowing
/// time (its z stays at the model's rest tilt, `Mth.PI / 16`). `right` is the model's right
/// paddle, whose y rotation mirrors.
pub fn animate_paddle(time: f32, right: bool) -> (f32, f32) {
    let x = clamped_lerp(((-time).sin() + 1.0) / 2.0, PADDLE_X_MIN, PADDLE_X_MAX);
    let mut y = clamped_lerp(
        ((-time + 1.0).sin() + 1.0) / 2.0,
        PADDLE_Y_MIN,
        PADDLE_Y_MAX,
    );
    if right {
        y = std::f32::consts::PI - y;
    }
    (x, y)
}

/// `Mth.clampedLerp(factor, min, max)`: a lerp whose factor is clamped to 0..1.
fn clamped_lerp(factor: f32, min: f32, max: f32) -> f32 {
    min + (max - min) * factor.clamp(0.0, 1.0)
}

/// `AbstractBoatModel.setupAnim` for one boat: the two paddles' rotations (additive to their
/// rest pose, like every `PoseOverrides` rotation; vanilla assigns absolute values, so the
/// rest x/y of the paddle parts — 0, and 0 or PI — is subtracted here).
pub fn setup_anim(e: &Entity, model: &CubeModel, partial: f32) -> PoseOverrides {
    let mut o = PoseOverrides::default();
    for (side, name, right, index) in [
        (0, "left_paddle", false, PADDLE_LEFT_METADATA_INDEX),
        (1, "right_paddle", true, PADDLE_RIGHT_METADATA_INDEX),
    ] {
        let Some(paddle) = find_part(&model.root, name) else {
            continue;
        };
        let time = rowing_time(
            boolean(&e.metadata, index),
            e.boat.paddle_positions[side],
            partial,
        );
        let (x, y) = animate_paddle(time, right);
        o.set_rotation(
            paddle.id,
            Vec3::new(x - paddle.rotation.x, y - paddle.rotation.y, 0.0),
        );
    }
    o
}

/// A part by name (the dump interns part names, so the renderer matches by id).
fn find_part<'a>(part: &'a ModelPart, name: &str) -> Option<&'a ModelPart> {
    if part.name == name {
        return Some(part);
    }
    part.children.iter().find_map(|c| find_part(c, name))
}

/// `AbstractBoatRenderer.submit`'s rotations, carried by the generic entity path: the 0.375
/// lift before the yaw (`pre_translate`), the hurt wobble after it, and the bubble-column
/// rocking (a rotation about the (1, 0, 1) axis by `bubbleAngle` degrees, expressed as the
/// conjugation `Ry(-PI/4) * Rx(angle) * Ry(PI/4)` whose inner axis `Ry(-PI/4)` maps onto that
/// diagonal). The trailing `scale(-1, -1, 1)` and 90-degree yaw are the path's own mirroring
/// plus the baked root rotation of [`model`].
pub fn body_transform(e: &Entity, partial: f32) -> BodyTransform {
    let mut b = BodyTransform {
        // The lift before the yaw: `poseStack.translate(0, 0.375, 0)` (world-space, so the
        // renderer's pre-yaw translation carries it).
        pre_translate: Vec3::new(0.0, 0.375, 0.0),
        ..Default::default()
    };
    // `AbstractBoatRenderer.extractRenderState`: the hurt time counts the partial tick down,
    // the damage down from 10.
    let hurt = var_int(&e.metadata, HURT_METADATA_INDEX).unwrap_or(0) as f32 - partial;
    let damage = (float_at(&e.metadata, DAMAGE_METADATA_INDEX).unwrap_or(0.0) - partial).max(0.0);
    let hurt_dir = var_int(&e.metadata, HURT_DIR_METADATA_INDEX).unwrap_or(1);
    if hurt > 0.0 {
        b.push(BodyOp::RotateX(
            hurt.sin() * hurt * damage / 10.0 * hurt_dir as f32,
        ));
    }
    let bubble = e.boat.bubble_angle(partial);
    if !e.boat.underwater && bubble != 0.0 {
        // `rotate(new Quaternionf().setAngleAxis(bubbleAngle * PI/180, 1, 0, 1))`: a rotation
        // about the (1, 0, 1) diagonal, as the conjugation `Ry(-PI/4) * Rx(angle) * Ry(PI/4)`
        // (whose inner x rotation's axis `Ry(-PI/4)` maps onto that diagonal).
        let quarter = std::f32::consts::FRAC_PI_4;
        b.push(BodyOp::RotateY(-quarter));
        b.push(BodyOp::RotateX(bubble.to_radians()));
        b.push(BodyOp::RotateY(quarter));
    }
    b
}

fn float_at(m: &Metadata, index: u8) -> Option<f32> {
    match m.get(index).map(|e| &e.value) {
        Some(MetadataValue::Float(v)) => Some(*v),
        _ => None,
    }
}

/// The boat's model: the dump's layer with the texture its renderer draws and vanilla's
/// trailing 90-degree yaw baked into the root part (`AbstractBoatRenderer.submit`'s
/// `rotateDegrees(Axis.YP, 90)` after the mirrored scale; the part matrices apply it after
/// the mirroring, exactly where vanilla applies it). `None` when the dump lacks the layer
/// (the caller falls back to the dimension box). Cached per entry, shared `Arc`s.
/// The per-layer model cache behind [`model`].
type ModelCache = std::sync::Mutex<ahash::AHashMap<&'static str, Option<Arc<CubeModel>>>>;

pub fn model(entry: &BoatEntry) -> Option<Arc<CubeModel>> {
    static MODELS: OnceLock<ModelCache> = OnceLock::new();
    let models = MODELS.get_or_init(|| {
        let mut map = ahash::AHashMap::with_capacity(BOATS.len());
        for (_, e) in BOATS {
            map.insert(e.layer, build_model(e));
        }
        std::sync::Mutex::new(map)
    });
    models.lock().unwrap().get(entry.layer).cloned().flatten()
}

/// The build behind [`model`]: dump layer, texture, root rotation.
fn build_model(entry: &BoatEntry) -> Option<Arc<CubeModel>> {
    let model = crate::model_dump::library()?.get(entry.layer, 1.0)?;
    let mut m = (*model).clone();
    m.texture = Identifier::minecraft(entry.texture);
    // `AbstractBoatRenderer.submit`: `poseStack.scale(-1, -1, 1); rotateDegrees(Axis.YP, 90)`.
    // The generic path applies the mirrored scale; the root part's rotation is applied after
    // it (part matrices multiply onto the scaled root), which is that same order.
    m.root.rotation.y += std::f32::consts::FRAC_PI_2;
    Some(Arc::new(m))
}

/// `BoatRenderer.submitTypeAdditions`: the water patch overlay for a boat that is not under
/// water (a raft has none).
pub fn water_patch_overlay(entry: &BoatEntry, state: &BoatState) -> Option<EntityOverlay> {
    if entry.raft || state.underwater {
        return None;
    }
    Some(EntityOverlay {
        model: water_patch_model(),
        tint: [1.0; 3],
    })
}

/// `BoatModel.createWaterPatch`: the hull-sized plate at the waterline. Haste's renderer
/// draws this model depth-only (it hides the water inside the hull, vanilla's
/// `RenderTypes.waterMask`), so its texture is a marker in Haste's own namespace and its
/// never-sampled layout is 1x1.
pub fn water_patch_model() -> Arc<CubeModel> {
    static MODEL: OnceLock<Arc<CubeModel>> = OnceLock::new();
    MODEL
        .get_or_init(|| {
            let root = ModelPart {
                name: "root".to_owned(),
                id: haste_core::parts::ROOT,
                pivot: Vec3::ZERO,
                rotation: Vec3::ZERO,
                visible: true,
                cubes: Vec::new(),
                children: vec![ModelPart {
                    name: "water_patch".to_owned(),
                    id: haste_core::parts::intern("water_patch"),
                    pivot: Vec3::new(0.0, -3.0, 1.0),
                    rotation: Vec3::new(std::f32::consts::FRAC_PI_2, 0.0, 0.0),
                    visible: true,
                    cubes: vec![Cube {
                        origin: Vec3::new(-14.0, -9.0, -3.0),
                        size: Vec3::new(28.0, 16.0, 3.0),
                        inflate: 0.0,
                        uv: [0, 0],
                        mirror: false,
                    }],
                    children: Vec::new(),
                }],
            };
            Arc::new(CubeModel {
                texture: Identifier::new("haste", "boat/water_patch"),
                tex_width: 1,
                tex_height: 1,
                root,
            })
        })
        .clone()
}
