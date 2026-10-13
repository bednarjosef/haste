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
    /// A dropped item entity (`minecraft:item`) drawn as the item itself (spinning and
    /// bobbing); the model above is ignored for these.
    pub item: Option<DroppedItem>,
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
