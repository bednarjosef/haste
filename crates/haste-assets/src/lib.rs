//! Textures, block/item models, blockstate definitions, fonts and GUI sprites read from the player's client jar and the assets index. Owner: renderer.
//!
//! See docs/INTERFACES.md for the contract this crate implements and AGENTS.md for the rules.
//!
//! Pieces derived from Pomme (GPL-3.0) carry a header and a THIRD_PARTY.md entry:
//! `assets.rs` (asset index over jar + objects dir), `model.rs` (model JSON parsing
//! and quad baking), `multipart.rs` (blockstate variants/multipart with `when`),
//! `atlas.rs` (atlas stitching + mip chain), `bake.rs` (per-state baking).

pub mod assets;
pub mod atlas;
pub mod bake;
pub mod equipment;
pub mod font;
pub mod item;
pub mod model;
pub mod multipart;
pub mod packstack;
pub mod particle;

pub use assets::{
    frame_at, Animation, AnimationFrame, AnimationMeta, AnimationSchedule, Assets, ModelJson,
    SpriteMeta, TextureMeta,
};
pub use atlas::{mean_linear, AnimatedSprite, AtlasBuilder, AtlasRegion, AtlasRegions};
pub use bake::{BakedBlockModels, BlocksJsonStates, RenderLayer};
pub use equipment::{parse_equipment_info, EquipmentInfo, EquipmentLayer};
pub use font::{
    for_each_formatted, BitmapFont, Glyph, GlyphKind, HexGlyph, TextFormat, LEGACY_COLORS,
};
pub use item::{GeneratedLayer, ItemGeometry, ItemModel, ItemModelPart, PartTransformation};
pub use model::{BakedQuad, Direction, DisplayTransform, ModelRef};
pub use multipart::{BlockStateDefinition, MultipartCase, MultipartCondition, VariantEntry};
pub use packstack::{
    discover_packs, pack_compatibility, PackCompatibility, PackInfo, PackStack,
    RESOURCE_PACK_FORMAT,
};
pub use particle::{ParticleAtlas, ParticleDefinition};
