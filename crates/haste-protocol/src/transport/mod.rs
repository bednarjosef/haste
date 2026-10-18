//! Length framing, zlib compression and AES-128-CFB8 encryption for the
//! Minecraft Java protocol, over tokio TCP streams.
//!
//! Vendored from Pumpkin `pumpkin-protocol` (GPL-3.0; see THIRD_PARTY.md):
//! `src/java/packet_decoder.rs`, `src/java/packet_encoder.rs` and the
//! `StreamDecryptor`/`StreamEncryptor` from `src/lib.rs`. Adapted for Haste:
//! single version (26.3), no `ClientPacket`/`ServerPacket` traits (raw id +
//! bytes instead), encoder compression level fixed at 6.
//!
//! Wire format:
//!
//! **Uncompressed:** `Packet Length (VarInt) | Packet ID (VarInt) | Data`
//!
//! **Compressed:** `Packet Length (VarInt) | Data Length (VarInt) | Packet ID (VarInt) | Data`

mod compression;
mod encryption;
mod framing;
mod io;

#[cfg(test)]
mod tests;

pub use compression::{CompressionLevel, CompressionThreshold, DecompressionReader};
pub use encryption::{DecryptionReader, EncryptionWriter, StreamDecryptor, StreamEncryptor};
pub use framing::{
    PacketDecodeError, PacketEncodeError, RawPacket, MAX_PACKET_DATA_SIZE, MAX_PACKET_SIZE,
};
pub use io::{TcpNetworkDecoder, TcpNetworkEncoder};

pub(crate) use encryption::{Aes128Cfb8Dec, Aes128Cfb8Enc};
