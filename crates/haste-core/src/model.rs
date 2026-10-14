//! Cube-list entity models, the shape of vanilla's `ModelPart` trees, expressed as data so
//! `haste-entity` can define/animate models and `haste-render` can draw any of them generically.
use glam::Vec3;
use serde::{Deserialize, Serialize};

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
    pub skin: Option<Identifier>,
    pub nametag: Option<String>,
    /// Hurt flash strength in 0..1 (1 just after the hit, easing to 0 over the 10 hurt
    /// ticks; death holds it at 1 for the 20-tick death animation). The renderer mixes
    /// red into the entity's colour for values above 0.
    pub hurt: f32,
    /// Overlay layers drawn on top of the base model with the same transform and pose
    /// (vanilla's `RenderLayer`s: sheep wool and undercoat, horse markings, cat/wolf
    /// collars, the tropical fish pattern). Fixed-size: sheep needs two, everything else
    /// at most one; empty for almost every mob, so no per-frame allocation.
    pub overlays: [Option<EntityOverlay>; 2],
    /// Set for hanging decorations (item frames, paintings): drawn specially instead of from
    /// `model` (vanilla's `ItemFrameRenderer` / `PaintingRenderer`). `None` for every other
    /// entity, which the renderer draws from its cube model.
    pub decoration: Option<Decoration>,
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
