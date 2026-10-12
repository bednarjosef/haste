//! Minecraft: Java Edition 26.3 protocol (protocol 777). Owner: WS1.
//!
//! See docs/INTERFACES.md for the contract this crate implements, AGENTS.md
//! for the rules and `NOTES.md` next to this crate for decisions.
//!
//! Layout:
//! - [`wire`]: data types inside packet bodies (VarInt, strings, NBT, text
//!   components, item stacks), [`wire::Reader`]/[`wire::Writer`].
//! - [`nbt`]: NBT, vendored from Pumpkin (GPL-3.0).
//! - [`transport`]: length framing, zlib compression, AES-128-CFB8, vendored
//!   from Pumpkin (GPL-3.0).
//! - [`packets`]: packet ids from `packets.json` and one struct per packet.

pub mod metadata;
pub mod nbt;
pub mod packets;
pub mod transport;
pub mod wire;

/// Minecraft 26.3's protocol version (sent in the handshake).
pub const PROTOCOL: i32 = 777;
