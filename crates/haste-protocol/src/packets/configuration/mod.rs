//! Configuration state: registries, tags, feature flags, resource packs.
pub mod clientbound;
pub mod serverbound;
pub use clientbound::ClientboundConfiguration;
pub use serverbound::ServerboundConfiguration;
