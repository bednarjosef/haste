//! Entity tracking from packet-derived events, interpolation, entity and player models. Owner: entities.
//!
//! See docs/INTERFACES.md for the contract this crate implements and AGENTS.md for the rules.

pub mod animation;
pub mod avatar;
pub mod decoration;
pub mod dims;
pub mod lerp;
pub mod metadata;
pub mod model_dump;
pub mod models;
pub mod registry;
pub mod riding;
pub mod skin;
pub mod store;
pub mod variants;

#[cfg(test)]
mod skin_tests;
#[cfg(test)]
mod tests;

pub use decoration::PaintingVariant;
pub use dims::{Dimensions, EntityDimensions};
pub use metadata::{Metadata, MobFlags, SharedFlags};
pub use riding::{passenger_seat, PassengerKind, SeatPoint, SeatQuery};
pub use store::{Entity, EntityStore};
pub use variants::{MobVariants, VariantAssets, VariantModel, VARIANT_REGISTRIES};
