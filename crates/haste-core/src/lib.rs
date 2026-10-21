//! Shared types and the interfaces between Haste's crates.
//!
//! This crate is owned by the lead. Workstreams consume it; they do not edit it. If you need a new
//! shared type or a change here, ask (see AGENTS.md). Keep this crate free of logic beyond small
//! conversions: no I/O, no game rules.
//!
//! Tiers (from the spikes): tier 1 state lives in Rust (`haste-world`, `haste-entity`,
//! `haste-physics`); tier 2 are the Java proxies of those objects (`haste-jvm`); tier 3 is real
//! vanilla Java for stateless classes. The types here are the vocabulary all tiers share.

pub mod chunk;
pub mod decoration;
pub mod display;
pub mod entity;
pub mod game;
pub mod gui;
pub mod ident;
pub mod ids;
pub mod input;
pub mod math;
pub mod model;
pub mod parts;
pub mod paths;
pub mod pos;
pub mod text;
pub mod view;

pub use ident::Identifier;
pub use ids::{BiomeId, BlockStateId, EntityId, RegistryIndex};
pub use math::{Aabb, DVec3, IVec3, Vec3};
pub use pos::{BlockPos, ChunkPos, Direction, SectionPos};
pub use text::Component;
