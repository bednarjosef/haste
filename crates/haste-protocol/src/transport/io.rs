//! The packet reader and writer over tokio streams (decode/encode with compression and encryption).
//!
//! Vendored from Pumpkin `pumpkin-protocol` (GPL-3.0; see docs/THIRD_PARTY.md), moved here out of
//! `transport/mod.rs`.

use aes::cipher::KeyIvInit;
use bytes::{Bytes, BytesMut};
use flate2::{Compress, Compression, FlushCompress, Status};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use crate::wire::varint::VarInt;

use super::{
    Aes128Cfb8Dec, Aes128Cfb8Enc, CompressionLevel, CompressionThreshold, DecompressionReader,
    DecryptionReader, EncryptionWriter, PacketDecodeError, PacketEncodeError, RawPacket,
    MAX_PACKET_DATA_SIZE, MAX_PACKET_SIZE,
};

/// Decoder: Server -> Client. Supports `ZLib` decoding/decompression and AES-128
/// encryption.
pub struct TcpNetworkDecoder<R: AsyncRead + Unpin> {
    reader: Option<DecryptionReader<R>>,
    compression: Option<CompressionThreshold>,
    pub(crate) payload_scratch: BytesMut,
    /// The last packet handed out: `(id, frame length, payload length)`. A
    /// desync shows up in the frame AFTER the one that caused it, so framing
    /// errors name this packet; that usually identifies the culprit.
    last_packet: Option<(i32, u64, usize)>,
}

impl<R: AsyncRead + Unpin> TcpNetworkDecoder<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader: Some(DecryptionReader::None(reader)),
            compression: None,
            payload_scratch: BytesMut::new(),
            last_packet: None,
        }
    }

    pub const fn set_compression(&mut self, threshold: CompressionThreshold) {
        self.compression = Some(threshold);
    }

    /// NOTE: Encryption can only be set; a minecraft stream cannot go back to being unencrypted
    pub fn set_encryption(&mut self, key: &[u8; 16]) -> Result<(), PacketDecodeError> {
        if matches!(self.reader, Some(DecryptionReader::Decrypt(_))) {
            return Err(PacketDecodeError::Message(
                "Encryption already enabled".into(),
            ));
        }
        let cipher = Aes128Cfb8Dec::new_from_slices(key, key)
            .map_err(|_| PacketDecodeError::Message("Invalid key".into()))?;

        if let Some(reader) = self.reader.take() {
            self.reader = Some(reader.upgrade(cipher));
        }
        Ok(())
    }

    /// Reads and discards whatever the current frame still holds: the reads in
    /// [`Self::get_raw_packet`] stop as soon as the packet's declared contents
    /// are there, so a compressed frame can leave bytes unread (a zlib decoder
    /// produces its last output byte before reading the checksum trailer that
    /// follows it). Left in the socket, they would be decoded as the next
    /// frame's length VarInt. Vanilla's `Varint21FrameDecoder` hands the
    /// compression step exactly the frame's bytes, so it never has this gap.
    async fn consume_rest_of_frame<S: AsyncRead + Unpin>(
        reader: &mut DecompressionReader<S>,
    ) -> Result<(), PacketDecodeError> {
        use tokio::io::AsyncReadExt;
        let mut discard = [0u8; 1024];
        loop {
            let n = reader
                .read(&mut discard)
                .await
                .map_err(|err| PacketDecodeError::FailedDecompression(err.to_string()))?;
            if n == 0 {
                break;
            }
        }
        // The zlib decoder stops at the end of its stream, so bytes the frame
        // holds after it have not been read from the socket either.
        if let DecompressionReader::Decompress(zlib) = reader {
            loop {
                let n = zlib
                    .get_mut()
                    .read(&mut discard)
                    .await
                    .map_err(|err| PacketDecodeError::FailedDecompression(err.to_string()))?;
                if n == 0 {
                    break;
                }
            }
        }
        Ok(())
    }

    pub async fn get_raw_packet(&mut self) -> Result<RawPacket, PacketDecodeError> {
        // Captured before the reader borrow: framing errors below name it.
        let last_packet = self.last_packet;
        let framing_error = |err: PacketDecodeError| {
            match last_packet {
                Some((id, frame_len, payload_len)) => PacketDecodeError::Message(format!(
                    "{err}; previous packet: id {id}, frame {frame_len} bytes, payload {payload_len} bytes"
                )),
                None => err,
            }
        };
        let reader = self
            .reader
            .as_mut()
            .ok_or_else(|| PacketDecodeError::Message("Reader missing".into()))?;

        let packet_len = VarInt::decode_async(reader)
            .await
            .map_err(|err| framing_error(PacketDecodeError::MalformedLength(err.to_string())))?;

        let packet_len = packet_len.0 as u64;

        if !(0..=MAX_PACKET_SIZE).contains(&packet_len) {
            Err(framing_error(PacketDecodeError::OutOfBounds))?;
        }

        let mut bounded_reader = reader.take(packet_len);
        let mut expected_packet_data_len = packet_len as usize;
        let mut expected_uncompressed_packet_data_len = None;

        let mut reader = if let Some(threshold) = self.compression {
            let (decompressed_length, decompressed_length_size) =
                VarInt::decode_async_sized(&mut bounded_reader).await?;
            let raw_packet_length = packet_len - decompressed_length_size as u64;
            let decompressed_length = decompressed_length.0 as usize;

            if !(0..=MAX_PACKET_DATA_SIZE).contains(&decompressed_length) {
                Err(framing_error(PacketDecodeError::TooLong))?;
            }

            if decompressed_length > 0 {
                expected_packet_data_len = decompressed_length;
                expected_uncompressed_packet_data_len = Some(decompressed_length);
                DecompressionReader::Decompress(
                    async_compression::tokio::bufread::ZlibDecoder::new(BufReader::new(
                        bounded_reader,
                    )),
                )
            } else {
                // Validate that we are not less than the compression threshold
                if raw_packet_length > threshold as u64 {
                    Err(framing_error(PacketDecodeError::NotCompressed))?;
                }

                expected_packet_data_len = raw_packet_length as usize;
                DecompressionReader::None(bounded_reader)
            }
        } else {
            DecompressionReader::None(bounded_reader)
        };

        let (packet_id, packet_id_len) = VarInt::decode_async_sized(&mut reader)
            .await
            .map_err(|_| framing_error(PacketDecodeError::DecodeID))?;

        let payload_len_hint = expected_packet_data_len.saturating_sub(packet_id_len);
        self.payload_scratch.clear();
        self.payload_scratch.reserve(payload_len_hint);

        let mut total_read = 0;
        while total_read < payload_len_hint {
            let bytes_read = reader
                .read_buf(&mut self.payload_scratch)
                .await
                .map_err(|err| PacketDecodeError::FailedDecompression(err.to_string()))?;
            if bytes_read == 0 {
                break;
            }
            total_read += bytes_read;
        }

        // A frame must be consumed whole: the reads above stop as soon as the
        // declared contents are there, and a zlib decoder hands out its last
        // output byte before it has read the frame's remaining bytes (the zlib
        // checksum trailer, and anything a server appends after it). Those
        // bytes would then be read as the next frame's length VarInt and the
        // stream would desynchronise ("failed to decode packet ID", or a
        // bogus VarInt while reading the next frame's data length). Vanilla's
        // frame decoder hands the compression step exactly the frame's bytes,
        // so the tail is always consumed there too.
        Self::consume_rest_of_frame(&mut reader).await?;

        if let Some(expected_uncompressed_packet_data_len) = expected_uncompressed_packet_data_len {
            let decoded_packet_data_len = packet_id_len + self.payload_scratch.len();
            if decoded_packet_data_len != expected_uncompressed_packet_data_len {
                return Err(PacketDecodeError::FailedDecompression(format!(
                    "Declared decompressed length {expected_uncompressed_packet_data_len} but decoded {decoded_packet_data_len} bytes"
                )));
            }
        }

        let payload = self
            .payload_scratch
            .split_to(self.payload_scratch.len())
            .freeze();

        self.last_packet = Some((packet_id.0, packet_len, payload.len()));
        Ok(RawPacket {
            id: packet_id.0,
            payload,
        })
    }
}
/// Encoder: Client -> Server. Supports `ZLib` encoding/compression and AES-128
/// encryption.
pub struct TcpNetworkEncoder<W: AsyncWrite + Unpin> {
    writer: Option<EncryptionWriter<W>>,
    // compression and compression threshold
    compression: Option<(CompressionThreshold, CompressionLevel)>,
    // Reused compressor to avoid constructing zlib state per packet.
    compressor: Option<(CompressionLevel, Compress)>,
    // Reused compression buffer to avoid allocating a new Vec for each packet.
    compression_scratch: Vec<u8>,
    frame_scratch: Vec<u8>,
}

impl<W: AsyncWrite + Unpin> TcpNetworkEncoder<W> {
    pub const fn new(writer: W) -> Self {
        Self {
            writer: Some(EncryptionWriter::None(writer)),
            compression: None,
            compressor: None,
            compression_scratch: Vec::new(),
            frame_scratch: Vec::new(),
        }
    }

    pub const fn set_compression(
        &mut self,
        compression_info: (CompressionThreshold, CompressionLevel),
    ) {
        self.compression = Some(compression_info);
    }

    /// NOTE: Encryption can only be set; a minecraft stream cannot go back to being unencrypted
    pub fn set_encryption(&mut self, key: &[u8; 16]) -> Result<(), PacketEncodeError> {
        if matches!(self.writer, Some(EncryptionWriter::Encrypt(_))) {
            return Err(PacketEncodeError::Message(
                "Encryption already enabled".into(),
            ));
        }
        let cipher = Aes128Cfb8Enc::new_from_slices(key, key)
            .map_err(|_| PacketEncodeError::Message("Invalid key".into()))?;

        if let Some(writer) = self.writer.take() {
            self.writer = Some(writer.upgrade(cipher));
        }
        Ok(())
    }

    fn compress_packet_data(
        &mut self,
        packet_data: &[u8],
        compression_level: CompressionLevel,
    ) -> Result<(), PacketEncodeError> {
        self.compression_scratch.clear();
        let reserve_hint = packet_data
            .len()
            .saturating_add(packet_data.len() / 16)
            .saturating_add(64);
        let current_capacity = self.compression_scratch.capacity();
        if reserve_hint > current_capacity {
            self.compression_scratch
                .reserve(reserve_hint.saturating_sub(current_capacity));
        }

        let needs_new_compressor = match self.compressor.as_ref() {
            Some((level, _)) => *level != compression_level,
            None => true,
        };
        if needs_new_compressor {
            self.compressor = Some((
                compression_level,
                Compress::new(Compression::new(compression_level), true),
            ));
        }

        let (_, compressor) = self.compressor.as_mut().ok_or_else(|| {
            PacketEncodeError::Message("compressor must be present after initialization".into())
        })?;
        compressor.reset();
        let status = compressor
            .compress_vec(
                packet_data,
                &mut self.compression_scratch,
                FlushCompress::Finish,
            )
            .map_err(|err| PacketEncodeError::CompressionFailed(err.to_string()))?;

        if !matches!(status, Status::StreamEnd) {
            return Err(PacketEncodeError::CompressionFailed(format!(
                "Unexpected compressor status: {status:?}"
            )));
        }
        Ok(())
    }

    /// Appends a packet (id VarInt + payload) to the stream with framing and,
    /// when enabled and large enough, compression.
    ///
    /// NOTE: This method does not flush. Call [`Self::flush`] to flush buffered data.
    pub async fn write_packet(&mut self, packet_data: Bytes) -> Result<(), PacketEncodeError> {
        let mut frame = std::mem::take(&mut self.frame_scratch);
        frame.clear();
        let framed = self.frame_packet(&packet_data, &mut frame);
        let result = match framed {
            Ok(()) => self.write_frame(&frame).await,
            Err(err) => Err(err),
        };
        self.frame_scratch = frame;
        result
    }

    pub async fn write_frame(&mut self, frame: &[u8]) -> Result<(), PacketEncodeError> {
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| PacketEncodeError::Message("Writer missing".into()))?;
        writer
            .write_all(frame)
            .await
            .map_err(|err| PacketEncodeError::Message(err.to_string()))
    }

    #[must_use]
    pub fn is_compressing_packet(&self, packet_data: &Bytes) -> bool {
        self.compression
            .is_some_and(|(threshold, _)| packet_data.len() >= threshold)
    }

    pub fn frame_packet(
        &mut self,
        packet_data: &Bytes,
        out: &mut Vec<u8>,
    ) -> Result<(), PacketEncodeError> {
        let data_len = packet_data.len();
        if data_len > MAX_PACKET_DATA_SIZE {
            return Err(PacketEncodeError::TooLong(data_len));
        }

        let data_len_var_int: VarInt = data_len.try_into().map_err(|_| {
            PacketEncodeError::Message(format!(
                "Packet data length is too large to fit in VarInt! ({data_len})"
            ))
        })?;

        let mut header_buf = [0u8; 10];
        let mut header_cursor = std::io::Cursor::new(&mut header_buf[..]);

        let payload_to_write: &[u8] = if let Some((compression_threshold, compression_level)) =
            self.compression
        {
            if data_len >= compression_threshold {
                self.compress_packet_data(packet_data.as_ref(), compression_level)?;
                debug_assert!(!self.compression_scratch.is_empty());

                let full_packet_len_var_int: VarInt = (data_len_var_int.written_size()
                    + self.compression_scratch.len())
                .try_into()
                .map_err(|_| {
                    PacketEncodeError::Message(format!(
                        "Full packet length is too large to fit in VarInt! ({data_len})"
                    ))
                })?;

                let complete_serialization_length =
                    full_packet_len_var_int.written_size() + full_packet_len_var_int.0 as usize;
                if complete_serialization_length > MAX_PACKET_SIZE as usize {
                    return Err(PacketEncodeError::TooLong(complete_serialization_length));
                }

                full_packet_len_var_int
                    .encode(&mut header_cursor)
                    .map_err(|err| PacketEncodeError::Message(err.to_string()))?;
                data_len_var_int
                    .encode(&mut header_cursor)
                    .map_err(|err| PacketEncodeError::Message(err.to_string()))?;

                self.compression_scratch.as_slice()
            } else {
                let data_len_var_int: VarInt = 0.into();
                let full_packet_len_var_int: VarInt = (data_len_var_int.written_size() + data_len)
                    .try_into()
                    .map_err(|_| {
                        PacketEncodeError::Message(format!(
                            "Full packet length is too large to fit in VarInt! ({data_len})"
                        ))
                    })?;

                let complete_serialization_length =
                    full_packet_len_var_int.written_size() + full_packet_len_var_int.0 as usize;
                if complete_serialization_length > MAX_PACKET_SIZE as usize {
                    return Err(PacketEncodeError::TooLong(complete_serialization_length));
                }

                full_packet_len_var_int
                    .encode(&mut header_cursor)
                    .map_err(|err| PacketEncodeError::Message(err.to_string()))?;
                data_len_var_int
                    .encode(&mut header_cursor)
                    .map_err(|err| PacketEncodeError::Message(err.to_string()))?;

                packet_data.as_ref()
            }
        } else {
            let full_packet_len_var_int = data_len_var_int;

            let complete_serialization_length =
                full_packet_len_var_int.written_size() + full_packet_len_var_int.0 as usize;
            if complete_serialization_length > MAX_PACKET_SIZE as usize {
                return Err(PacketEncodeError::TooLong(complete_serialization_length));
            }

            full_packet_len_var_int
                .encode(&mut header_cursor)
                .map_err(|err| PacketEncodeError::Message(err.to_string()))?;

            packet_data.as_ref()
        };

        let header_len = header_cursor.position() as usize;
        let header_bytes = &header_buf[..header_len];

        out.reserve(header_len + payload_to_write.len());
        out.extend_from_slice(header_bytes);
        out.extend_from_slice(payload_to_write);

        Ok(())
    }

    pub async fn flush(&mut self) -> Result<(), PacketEncodeError> {
        self.writer
            .as_mut()
            .ok_or_else(|| PacketEncodeError::Message("Writer missing".into()))?
            .flush()
            .await
            .map_err(|err| PacketEncodeError::Message(err.to_string()))
    }
}
