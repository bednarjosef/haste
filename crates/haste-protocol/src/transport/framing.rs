//! Length framing: packet size limits and the raw packet type.
//!
//! Vendored from Pumpkin `pumpkin-protocol` (GPL-3.0; see docs/THIRD_PARTY.md), moved here out of
//! `transport/mod.rs`.

use bytes::Bytes;
use thiserror::Error;

use crate::wire::varint::VarError;

/// Vanilla's maximum packet length (2 MiB), enforced on read and write.
pub const MAX_PACKET_SIZE: u64 = 2_097_152;
/// Vanilla's maximum decompressed packet data size (8 MiB).
pub const MAX_PACKET_DATA_SIZE: usize = 8_388_608;

pub struct RawPacket {
    pub id: i32,
    pub payload: Bytes,
}

#[derive(Error, Debug)]
pub enum PacketDecodeError {
    #[error("failed to decode packet ID")]
    DecodeID,
    #[error("packet exceeds maximum length")]
    TooLong,
    #[error("packet length is out of bounds")]
    OutOfBounds,
    #[error("malformed packet length VarInt: {0}")]
    MalformedLength(String),
    #[error("failed to decompress packet: {0}")]
    FailedDecompression(String),
    #[error("packet is uncompressed but greater than the threshold")]
    NotCompressed,
    #[error("the connection has closed")]
    ConnectionClosed,
    #[error("{0}")]
    Message(String),
}

#[derive(Error, Debug)]
pub enum PacketEncodeError {
    #[error("Packet exceeds maximum length: {0}")]
    TooLong(usize),
    #[error("Compression failed {0}")]
    CompressionFailed(String),
    #[error("Writing packet failed: {0}")]
    Message(String),
}

impl From<VarError> for PacketDecodeError {
    fn from(value: VarError) -> Self {
        Self::FailedDecompression(value.to_string())
    }
}
