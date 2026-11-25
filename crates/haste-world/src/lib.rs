//! Chunk storage, paletted containers, lighting data, block updates. Owner: world-and-data.
//!
//! See docs/INTERFACES.md for the contract this crate implements and docs/AGENTS.md for the rules.

pub mod chunk;
pub mod light;
pub mod light_stats;
pub mod palette;
mod sky_light;
pub mod snapshot;
pub mod wire;
pub mod world;
pub mod world_error;

pub use chunk::{Chunk, ChunkLight, Section};
pub use light_stats::SkyLightStats;
pub use palette::{
    BiomeContainer, BlockContainer, DecodedSection, PaletteError, PalettedContainer,
};
pub use world::{World, WorldMemoryStats};
pub use world_error::WorldError;
