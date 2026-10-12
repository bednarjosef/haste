//! Loader for recorded clientbound play packet dumps (`packets/<name>.bin`, see FORMATS.md):
//! length-prefixed, decompressed frames with a small header (magic, protocol version,
//! timestamp, packet count).

use std::path::Path;

use anyhow::{bail, Context, Result};
use bytes::{Buf, Bytes};

/// The recording header.
#[derive(Clone, Copy, Debug)]
pub struct PacketRecordingHeader {
    pub protocol_version: u32,
    /// Unix millis when recording started.
    pub timestamp: u64,
    pub packet_count: u32,
}

/// One recorded packet: id (as on the wire, first varint of the frame) and body after it.
#[derive(Clone, Debug)]
pub struct RecordedPacket {
    pub id: i32,
    pub body: Bytes,
}

/// A parsed recording.
#[derive(Clone, Debug)]
pub struct PacketRecording {
    pub header: PacketRecordingHeader,
    pub packets: Vec<RecordedPacket>,
}

pub const MAGIC: &[u8; 5] = b"HSTK\0";
pub const PROTOCOL_26_3: u32 = 777;

impl PacketRecording {
    pub fn load(path: &Path) -> Result<PacketRecording> {
        let raw = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(Bytes::from(raw)).with_context(|| format!("parsing {}", path.display()))
    }

    /// Parses the format from FORMATS.md:
    /// `HSTK\0` | u32 protocol | u64 millis | u32 count | ( u32 len | bytes[len] )*.
    pub fn parse(mut buf: Bytes) -> Result<PacketRecording> {
        if buf.len() < 5 + 4 + 8 + 4 {
            bail!("truncated header ({} bytes)", buf.len());
        }
        if &buf[..5] != MAGIC {
            bail!("bad magic {:02x?} (expected {MAGIC:02x?})", &buf[..5]);
        }
        buf.advance(5);
        let protocol_version = buf.get_u32();
        let timestamp = buf.get_u64();
        let packet_count = buf.get_u32();

        let mut packets = Vec::with_capacity(packet_count as usize);
        for _ in 0..packet_count {
            if buf.remaining() < 4 {
                bail!("truncated length for packet {}", packets.len());
            }
            let len = buf.get_u32() as usize;
            if buf.remaining() < len {
                bail!(
                    "truncated body for packet {} (need {len}, have {})",
                    packets.len(),
                    buf.remaining()
                );
            }
            let mut frame = buf.split_to(len);
            let id = read_varint(&mut frame)?;
            packets.push(RecordedPacket { id, body: frame });
        }
        if buf.has_remaining() {
            bail!(
                "{} trailing bytes after {} packets",
                buf.remaining(),
                packet_count
            );
        }
        Ok(PacketRecording {
            header: PacketRecordingHeader {
                protocol_version,
                timestamp,
                packet_count,
            },
            packets,
        })
    }

    /// Serialises back to the on-disk format (round trip used by tests and the proxy).
    pub fn to_bytes(&self) -> Bytes {
        let mut out =
            Vec::with_capacity(21 + self.packets.iter().map(|p| p.body.len() + 8).sum::<usize>());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.header.protocol_version.to_be_bytes());
        out.extend_from_slice(&self.header.timestamp.to_be_bytes());
        out.extend_from_slice(&(self.packets.len() as u32).to_be_bytes());
        for p in &self.packets {
            let id_bytes = varint_bytes(p.id);
            let len = id_bytes.len() + p.body.len();
            out.extend_from_slice(&(len as u32).to_be_bytes());
            out.extend_from_slice(&id_bytes);
            out.extend_from_slice(&p.body);
        }
        Bytes::from(out)
    }
}

/// Protocol varint (LEB128), as on the wire.
pub fn read_varint(buf: &mut Bytes) -> Result<i32> {
    let mut value: i32 = 0;
    for i in 0..5 {
        if !buf.has_remaining() {
            bail!("truncated varint");
        }
        let b = buf.get_u8();
        value |= ((b & 0x7f) as i32) << (i * 7);
        if b & 0x80 == 0 {
            return Ok(value);
        }
    }
    bail!("varint longer than 5 bytes")
}

pub fn varint_bytes(v: i32) -> Vec<u8> {
    // Encode as the unsigned LE bytes the wire format uses: negative values sign-extend to
    // the full 5 bytes (e.g. -1 -> ff ff ff ff 0f), so cast to u32 first.
    let mut v = v as u32;
    let mut out = Vec::with_capacity(5);
    loop {
        let mut b = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            b |= 0x80;
        }
        out.push(b);
        if v == 0 {
            return out;
        }
    }
}

/// Frame length on the wire for a body of `body_len` bytes (id varint included in the body).
pub fn frame_len(body_len: usize) -> usize {
    body_len
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recording() -> PacketRecording {
        PacketRecording {
            header: PacketRecordingHeader {
                protocol_version: PROTOCOL_26_3,
                timestamp: 1_700_000_000_000,
                packet_count: 0,
            },
            packets: vec![
                RecordedPacket {
                    id: 0x20,
                    body: Bytes::from_static(b"\x01\x02\x03"),
                },
                RecordedPacket {
                    id: 0x1234,
                    body: Bytes::from_static(b"hello packets"),
                },
                RecordedPacket {
                    id: -1,
                    body: Bytes::new(),
                },
            ],
        }
    }

    #[test]
    fn varint_round_trip() {
        for v in [
            0i32,
            1,
            127,
            128,
            255,
            256,
            1000,
            0x1234,
            i32::MAX,
            -1,
            i32::MIN,
        ] {
            let bytes = varint_bytes(v);
            let parsed = read_varint(&mut Bytes::from(bytes.clone())).unwrap();
            assert_eq!(parsed, v, "value {v}");
        }
    }

    #[test]
    fn recording_round_trip() {
        let r = recording();
        let bytes = r.to_bytes();
        let back = PacketRecording::parse(bytes.clone()).unwrap();
        assert_eq!(back.header.protocol_version, PROTOCOL_26_3);
        assert_eq!(back.header.timestamp, 1_700_000_000_000);
        assert_eq!(back.packets.len(), 3);
        assert_eq!(back.packets[0].id, 0x20);
        assert_eq!(&back.packets[0].body[..], b"\x01\x02\x03");
        assert_eq!(back.packets[1].id, 0x1234);
        assert_eq!(&back.packets[1].body[..], b"hello packets");
        assert_eq!(back.packets[2].id, -1);
        assert!(back.packets[2].body.is_empty());
        // 21-byte header + per packet: 4B len + id varint + body
        // (0x20: 1B id, 3B body; 0x1234: 2B id, 13B body; -1: 5B id, 0B body)
        assert_eq!(bytes.len(), 21 + (4 + 1 + 3) + (4 + 2 + 13) + (4 + 5 + 0));
    }

    #[test]
    fn rejects_bad_magic_and_truncation() {
        let r = recording();
        let mut bytes = r.to_bytes().to_vec();
        bytes[0] = b'X';
        assert!(PacketRecording::parse(Bytes::from(bytes)).is_err());
        let bytes = r.to_bytes();
        assert!(PacketRecording::parse(bytes.clone().slice(..bytes.len() - 3)).is_err());
        let mut extra = bytes.to_vec();
        extra.push(0);
        assert!(PacketRecording::parse(Bytes::from(extra)).is_err());
    }
}
