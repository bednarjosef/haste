//! Clientbound play packets.
//!
//! Layouts from the minecraft.wiki "Java Edition protocol/Packets" page for
//! 26.3; see NOTES.md for what was checked against the local server. Packets
//! whose body we do not interpret yet keep it raw (`body: Bytes`).
//!
//! The packets are grouped into one module per vanilla concept, the way the
//! decompiled client's `net.minecraft.network.protocol.game` classes group
//! them: [`common`] (shared types), [`level_chunks`] (world, chunks and
//! block updates), [`entities`] and [`entity_movement`], [`player`],
//! [`player_info`] (the tab list), [`inventory`] (containers, recipes and
//! advancements), [`chat`] and [`chat_ui`] (chat, titles, scoreboards and
//! commands), [`sound_particles`], and [`misc`]. Every type is re-exported
//! here, so `clientbound::Foo` paths are unchanged.

use bytes::Bytes;
use haste_core::chunk::{BlockChange, BlockEntityData as ChunkBlockEntity, ChunkData, LightData};
use haste_core::entity::{EntitySpawn, ItemStackData, MetadataEntry};
use haste_core::{
    BlockPos, BlockStateId, ChunkPos, Component, DVec3, EntityId, Identifier, RegistryIndex,
    SectionPos,
};
use uuid::Uuid;

use crate::metadata::Metadata;
pub use crate::packets::configuration::clientbound::{
    ClearDialog, CookieRequest, CustomPayload, CustomReportDetails, Disconnect, KeepAlive, Ping,
    PostEffects, ResourcePackPop, ResourcePackPush, ServerLinks, StoreCookie, Transfer, UpdateTags,
};
use crate::packets::{packet, packet_enum, Packet};
use crate::wire::particle::Particle;
use crate::wire::{
    Angle, ByteArray, Codec, DecodeError, DecodeResult, IdOr, List, LpVec3, Opt, ProfileProperty,
    RawNbt, Reader, Rest, SoundEvent, VarInt, VarLong, Wire, Writer,
};

mod chat;
mod chat_ui;
mod common;
mod entities;
mod entity_movement;
mod inventory;
mod level_chunks;
mod map;
mod misc;
mod player;
mod player_info;
mod sound_particles;

pub use chat::*;
pub use chat_ui::*;
pub use common::*;
pub use entities::*;
pub use entity_movement::*;
pub use inventory::*;
pub use level_chunks::*;
pub use map::*;
pub use misc::*;
pub use player::*;
pub use player_info::*;
pub use sound_particles::*;

#[cfg(test)]
mod tests;

packet_enum! {
    /// Clientbound play packets, in `packets.json` id order. Ids without a
    /// struct (none in 26.3) or failed decodes arrive as `Unknown`.
    ClientboundPlay {
        BundleDelimiter, AddEntity, Animate, AwardStats, BlockChangedAck, BlockDestruction, BlockEntityData,
        BlockEvent, BlockUpdate, BossEvent, ChangeDifficulty, ChunkBatchFinished, ChunkBatchStart, ChunksBiomes,
        ClearTitles, CommandSuggestions, Commands, ContainerClose, ContainerSetContent, ContainerSetData,
        ContainerSetSlot, CookieRequest, Cooldown, CustomChatCompletions, CustomPayload, DamageEvent, DebugBlockValue,
        DebugChunkValue, DebugEntityValue, DebugEvent, DebugSample, DeleteChat, Disconnect, DisguisedChat,
        EntityEvent, EntityPositionSync, Explode, AddTransientBlock, ForgetLevelChunk, GameEvent, GameRuleValues,
        GameTestHighlightPos, MountScreenOpen, HurtAnimation, InitializeBorder, KeepAlive, LevelChunkWithLight,
        LevelEvent, LevelParticles, LightUpdate, Login, LowDiskSpaceWarning, MapItemData, MerchantOffers,
        MoveEntityPos, MoveEntityPosRot, MoveMinecartAlongTrack, MoveEntityRot, MoveVehicle, OpenBook, OpenScreen,
        OpenSignEditor, Ping, PongResponse, PlaceGhostRecipe, PlayerAbilities, PlayerChat, PlayerCombatEnd,
        PlayerCombatEnter, PlayerCombatKill, PlayerInfoRemove, PlayerInfoUpdate, PlayerLookAt, PlayerPosition,
        PlayerRotation, RecipeBookAdd, RecipeBookRemove, RecipeBookSettings, RemoveEntities, RemoveMobEffect,
        ResetScore, ResourcePackPop, ResourcePackPush, PostEffects, Respawn, RotateHead, SectionBlocksUpdate,
        SelectAdvancementsTab, ServerData, SetActionBarText, SetBorderCenter, SetBorderLerpSize, SetBorderSize,
        SetBorderWarningDelay, SetBorderWarningDistance, SetCamera, SetChunkCacheCenter, SetChunkCacheRadius,
        SetCursorItem, SetDefaultSpawnPosition, SetDisplayObjective, SetEntityData, SetEntityLink, SetEntityMotion,
        SetEquipment, SetExperience, SetHealth, SetHeldSlot, SetObjective, SetPassengers, SetPlayerInventory,
        SetPlayerTeam, SetScore, SetSimulationDistance, SetSubtitleText, SetTime, SetTitleText, SetTitlesAnimation,
        SoundEntity, Sound, StartConfiguration, StopSound, StoreCookie, SwingAnimation, SystemChat, TabList, TagQuery,
        TakeItemEntity, TeleportEntity, TestInstanceBlockStatus, TickingState, TickingStep, Transfer,
        UpdateAdvancements, UpdateAttributes, UpdateMobEffect, UpdateRecipes, UpdateTags, ProjectilePower,
        CustomReportDetails, ServerLinks, Waypoint, ClearDialog, ShowDialog,
    }
}
