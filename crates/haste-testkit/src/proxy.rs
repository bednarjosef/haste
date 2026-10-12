//! The TCP framing half of `haste-packet-proxy`: parse protocol frames (optionally zlib
//! compressed), and re-frame for tests. The binary itself is `src/bin/haste-packet-proxy.rs`.
//!
//! Protocol framing (all states): `varint length | varint packet id | data`. Once the peer has
//! sent `set_compression`, length covers `varint data_len | zlib data` where data_len 0 marks an
//! uncompressed frame. We only ever parse; forwarding uses the original bytes untouched.

use std::io::Read;
use std::io::Write as IoWrite;
use std::net::TcpStream;

use anyhow::{bail, Context, Result};
use bytes::{Buf, Bytes, BytesMut};

use crate::packets::{varint_bytes, PacketRecordingHeader, PROTOCOL_26_3};

/// One parsed frame: the raw wire bytes (length prefix included, for forwarding) plus the
/// decoded id and decompressed body (id varint not included).
pub struct Frame {
    pub raw: Bytes,
    pub id: i32,
    pub body: Bytes,
}

/// Reads and writes protocol frames on one TCP stream.
pub struct FrameIo {
    buf: BytesMut,
    stream: TcpStream,
}

impl FrameIo {
    pub fn new(stream: TcpStream) -> FrameIo {
        FrameIo {
            buf: BytesMut::new(),
            stream,
        }
    }

    pub fn stream(&self) -> &TcpStream {
        &self.stream
    }

    /// Reads one frame. `Ok(None)` on clean EOF. `decompress` must reflect whether the peer has
    /// enabled compression at this point in the stream.
    pub fn read_frame(&mut self, decompress: bool) -> Result<Option<Frame>> {
        loop {
            if let Some(frame) = take_frame(&mut self.buf, decompress)? {
                return Ok(Some(frame));
            }
            let mut chunk = [0u8; 4096];
            let n = self
                .stream
                .read(&mut chunk)
                .context("reading from stream")?;
            if n == 0 {
                return Ok(None);
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }

    /// Writes raw framed bytes unchanged.
    pub fn write_raw(&mut self, bytes: &[u8]) -> Result<()> {
        self.stream.write_all(bytes).context("writing to stream")?;
        self.stream.flush().context("flushing stream")
    }
}

/// Attempts to parse one frame from `buf`. `Ok(None)` = need more bytes; consumed bytes are
/// removed from `buf` only when a full frame was parsed.
pub fn take_frame(buf: &mut BytesMut, decompress: bool) -> Result<Option<Frame>> {
    let probe = buf.clone().freeze();
    let Some((len, len_size)) = try_read_varint(&probe) else {
        return Ok(None);
    };
    let len = len as usize;
    if probe.len() < len_size + len {
        return Ok(None);
    }
    let total = len_size + len;
    let raw = buf.split_to(total).freeze();
    let mut frame = raw.clone();
    frame.advance(len_size); // strip the length prefix for parsing

    if !decompress {
        let id = read_varint(&mut frame)?;
        return Ok(Some(Frame {
            raw,
            id,
            body: frame,
        }));
    }

    // compressed: varint data_length | zlib data (data_length 0 = stored uncompressed)
    let (data_len, dl_size) = try_read_varint(&frame).context("truncated compression header")?;
    let mut payload = frame;
    payload.advance(dl_size);
    if data_len == 0 {
        let id = read_varint(&mut payload)?;
        return Ok(Some(Frame {
            raw,
            id,
            body: payload,
        }));
    }
    let mut decoder = flate2::read::ZlibDecoder::new(&payload[..]);
    let mut decompressed = Vec::with_capacity(data_len as usize);
    decoder
        .read_to_end(&mut decompressed)
        .context("decompressing frame")?;
    if decompressed.len() != data_len as usize {
        bail!(
            "decompressed {} bytes, expected {data_len}",
            decompressed.len()
        );
    }
    let mut body = Bytes::from(decompressed);
    let id = read_varint(&mut body)?;
    Ok(Some(Frame { raw, id, body }))
}

/// Reads a varint, advancing `buf`.
pub fn read_varint(buf: &mut Bytes) -> Result<i32> {
    let mut value: i32 = 0;
    for i in 0..5 {
        let Some(b) = buf.first().copied() else {
            bail!("truncated varint");
        };
        buf.advance(1);
        value |= ((b & 0x7f) as i32) << (i * 7);
        if b & 0x80 == 0 {
            return Ok(value);
        }
    }
    bail!("varint longer than 5 bytes")
}

/// Reads a varint without failing on truncation: `None` if more bytes are needed.
pub fn try_read_varint(buf: &Bytes) -> Option<(i32, usize)> {
    let mut value: i32 = 0;
    for i in 0..5 {
        let b = *buf.get(i)?;
        value |= ((b & 0x7f) as i32) << (i * 7);
        if b & 0x80 == 0 {
            return Some((value, i + 1));
        }
    }
    Some((value, 5)) // 5 bytes is the max; accept rather than spin
}

/// Builds a wire frame from (id, body) with the given compression threshold (-1 = off),
/// mirroring vanilla's `set_compression` rules. Used by tests to feed the parser.
pub fn frame_with_compression(id: i32, body: &[u8], threshold: i32) -> Bytes {
    let id_bytes = varint_bytes(id);
    let uncompressed_len = id_bytes.len() + body.len();
    let mut out = Vec::with_capacity(uncompressed_len + 12);
    if threshold < 0 {
        out.extend_from_slice(&varint_bytes(uncompressed_len as i32));
        out.extend_from_slice(&id_bytes);
        out.extend_from_slice(body);
    } else if uncompressed_len < threshold as usize {
        // below threshold: data_length = 0, then the raw frame
        out.extend_from_slice(&varint_bytes(uncompressed_len as i32 + 1));
        out.push(0);
        out.extend_from_slice(&id_bytes);
        out.extend_from_slice(body);
    } else {
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(&id_bytes).unwrap();
        enc.write_all(body).unwrap();
        let deflated = enc.finish().unwrap();
        let data_len_bytes = varint_bytes(uncompressed_len as i32);
        let total = data_len_bytes.len() + deflated.len();
        out.extend_from_slice(&varint_bytes(total as i32));
        out.extend_from_slice(&data_len_bytes);
        out.extend_from_slice(&deflated);
    }
    Bytes::from(out)
}

/// The recording header for a capture started now.
pub fn default_header() -> PacketRecordingHeader {
    PacketRecordingHeader {
        protocol_version: PROTOCOL_26_3,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        packet_count: 0,
    }
}

/// Writes the recording file for a finished capture.
pub fn write_recording(
    path: &std::path::Path,
    header: &PacketRecordingHeader,
    packets: &[(i32, Bytes)],
) -> Result<()> {
    use crate::packets::{PacketRecording, RecordedPacket};
    let rec = PacketRecording {
        header: *header,
        packets: packets
            .iter()
            .map(|(id, body)| RecordedPacket {
                id: *id,
                body: body.clone(),
            })
            .collect(),
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, rec.to_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_uncompressed_frame() {
        // len=4, id=0x20, body 01 02 03
        let mut raw = BytesMut::new();
        raw.extend_from_slice(&[4]);
        raw.extend_from_slice(&[0x20]);
        raw.extend_from_slice(b"\x01\x02\x03");
        let frame = take_frame(&mut raw, false).unwrap().unwrap();
        assert_eq!(frame.id, 0x20);
        assert_eq!(&frame.body[..], b"\x01\x02\x03");
        // raw keeps the length prefix for forwarding
        assert_eq!(&frame.raw[..], &[4, 0x20, 1, 2, 3]);
        assert!(raw.is_empty());
    }

    #[test]
    fn waits_for_partial_frame() {
        let mut raw = BytesMut::from(&[10, 0x20, 1, 2][..]);
        assert!(take_frame(&mut raw, false).unwrap().is_none());
        assert_eq!(raw.len(), 4); // nothing consumed
    }

    #[test]
    fn two_frames_in_one_buffer() {
        let mut raw = BytesMut::new();
        raw.extend_from_slice(&[2, 0x01, 0xaa]);
        raw.extend_from_slice(&[3, 0x02, 0xbb, 0xcc]);
        let f1 = take_frame(&mut raw, false).unwrap().unwrap();
        let f2 = take_frame(&mut raw, false).unwrap().unwrap();
        assert_eq!(f1.id, 1);
        assert_eq!(&f1.body[..], &[0xaa]);
        assert_eq!(f2.id, 2);
        assert_eq!(&f2.body[..], &[0xbb, 0xcc]);
        assert!(raw.is_empty());
    }

    #[test]
    fn compressed_round_trip() {
        for threshold in [-1i32, 0, 4, 100] {
            let frame = frame_with_compression(0x1234, b"hello compressed world", threshold);
            let mut buf = BytesMut::from(&frame[..]);
            let parsed = take_frame(&mut buf, threshold >= 0)
                .unwrap()
                .unwrap_or_else(|| panic!("threshold {threshold}: frame not parsed"));
            assert_eq!(parsed.id, 0x1234, "threshold {threshold}");
            assert_eq!(&parsed.body[..], b"hello compressed world");
            assert!(buf.is_empty());
            // raw round trips byte-identical
            assert_eq!(parsed.raw, frame);
        }
    }

    #[test]
    fn uncompressed_flag_round_trip() {
        // data_length 0 (frame below threshold) must read back without zlib
        let frame = frame_with_compression(7, b"tiny", 100);
        let mut buf = BytesMut::from(&frame[..]);
        let parsed = take_frame(&mut buf, true).unwrap().unwrap();
        assert_eq!(parsed.id, 7);
        assert_eq!(&parsed.body[..], b"tiny");
    }
}
