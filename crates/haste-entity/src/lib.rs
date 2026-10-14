//! Entity tracking from packet-derived events, interpolation, entity and player models. Owner: entities.
//!
//! See docs/INTERFACES.md for the contract this crate implements and AGENTS.md for the rules.

pub mod animation;
pub mod dims;
pub mod lerp;
pub mod metadata;
pub mod model_dump;
pub mod models;
pub mod registry;
pub mod store;

#[cfg(test)]
mod tests;

pub use dims::{Dimensions, EntityDimensions};
pub use metadata::{Metadata, SharedFlags};
pub use store::{Entity, EntityStore};
