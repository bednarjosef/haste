use serde::{Deserialize, Serialize};

/// Global block-state id as used on the wire and in chunk palettes (from `blocks.json`).
#[derive(
    Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default, Serialize, Deserialize,
)]
#[repr(transparent)]
pub struct BlockStateId(pub u32);

impl BlockStateId {
    /// `minecraft:air` is always state 0 in vanilla's block state ordering.
    pub const AIR: BlockStateId = BlockStateId(0);
}

/// Biome registry index (per the synced `minecraft:worldgen/biome` registry).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
#[repr(transparent)]
pub struct BiomeId(pub u32);

/// Network entity id (the `int` the server assigns, not the UUID).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[repr(transparent)]
pub struct EntityId(pub i32);

/// Index into a static registry from `registries.json` (entity types, items, particles, ...).
/// Always paired with the registry it indexes; never mix registries.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[repr(transparent)]
pub struct RegistryIndex(pub u32);
