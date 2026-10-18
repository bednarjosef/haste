
use super::*;
use crate::wire::varint::VarInt;
use aes::cipher::{AsyncStreamCipher, KeyIvInit};
use bytes::Bytes;
use flate2::Compression;

/// Helper function to compress data using zlib.
fn compress_zlib(data: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    let mut compressed = Vec::new();
    let mut encoder = flate2::write::ZlibEncoder::new(&mut compressed, Compression::default());
    std::io::Write::write_all(&mut encoder, data)?;
    encoder.finish()?;
    Ok(compressed)
}

/// Helper function to decrypt data using AES-128 CFB-8 mode.
fn decrypt_aes128(
    encrypted_data: &mut [u8],
    key: &[u8; 16],
    iv: &[u8; 16],
) -> Result<(), Box<dyn std::error::Error>> {
    let decryptor =
        cfb8::Decryptor::<aes::Aes128>::new_from_slices(key, iv).map_err(|_| "Invalid key/iv")?;
    decryptor.decrypt(encrypted_data);
    Ok(())
}

/// Helper function to build a frame with optional compression and encryption.
fn build_packet(
    packet_id: i32,
    payload: &[u8],
    compress: bool,
    key: Option<&[u8; 16]>,
    iv: Option<&[u8; 16]>,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut data_to_compress = Vec::new();
    VarInt(packet_id).encode(&mut data_to_compress)?;
    data_to_compress.extend_from_slice(payload);

    let mut buffer = Vec::new();
    if compress {
        let compressed_payload = compress_zlib(&data_to_compress)?;
        let data_len = data_to_compress.len() as i32;
        VarInt(data_len).encode(&mut buffer)?;
        buffer.extend_from_slice(&compressed_payload);
    } else {
        VarInt(packet_id).encode(&mut buffer)?;
        buffer.extend_from_slice(payload);
    }

    let packet_len = buffer.len() as i32;
    let mut packet = Vec::new();
    VarInt(packet_len).encode(&mut packet)?;
    packet.extend_from_slice(&buffer);

    if let (Some(k), Some(v)) = (key, iv) {
        let encryptor =
            cfb8::Encryptor::<aes::Aes128>::new_from_slices(k, v).map_err(|_| "Invalid key/iv")?;
        encryptor.encrypt(&mut packet);
    }
    Ok(packet)
}

#[tokio::test]
async fn decode_without_compression_and_encryption() -> Result<(), Box<dyn std::error::Error>> {
    let packet = build_packet(1, b"Hello", false, None, None)?;
    let mut decoder = TcpNetworkDecoder::new(packet.as_slice());
    let raw_packet = decoder.get_raw_packet().await?;
    assert_eq!(raw_packet.id, 1);
    assert_eq!(raw_packet.payload.as_ref(), b"Hello");
    Ok(())
}

#[tokio::test]
async fn decode_with_compression() -> Result<(), Box<dyn std::error::Error>> {
    let packet = build_packet(2, b"Hello, compressed world!", true, None, None)?;
    let mut decoder = TcpNetworkDecoder::new(packet.as_slice());
    decoder.set_compression(1000);
    let raw_packet = decoder.get_raw_packet().await?;
    assert_eq!(raw_packet.id, 2);
    assert_eq!(raw_packet.payload.as_ref(), b"Hello, compressed world!");
    Ok(())
}

#[tokio::test]
async fn decode_with_encryption() -> Result<(), Box<dyn std::error::Error>> {
    let key = [0x00u8; 16];
    let packet = build_packet(3, b"Hello, encrypted world!", false, Some(&key), Some(&key))?;
    let mut decoder = TcpNetworkDecoder::new(packet.as_slice());
    decoder.set_encryption(&key)?;
    let raw_packet = decoder.get_raw_packet().await?;
    assert_eq!(raw_packet.id, 3);
    assert_eq!(raw_packet.payload.as_ref(), b"Hello, encrypted world!");
    Ok(())
}

#[tokio::test]
async fn decode_with_compression_and_encryption() -> Result<(), Box<dyn std::error::Error>> {
    let key = [0x01u8; 16];
    let packet = build_packet(
        4,
        b"Hello, compressed and encrypted world!",
        true,
        Some(&key),
        Some(&key),
    )?;
    let mut decoder = TcpNetworkDecoder::new(packet.as_slice());
    decoder.set_compression(1000);
    decoder.set_encryption(&key)?;
    let raw_packet = decoder.get_raw_packet().await?;
    assert_eq!(raw_packet.id, 4);
    assert_eq!(
        raw_packet.payload.as_ref(),
        b"Hello, compressed and encrypted world!"
    );
    Ok(())
}

#[tokio::test]
async fn decode_with_invalid_compressed_data() -> Result<(), Box<dyn std::error::Error>> {
    let mut buffer = Vec::new();
    VarInt(10).encode(&mut buffer)?;
    buffer.extend_from_slice(&[0xFF, 0xFF, 0xFF]); // Invalid zlib data

    let mut packet_buffer = Vec::new();
    VarInt(buffer.len() as i32).encode(&mut packet_buffer)?;
    packet_buffer.extend_from_slice(&buffer);

    let mut decoder = TcpNetworkDecoder::new(&packet_buffer[..]);
    decoder.set_compression(1000);
    assert!(decoder.get_raw_packet().await.is_err());
    Ok(())
}

#[tokio::test]
async fn decode_multiple_packets_reuses_scratch() -> Result<(), Box<dyn std::error::Error>> {
    let packet1 = build_packet(1, b"Hello", false, None, None)?;
    let packet2 = build_packet(2, b"World", false, None, None)?;

    let mut stream = Vec::new();
    stream.extend_from_slice(&packet1);
    stream.extend_from_slice(&packet2);

    let mut decoder = TcpNetworkDecoder::new(stream.as_slice());

    let p1 = decoder.get_raw_packet().await?;
    assert_eq!(p1.id, 1);
    assert_eq!(p1.payload.as_ref(), b"Hello");
    drop(p1);

    let cap_after_p1 = decoder.payload_scratch.capacity();
    assert!(cap_after_p1 > 0);

    let p2 = decoder.get_raw_packet().await?;
    assert_eq!(p2.id, 2);
    assert_eq!(p2.payload.as_ref(), b"World");
    drop(p2);

    assert_eq!(decoder.payload_scratch.capacity(), cap_after_p1);
    Ok(())
}

#[tokio::test]
async fn encoder_decoder_round_trip_all_modes() -> Result<(), Box<dyn std::error::Error>> {
    for (compression, key) in [
        (None, None),
        (Some(1000usize), None),
        (None, Some([0x42u8; 16])),
        (Some(1000usize), Some([0x42u8; 16])),
    ] {
        let mut body = Vec::new();
        VarInt(0x2A).encode(&mut body)?;
        body.extend_from_slice(b"round trip body");

        let mut wire = Vec::new();
        {
            let mut encoder = TcpNetworkEncoder::new(&mut wire);
            if let Some(threshold) = compression {
                encoder.set_compression((threshold, 6));
            }
            if let Some(k) = key {
                encoder.set_encryption(&k)?;
            }
            encoder.write_packet(body.clone().into()).await?;
            encoder.flush().await?;
        }

        let mut decoder = TcpNetworkDecoder::new(&wire[..]);
        if let Some(threshold) = compression {
            decoder.set_compression(threshold);
        }
        if let Some(k) = key {
            decoder.set_encryption(&k)?;
        }
        let raw = decoder.get_raw_packet().await?;
        assert_eq!(raw.id, 0x2A);
        assert_eq!(raw.payload.as_ref(), b"round trip body");
    }
    Ok(())
}

#[tokio::test]
async fn small_packet_not_compressed_below_threshold() {
    // With threshold 100 and a small body, the frame carries Data Length 0.
    let mut body = Vec::new();
    VarInt(7).encode(&mut body).unwrap();

    let mut wire = Vec::new();
    {
        let mut encoder = TcpNetworkEncoder::new(&mut wire);
        encoder.set_compression((100, 6));
        encoder.write_packet(body.into()).await.unwrap();
    }

    let mut decoder = TcpNetworkDecoder::new(&wire[..]);
    decoder.set_compression(100);
    let raw = decoder.get_raw_packet().await.unwrap();
    assert_eq!(raw.id, 7);
    assert!(raw.payload.is_empty());
}

/// A reader that hands out a fixed byte stream in randomly sized pieces,
/// like TCP would. Deterministic (LCG) so failures reproduce.
struct ChunkedReader {
    data: Vec<u8>,
    pos: usize,
    rng: u64,
    max_chunk: usize,
}

impl ChunkedReader {
    fn new(data: Vec<u8>, seed: u64, max_chunk: usize) -> Self {
        Self {
            data,
            pos: 0,
            rng: seed | 1,
            max_chunk,
        }
    }

    fn next_chunk(&mut self) -> usize {
        // xorshift64*, mapped into 1..=max_chunk (also likes tiny reads).
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        let spread = x.wrapping_mul(0x2545_F491_4F6C_DD1D) % self.max_chunk as u64;
        let n = spread as usize + 1;
        // Every eighth read is tiny, to split varints across reads.
        if self.rng.is_multiple_of(8) {
            1
        } else {
            n
        }
    }
}

impl tokio::io::AsyncRead for ChunkedReader {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if self.pos >= self.data.len() {
            return std::task::Poll::Ready(Ok(()));
        }
        let n = self.next_chunk().min(self.data.len() - self.pos);
        let n = n.min(buf.remaining());
        let start = self.pos;
        self.pos += n;
        buf.put_slice(&self.data[start..start + n]);
        std::task::Poll::Ready(Ok(()))
    }
}

/// Builds one frame (optionally compressed when the body reaches the
/// threshold) exactly as a vanilla server does.
fn frame(body: &[u8], threshold: usize) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut inner = Vec::new();
    if body.len() >= threshold {
        let compressed = compress_zlib(body)?;
        VarInt(body.len() as i32).encode(&mut inner)?;
        inner.extend_from_slice(&compressed);
    } else {
        VarInt(0).encode(&mut inner)?;
        inner.extend_from_slice(body);
    }
    let mut out = Vec::new();
    VarInt(inner.len() as i32).encode(&mut out)?;
    out.extend_from_slice(&inner);
    Ok(out)
}

/// One body as id + payload (what the decoder must return).
fn body(packet_id: i32, payload: &[u8]) -> Bytes {
    let mut v = Vec::new();
    VarInt(packet_id).encode(&mut v).unwrap();
    v.extend_from_slice(payload);
    v.into()
}

/// A reader that hands out a fixed byte stream in blocks, but forces one
/// read boundary at `split_at`: the read that would cross it stops there
/// and the rest follows in the next read. This is how TCP splits a frame.
struct SplitReader {
    data: Vec<u8>,
    pos: usize,
    split_at: usize,
    block: usize,
}

impl SplitReader {
    fn new(data: Vec<u8>, split_at: usize, block: usize) -> Self {
        Self {
            data,
            pos: 0,
            split_at,
            block,
        }
    }
}

impl tokio::io::AsyncRead for SplitReader {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if self.pos >= self.data.len() {
            return std::task::Poll::Ready(Ok(()));
        }
        let mut n = self.block.min(self.data.len() - self.pos);
        if self.pos < self.split_at {
            n = n.min(self.split_at - self.pos);
        }
        let n = n.min(buf.remaining());
        let start = self.pos;
        self.pos += n;
        buf.put_slice(&self.data[start..start + n]);
        std::task::Poll::Ready(Ok(()))
    }
}

/// A compressed frame ends with the zlib checksum trailer. The decoder
/// stops reading once it has the packet's bytes, so a read that ends
/// between the deflate data and the trailer must not leave the trailer
/// for the next frame's length VarInt to be decoded from.
#[tokio::test]
async fn compressed_frame_trailer_is_consumed_before_the_next_frame(
) -> Result<(), Box<dyn std::error::Error>> {
    const THRESHOLD: usize = 256;
    let payload: Vec<u8> = (0..2_000u32).map(|i| (i % 251) as u8).collect();
    let mut wire = frame(&body(9, &payload), THRESHOLD)?;
    let trailer_start = wire.len() - 4;
    // A small frame after it: if the trailer is left unread, its length
    // VarInt is decoded from trailer bytes.
    wire.extend_from_slice(&frame(&body(10, b"tail"), THRESHOLD)?);

    let mut decoder = TcpNetworkDecoder::new(SplitReader::new(wire, trailer_start, 8192));
    decoder.set_compression(THRESHOLD);
    let p = decoder.get_raw_packet().await?;
    assert_eq!(p.id, 9);
    assert_eq!(&p.payload[..], &payload[..]);
    let p = decoder.get_raw_packet().await?;
    assert_eq!(p.id, 10);
    assert_eq!(&p.payload[..], b"tail");
    Ok(())
}

/// Every frame in a burst of mixed sizes must come out identical, whatever
/// sizes the stream is handed to the decoder in.
#[tokio::test]
async fn decodes_burst_of_mixed_frames_in_random_read_sizes(
) -> Result<(), Box<dyn std::error::Error>> {
    const THRESHOLD: usize = 256;
    // Sizes around the compression threshold and around read-buffer
    // boundaries, like chunk sections and small packets during a join, up
    // to a whole chunk-burst frame at view distance 32 (a
    // ClientboundLevelChunkWithLightPacket with light data reaches ~2 MiB,
    // the protocol's maximum frame length).
    let sizes = [
        0usize, 1, 5, 100, 255, 256, 257, 300, 1_000, 4_000, 8_191, 8_192, 8_193, 16_384, 40_000,
        150_000, 300_000,
        // A frame near the 2 MiB protocol maximum, split across many
        // reads: the chunk burst at render distance 32 sends these back to
        // back, and one unread trailer byte used to desync the next
        // frame's length VarInt ("failed to decode packet ID").
        2_000_000,
    ];
    let mut expected: Vec<(i32, Bytes)> = Vec::new();
    let mut wire = Vec::new();
    let mut id = 1;
    for pass in 0..3 {
        for &size in &sizes {
            let mut payload = Vec::with_capacity(size);
            for i in 0..size {
                // Both compressible and incompressible stretches.
                payload.push(if pass == 0 {
                    b'a'
                } else {
                    (i.wrapping_mul(2_654_435_761) >> 13) as u8
                });
            }
            let p = if id < 0 { id - 1 } else { id + 1 };
            id = p;
            let b = body(p, &payload);
            wire.extend_from_slice(&frame(&b, THRESHOLD)?);
            expected.push((p, payload.clone().into()));
        }
    }

    for seed in [1u64, 7, 42, 1234, 999_999] {
        for max_chunk in [1usize, 2, 3, 5, 64, 8_192, 64 * 1_024] {
            let reader = ChunkedReader::new(wire.clone(), seed, max_chunk);
            let mut decoder = TcpNetworkDecoder::new(reader);
            decoder.set_compression(THRESHOLD);
            for (idx, (want_id, want_body)) in expected.iter().enumerate() {
                let raw = decoder.get_raw_packet().await.unwrap_or_else(|e| {
                    panic!("seed {seed} chunk {max_chunk} frame {idx} (id {want_id}): {e}")
                });
                assert_eq!(raw.id, *want_id, "seed {seed} chunk {max_chunk}");
                assert_eq!(&raw.payload, want_body, "seed {seed} chunk {max_chunk}");
            }
            // Nothing left: the next read must hit clean EOF.
            let end = decoder.get_raw_packet().await;
            assert!(
                end.is_err(),
                "seed {seed} chunk {max_chunk}: stream not exhausted"
            );
        }
    }
    Ok(())
}

/// A framing error names the previous packet (id, frame length, payload
/// length): a desync shows up in the frame AFTER the one that caused it,
/// so that line is what pinpoints the culprit packet.
#[tokio::test]
async fn framing_error_names_the_previous_packet() -> Result<(), Box<dyn std::error::Error>> {
    const THRESHOLD: usize = 256;
    let mut wire = frame(&body(9, b"fine"), THRESHOLD)?;
    // Garbage that decodes as no valid VarInt id (EOF mid-varint).
    wire.extend_from_slice(&[0x80]);

    let mut decoder = TcpNetworkDecoder::new(SplitReader::new(wire, usize::MAX, 8192));
    decoder.set_compression(THRESHOLD);
    let p = decoder.get_raw_packet().await?;
    assert_eq!(p.id, 9);
    let Err(err) = decoder.get_raw_packet().await else {
        panic!("the truncated frame must fail");
    };
    let msg = err.to_string();
    assert!(msg.contains("id 9"), "message lacks the packet id: {msg}");
    assert!(
        msg.contains("frame"),
        "message lacks the frame length: {msg}"
    );
    assert!(
        msg.contains("payload 4 bytes"),
        "message lacks the payload length: {msg}"
    );
    Ok(())
}

#[tokio::test]
async fn stream_encryptor_matches_reference_cfb8_with_random_key() {
    use rsa::rand_core::{OsRng, RngCore};
    let mut key = [0u8; 16];
    OsRng.fill_bytes(&mut key);
    let mut plain = vec![0u8; 5000];
    OsRng.fill_bytes(&mut plain);
    let mut wire = Vec::new();
    {
        let cipher = Aes128Cfb8Enc::new_from_slices(&key, &key).unwrap();
        let mut enc = StreamEncryptor::new(cipher, &mut wire);
        // Several writes: the cipher state must carry across them.
        for chunk in plain.chunks(777) {
            tokio::io::AsyncWriteExt::write_all(&mut enc, chunk)
                .await
                .unwrap();
        }
        tokio::io::AsyncWriteExt::flush(&mut enc).await.unwrap();
    }
    assert_ne!(wire, plain);
    let mut check = wire.clone();
    decrypt_aes128(&mut check, &key, &key).unwrap();
    assert_eq!(check, plain);
    // And our decrypting reader agrees.
    let cipher = Aes128Cfb8Dec::new_from_slices(&key, &key).unwrap();
    let mut dec = StreamDecryptor::new(cipher, &wire[..]);
    let mut out = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut dec, &mut out)
        .await
        .unwrap();
    assert_eq!(out, plain);
}

#[tokio::test]
async fn large_packet_compressed_round_trip_and_threshold_boundary() {
    // 256 is the vanilla server's default threshold: a body of exactly
    // the threshold is compressed, one byte less is not.
    for len in [255usize, 256, 100_000] {
        let mut body = Vec::new();
        VarInt(0x10).encode(&mut body).unwrap();
        body.extend((0..len - 1).map(|i| (i % 7) as u8));
        let mut wire = Vec::new();
        {
            let mut enc = TcpNetworkEncoder::new(&mut wire);
            enc.set_compression((256, 6));
            enc.write_packet(body.clone().into()).await.unwrap();
            enc.flush().await.unwrap();
        }
        // Data Length field: 0 when not compressed.
        let mut r = &wire[..];
        VarInt::decode(&mut r).unwrap();
        let data_len = VarInt::decode(&mut r).unwrap().0;
        assert_eq!(data_len == 0, len < 256, "len {len}");
        let mut dec = TcpNetworkDecoder::new(&wire[..]);
        dec.set_compression(256);
        let p = dec.get_raw_packet().await.unwrap();
        assert_eq!(p.id, 0x10);
        assert_eq!(&p.payload[..], &body[1..]);
    }
}
