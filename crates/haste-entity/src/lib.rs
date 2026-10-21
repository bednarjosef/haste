//! Entity tracking from packet-derived events, interpolation, entity and player models. Owner: entities.
//!
//! See docs/INTERFACES.md for the contract this crate implements and AGENTS.md for the rules.

pub mod animation;
pub mod armor_stand;
pub mod arrow;
pub mod avatar;
pub mod boat;
pub mod decoration;
pub mod dims;
pub mod dragon_fireball;
pub mod fishing_bobber;
pub mod fishing_hook;
pub mod item_entity;
pub mod lerp;
pub mod local_player;
pub mod metadata;
pub mod minecart;
pub mod model_dump;
pub mod models;
pub mod registry;
pub mod render;
pub mod riding;
pub mod skin;
pub mod store;
pub mod thrown_item;
pub mod using_item;
pub mod variants;
pub mod walk_swing;

#[cfg(test)]
mod skin_tests;
#[cfg(test)]
mod tests;

pub use decoration::PaintingVariant;
pub use dims::{Dimensions, EntityDimensions};
pub use local_player::{equipment_by_slot, LocalTick, MirroredSwing};
pub use metadata::{Metadata, MobFlags, SharedFlags};
pub use riding::{passenger_seat, PassengerKind, SeatPoint, SeatQuery};
pub use store::{Entity, EntityStore};
pub use variants::{MobVariants, VariantAssets, VariantModel, VARIANT_REGISTRIES};
pub use walk_swing::{SwingState, WalkAnimationState};
