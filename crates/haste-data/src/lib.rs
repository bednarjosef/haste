//! Runtime registries loaded from the data-generator reports and oracle dumps in the cache (block states, registries, block shapes). Owner: world-and-data.
//!
//! See docs/INTERFACES.md for the contract this crate implements and AGENTS.md for the rules.

mod block_entity_types;
mod blocks;
mod components;
mod loader;
mod physics;
mod registries;
mod shapes;
mod sounds;

#[cfg(test)]
mod tests;

pub use block_entity_types::BlockEntityTypes;
pub use blocks::BlockStates;
pub use components::{AttributeModifier, Consumable, ItemComponents, ItemEquippable, UseEffects};
pub use loader::{missing_reports, GameData, RUNTIME_REPORTS};
pub use physics::{BlockProperties, BlockProps};
pub use registries::Registries;
pub use shapes::BlockShapes;
pub use sounds::{BlockSoundType, BlockSoundTypes};
