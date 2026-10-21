//! Entity tracking from packet-derived events, interpolation, entity and player models. Owner: entities.
//!
//! See docs/INTERFACES.md for the contract this crate implements and AGENTS.md for the rules.

pub mod animation;
pub mod armor_stand;
pub mod arrow;
pub mod avatar;
pub mod boat;
pub mod camera_view;
pub mod decoration;
pub mod dims;
pub mod display;
pub mod dragon_fireball;
pub mod entity;
pub mod entity_events;
pub mod evoker_fangs;
pub mod evoker_fangs_model;
pub mod eye;
mod firework_rocket;
pub mod item_entity;
pub mod leash;
pub mod leash_knot;
pub mod lerp;
pub mod living;
pub mod llama_spit_model;
pub mod local_player;
pub mod metadata;
pub mod minecart;
pub mod model_dump;
pub mod models;
pub mod mount_equipment;
pub mod new_minecart_behavior;
pub mod registry;
pub mod render;
mod ride_tick;
pub mod riding;
pub mod skin;
pub mod sleeping;
pub mod spin_attack;
pub mod store;
pub mod stuck;
pub mod thrown_item;
pub mod trident;
pub mod using_item;
pub mod variants;
pub mod walk_swing;

#[cfg(test)]
mod leash_tests;
#[cfg(test)]
mod skin_tests;
#[cfg(test)]
mod stuck_tests;
#[cfg(test)]
mod tests;

pub use decoration::PaintingVariant;
pub use dims::{Dimensions, EntityDimensions};
pub use local_player::{equipment_by_slot, LocalTick, MirroredSwing};
pub use metadata::{Metadata, MobFlags, SharedFlags};
pub use riding::{passenger_seat, PassengerKind, SeatPoint, SeatQuery};
pub use store::{Entity, EntityStore};
pub use stuck::{stuck_models, StuckModels, StuckState};
pub use variants::{MobVariants, VariantAssets, VariantModel, VARIANT_REGISTRIES};
pub use walk_swing::{SwingState, WalkAnimationState};
