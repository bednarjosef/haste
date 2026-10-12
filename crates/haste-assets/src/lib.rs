//! Textures, block/item models, blockstate definitions, fonts and GUI sprites read from the player's client jar and the assets index. Owner: WS3.
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
pub mod model;
pub mod multipart;

pub use assets::{AnimationMeta, Assets, BitmapFont, Glyph, ModelJson, SpriteMeta, TextureMeta};
pub use atlas::{AtlasBuilder, AtlasRegion, AtlasRegions};
pub use bake::{BakedBlockModels, BlocksJsonStates, RenderLayer};
pub use model::{BakedQuad, Direction, ModelRef};
pub use multipart::{BlockStateDefinition, MultipartCase, MultipartCondition, VariantEntry};
