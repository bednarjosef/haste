//! Cube-list entity models, the shape of vanilla's `ModelPart` trees, expressed as data so
//! `haste-entity` can define/animate models and `haste-render` can draw any of them generically.
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::ids::RegistryIndex;
use crate::Identifier;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CubeModel {
    pub texture: Identifier,
    pub tex_width: u32,
    pub tex_height: u32,
    pub root: ModelPart,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelPart {
    pub name: String,
    /// Pivot in model units (1/16 block), vanilla's `PartPose` offset.
    pub pivot: Vec3,
    /// Rotation in radians, applied around the pivot (x, y, z order like vanilla).
    pub rotation: Vec3,
    pub visible: bool,
    pub cubes: Vec<Cube>,
    pub children: Vec<ModelPart>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cube {
    /// Origin and size in model units, relative to the part pivot.
    pub origin: Vec3,
    pub size: Vec3,
    /// Uniform inflation (vanilla `CubeDeformation`).
    pub inflate: f32,
    /// Top-left texture offset in pixels (vanilla box UV layout).
    pub uv: [u32; 2],
    pub mirror: bool,
}

/// Per-frame pose overrides, keyed by part name, so animation state stays in `haste-entity`.
#[derive(Clone, Debug, Default)]
pub struct PoseOverrides {
    pub rotations: Vec<(String, Vec3)>,
    pub hidden: Vec<String>,
}

/// One step of the whole-model transform `LivingEntityRenderer.setupRotations` builds (plus
/// `AvatarRenderer`'s flying/swimming pitch), in the order vanilla applies it (`poseStack` calls
/// compose left to right, so the first op is outermost).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BodyOp {
    RotateX(f32),
    RotateY(f32),
    RotateZ(f32),
    /// A translation in blocks, in the pre-scale (world-oriented) space.
    Translate(Vec3),
}

/// The whole-model transform applied between the body yaw and the model scale. Fixed-size and
/// `Copy`: no per-frame allocation (`setupRotations` never emits more than four steps).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyTransform {
    pub ops: [BodyOp; 4],
    pub len: u8,
    /// `LivingEntityRenderer.setupRotations`: a sleeping entity skips the `180 - bodyRot` yaw and
    /// uses its own rotation sequence instead.
    pub suppress_yaw: bool,
    /// A translation applied before the yaw (sleeping's bed offset), in blocks.
    pub pre_translate: Vec3,
}

impl Default for BodyTransform {
    fn default() -> Self {
        BodyTransform {
            ops: [BodyOp::RotateY(0.0); 4],
            len: 0,
            suppress_yaw: false,
            pre_translate: Vec3::ZERO,
        }
    }
}

impl BodyTransform {
    pub fn push(&mut self, op: BodyOp) {
        if (self.len as usize) < self.ops.len() {
            self.ops[self.len as usize] = op;
            self.len += 1;
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &BodyOp> {
        self.ops[..self.len as usize].iter()
    }
}

/// One drawable entity instance handed to the renderer each frame (already interpolated).
#[derive(Clone, Debug)]
pub struct EntityInstance {
    pub id: crate::EntityId,
    pub model: std::sync::Arc<CubeModel>,
    pub pos: glam::DVec3,
    pub yaw: f32,
    pub pitch: f32,
    pub head_yaw: f32,
    pub scale: f32,
    pub pose: PoseOverrides,
    /// Whole-model rotations (`LivingEntityRenderer.setupRotations`: death tilt, sleeping,
    /// upside-down, player flying/swimming pitch), applied by the renderer between the body yaw
    /// and the model scale.
    pub body: BodyTransform,
    /// Per-axis subclass scale (`EntityRenderer.scale`, e.g. `CreeperRenderer.scale`'s swell
    /// wobble), multiplied into the model scale. `Vec3::ONE` for most entities.
    pub body_scale: Vec3,
    pub skin: Option<Identifier>,
    /// The name tag's text when it should show (`EntityRenderer.extractNameTags`: within 64
    /// blocks and `shouldShowName`), `None` otherwise. For players this is the player name.
    pub nametag: Option<String>,
    /// The scoreboard's below-name line (`Entity.belowNameDisplay`), drawn under the name tag.
    pub below_name: Option<String>,
    /// `EntityRenderer.shadowRadius` for this entity's renderer, already scaled by the age
    /// (`MobRenderer.getShadowRadius` * `ageScale`) or living scale; 0 draws no shadow.
    pub shadow_radius: f32,
    /// `EntityRenderer.shadowStrength` (1.0, or 0.75 for items/experience orbs).
    pub shadow_strength: f32,
    /// `Entity.isInvisible()`: no shadow (`extractShadow` skips it) and no name tag.
    pub invisible: bool,
    /// `Entity.isDiscrete()` (crouching): the name tag hides past 32 blocks and drops the
    /// see-through pass (`LivingEntityRenderer.shouldShowName`, `submitNameTag`).
    pub is_discrete: bool,
    /// The name tag's anchor above the entity's feet: `EntityAttachment.NAME_TAG`'s fallback
    /// `AT_HEIGHT` (`Entity.getBbHeight`, 2.05 for the sniffer).
    pub name_tag_y: f32,
    /// Hurt flash strength in 0..1 (1 just after the hit, easing to 0 over the 10 hurt
    /// ticks; death holds it at 1 for the 20-tick death animation). The renderer mixes
    /// red into the entity's colour for values above 0.
    pub hurt: f32,
    /// Overlay layers drawn on top of the base model with the same transform and pose
    /// (vanilla's `RenderLayer`s: sheep wool and undercoat, horse markings, cat/wolf
    /// collars, the tropical fish pattern). Fixed-size: sheep needs two, everything else
    /// at most one; empty for almost every mob, so no per-frame allocation.
    pub overlays: [Option<EntityOverlay>; 2],
    /// A dropped item entity (`minecraft:item`) drawn as the item itself (spinning and
    /// bobbing); the model above is ignored for these.
    pub item: Option<DroppedItem>,
    /// Set for hanging decorations (item frames, paintings): drawn specially instead of from
    /// `model` (vanilla's `ItemFrameRenderer` / `PaintingRenderer`). `None` for every other
    /// entity, which the renderer draws from its cube model.
    pub decoration: Option<Decoration>,
    /// The item held in each hand, by item registry index: `[right (main), left (off)]`.
    /// `None` is an empty hand (vanilla `ItemInHandLayer` draws nothing for it).
    pub hands: [Option<RegistryIndex>; 2],
    /// Armour and other equipment layers the renderer draws, already resolved to the equipment
    /// asset each item names (vanilla `HumanoidArmorLayer`). Empty for entities with none.
    pub equipment: Vec<EquipmentPiece>,
    /// The head-slot item when it is not armour (vanilla `CustomHeadLayer`: pumpkins, mob and
    /// player heads). `None` when the head is bare or wears a helmet (drawn by `equipment`).
    pub head_item: Option<RegistryIndex>,
}

/// An equipment slot, matching vanilla `EquipmentSlot`'s wire order (the ordinals the
/// `set_equipment` packet uses).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EquipmentSlot {
    MainHand,
    OffHand,
    Feet,
    Legs,
    Chest,
    Head,
    Body,
    Saddle,
}

impl EquipmentSlot {
    /// The slot for a `set_equipment` wire ordinal, or `None` for an id we do not know.
    pub fn from_wire(id: u8) -> Option<EquipmentSlot> {
        use EquipmentSlot::*;
        Some(match id {
            0 => MainHand,
            1 => OffHand,
            2 => Feet,
            3 => Legs,
            4 => Chest,
            5 => Head,
            6 => Body,
            7 => Saddle,
            _ => return None,
        })
    }

    /// Whether the slot is humanoid armour (vanilla `EquipmentSlot.Type.HUMANOID_ARMOR`).
    pub fn is_armor(self) -> bool {
        matches!(
            self,
            EquipmentSlot::Feet | EquipmentSlot::Legs | EquipmentSlot::Chest | EquipmentSlot::Head
        )
    }
}

/// One worn equipment piece, resolved by `haste-entity` from the entity's equipment and the
/// item's default `equippable` component.
#[derive(Clone, Debug, PartialEq)]
pub struct EquipmentPiece {
    pub slot: EquipmentSlot,
    /// The item's registry index (`None` for an empty slot).
    pub item: Option<RegistryIndex>,
    /// The `equippable.asset_id` (`minecraft:iron` -> `equipment/iron.json`), if the item has one.
    pub asset: Option<Identifier>,
    /// The stack's `dyed_color` component (ARGB) when dyed (leather armour); `None` otherwise.
    pub dye: Option<u32>,
}

/// What a dropped item entity carries: which item to draw (index into `minecraft:item`,
/// resolved against the renderer's item names) and the stack size (stacks draw 1-5
/// copies). `age` is the item entity's age in ticks plus the partial tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DroppedItem {
    pub item: RegistryIndex,
    pub count: i32,
    pub age: f32,
}

/// One overlay layer: the model (usually a copy of the base shape with its own texture,
/// sometimes the base model itself with one, like vanilla's wolf collar and horse
/// markings) and the tint vanilla multiplies into its vertex colours
/// (`coloredCutoutModelCopyLayerRender`'s colour, `DyeColor.getTextureDiffuseColor` for
/// dyed layers; white when the layer is untinted).
#[derive(Clone, Debug)]
pub struct EntityOverlay {
    pub model: std::sync::Arc<CubeModel>,
    /// Linear RGB multiplier, 0..1 per channel.
    pub tint: [f32; 3],
}

/// A hanging decoration the renderer draws itself rather than from a cube model.
#[derive(Clone, Debug, PartialEq)]
pub enum Decoration {
    ItemFrame(ItemFrameDecoration),
    Painting(PaintingDecoration),
}

/// The state `ItemFrameRenderer.extractRenderState` collects for an item frame.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemFrameDecoration {
    /// The direction the frame faces (`ItemFrame.getDirection`, from the spawn packet's data).
    pub facing: crate::Direction,
    /// `ItemFrame.getRotation` 0..7; the item turns `rotation * 45` degrees.
    pub rotation: u8,
    /// Glow item frames light the frame at least block-light 5 and the item full-bright.
    pub glow: bool,
    /// The framed item (`DATA_ITEM`), by item registry index.
    pub item: Option<crate::RegistryIndex>,
}

/// The state `PaintingRenderer.extractRenderState` collects for a painting.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintingDecoration {
    /// The direction the painting faces (`Painting.getDirection`).
    pub facing: crate::Direction,
    /// The variant's size in blocks (`PaintingVariant.width`/`height`).
    pub width: u8,
    pub height: u8,
    /// The front sprite's texture id, `None` for a variant we do not know (draw the back only).
    pub front: Option<Identifier>,
}
