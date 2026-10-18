//! Minecraft: Java Edition 26.3 protocol (protocol 777). Owner: protocol.
//!
//! See docs/INTERFACES.md for the contract this crate implements, AGENTS.md
//! for the rules and `NOTES.md` next to this crate for decisions.
//!
//! Layout:
//! - [`connection`]: [`Connection`] (resolve, handshake, login, configuration,
//!   play task with channels), [`ping`] (server list ping).
//! - [`auth`]: encryption handshake helpers and the Mojang session join.
//! - [`chat`]: signed chat (online mode): sessions, message signing,
//!   last-seen acknowledgement tracking.
//! - [`packets`]: packet ids from `packets.json` ([`PacketIds`]) and one struct
//!   per packet, grouped by state and direction.
//! - [`metadata`]: entity metadata into `haste_core::entity::MetadataEntry`.
//! - [`wire`]: data types inside packet bodies (VarInt, strings, NBT, text
//!   components, item stacks, particles), [`wire::Reader`]/[`wire::Writer`].
//! - [`nbt`]: NBT, vendored from Pumpkin (GPL-3.0).
//! - [`transport`]: length framing, zlib compression, AES-128-CFB8, vendored
//!   from Pumpkin (GPL-3.0).

pub mod auth;
pub mod chat;
pub mod connection;
pub mod metadata;
pub mod nbt;
pub mod packets;
pub mod serverpack;
pub mod transport;
pub mod wire;

/// The shared types used in packet structs, re-exported so users of this
/// crate can name them without a direct dependency.
pub use haste_core;

pub use auth::MsaTokens;
pub use chat::{
    ChatSigner, LastSeenMessagesTracker, LocalChatSession, MessageSignature, ProfileKeyPair,
};
pub use connection::{
    ping, resource_pack_status, Account, Arrived, ConnectOptions, Connection, CookieJar, Login,
    ProtocolError, ResourcePackPolicy, ServerStatus, StatsSnapshot, SyncedRegistries,
};
pub use packets::play::{ClientboundPlay, ServerboundPlay};
pub use packets::{PacketIds, ProtocolTables};

/// Minecraft 26.3's protocol version (sent in the handshake).
pub const PROTOCOL: i32 = 777;
