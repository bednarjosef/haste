//! Vocabulary for entity tracking shared by `haste-protocol` (which decodes it), `haste-entity`
//! (which owns the entities) and `haste-render` (which draws them).
use glam::DVec3;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Component, EntityId, RegistryIndex};

/// What `add_entity` tells us.
#[derive(Clone, Debug)]
pub struct EntitySpawn {
    pub id: EntityId,
    pub uuid: Uuid,
    /// Index into `minecraft:entity_type`.
    pub kind: RegistryIndex,
    pub pos: DVec3,
    /// Degrees, vanilla convention (yaw around Y, pitch; positive pitch looks down).
    pub yaw: f32,
    pub pitch: f32,
    pub head_yaw: f32,
    pub data: i32,
    /// Blocks per tick.
    pub velocity: DVec3,
}

/// Entity metadata ("synched entity data") value. Indices are per entity class; `haste-entity`
/// interprets them. Variants mirror the wire type ids in order.
#[derive(Clone, Debug, PartialEq)]
pub enum MetadataValue {
    Byte(i8),
    VarInt(i32),
    VarLong(i64),
    Float(f32),
    String(String),
    Component(Component),
    OptionalComponent(Option<Component>),
    ItemStack(ItemStackData),
    Boolean(bool),
    Rotations([f32; 3]),
    BlockPos(crate::BlockPos),
    OptionalBlockPos(Option<crate::BlockPos>),
    Direction(crate::Direction),
    OptionalUuid(Option<Uuid>),
    BlockState(crate::BlockStateId),
    OptionalBlockState(Option<crate::BlockStateId>),
    Pose(Pose),
    /// Any type we do not model yet: wire type id + raw bytes so nothing is lost.
    Other { type_id: i32, raw: bytes::Bytes },
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetadataEntry {
    pub index: u8,
    pub value: MetadataValue,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum Pose {
    Standing = 0, FallFlying = 1, Sleeping = 2, Swimming = 3, SpinAttack = 4, Crouching = 5,
    LongJumping = 6, Dying = 7, Croaking = 8, UsingTongue = 9, Sitting = 10, Roaring = 11,
    Sniffing = 12, Emerging = 13, Digging = 14, Sliding = 15, Shooting = 16, Inhaling = 17,
}
impl Pose {
    pub fn from_id(i: i32) -> Option<Pose> {
        use Pose::*;
        Some(match i { 0 => Standing, 1 => FallFlying, 2 => Sleeping, 3 => Swimming, 4 => SpinAttack, 5 => Crouching, 6 => LongJumping, 7 => Dying, 8 => Croaking, 9 => UsingTongue, 10 => Sitting, 11 => Roaring, 12 => Sniffing, 13 => Emerging, 14 => Digging, 15 => Sliding, 16 => Shooting, 17 => Inhaling, _ => return None })
    }
}

/// Minimal item stack: enough for the hotbar and for passing through to tier 3. Components are
/// kept raw (network encoding) until tier 3 needs them.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ItemStackData {
    pub count: i32,
    /// Index into `minecraft:item`; `None` for the empty stack.
    pub item: Option<RegistryIndex>,
    pub components_raw: bytes::Bytes,
}
impl ItemStackData {
    pub fn is_empty(&self) -> bool { self.count <= 0 || self.item.is_none() }
}

/// Player-list entry data the HUD/tab list and nametags need.
#[derive(Clone, Debug)]
pub struct PlayerInfo {
    pub uuid: Uuid,
    pub name: String,
    pub display_name: Option<Component>,
    pub latency: i32,
    pub game_mode: Option<crate::game::GameMode>,
    pub listed: bool,
    pub skin_texture_url: Option<String>,
}
