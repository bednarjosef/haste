//! Chunk storage, paletted containers, lighting data, block updates. Owner: world-and-data.
//!
//! See docs/INTERFACES.md for the contract this crate implements and AGENTS.md for the rules.

pub mod chunk;
pub mod light;
pub mod palette;
mod sky_light;
pub mod snapshot;
pub mod wire;
pub mod world;
pub mod world_error;

pub use chunk::{Chunk, ChunkLight, Section};
pub use palette::{
    BiomeContainer, BlockContainer, DecodedSection, PaletteError, PalettedContainer,
};
pub use world::World;
pub use world::WorldMemoryStats;
pub use world_error::WorldError;
