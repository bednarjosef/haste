//! Serverbound play packets.
//!
//! Layouts from the minecraft.wiki "Java Edition protocol/Packets" page for
//! 26.3. Note: 26.3 has no serverbound `swing` packet; arm swings are
//! `punch` (no fields) and attacks are `attack`.
//!
//! The packets are grouped into one module per vanilla concept: [`movement`]
//! (player and vehicle movement), [`interaction`] (attacking, using and
//! picking), [`inventory`] (containers, the recipe book and creative slots),
//! [`chat`] (chat and commands) and [`misc`] (settings and debug packets).
//! Every type is re-exported here, so `serverbound::Foo` paths are unchanged.

use bytes::Bytes;
use haste_core::{BlockPos, DVec3, Identifier};
use uuid::Uuid;

use super::clientbound::{ChatSession, GameRuleValue};
pub use crate::packets::configuration::serverbound::{
    ClientInformation, CookieResponse, CustomClickAction, CustomPayload, KeepAlive, Pong,
    ResourcePack,
};
use crate::packets::{packet, packet_enum, Packet};
use crate::wire::{DecodeResult, List, LpVec3, Opt, Reader, Rest, VarInt, VarLong, Wire, Writer};

mod chat;
mod interaction;
mod inventory;
mod misc;
mod movement;

pub use chat::*;
pub use interaction::*;
pub use inventory::*;
pub use misc::*;
pub use movement::*;

#[cfg(test)]
mod tests;

packet_enum! {
    /// Serverbound play packets, in `packets.json` id order.
    ServerboundPlay {
        AcceptTeleportation, Attack, BlockEntityTagQuery, BundleItemSelected, ChangeDifficulty, ChangeGameMode,
        ChatAck, ChatCommand, ChatCommandSigned, Chat, ChatSessionUpdate, ChunkBatchReceived, ClientCommand,
        ClientTickEnd, ClientInformation, CommandSuggestion, ConfigurationAcknowledged, ContainerButtonClick,
        ContainerClick, ContainerClose, ContainerSlotStateChanged, CookieResponse, CustomPayload,
        DebugSubscriptionRequest, EditBook, EntityTagQuery, Interact, JigsawGenerate, KeepAlive, LockDifficulty,
        MovePlayerPos, MovePlayerPosRot, MovePlayerRot, MovePlayerStatusOnly, MoveVehicle, PaddleBoat,
        PickItemFromBlock, PickItemFromEntity, PingRequest, PlaceRecipe, PlayerAbilities, PlayerAction, PlayerCommand,
        PlayerInput, PlayerLoaded, Pong, Punch, RecipeBookChangeSettings, RecipeBookSeenRecipe, RenameItem,
        ResourcePack, SeenAdvancements, SelectTrade, SetBeacon, SetCarriedItem, SetCommandBlock, SetCommandMinecart,
        SetCreativeModeSlot, SetGameRule, SetJigsawBlock, SetStructureBlock, SetTestBlock, SignUpdate,
        SpectatorAction, TeleportToEntity, TestInstanceBlockAction, UseItemOn, UseItem, CustomClickAction,
    }
}
