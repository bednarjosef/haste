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

use aes::cipher::KeyIvInit;
use bytes::{Bytes, BytesMut};
use flate2::{Compress, Compression, FlushCompress, Status};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use crate::wire::varint::{VarError, VarInt};

/// Vanilla's maximum packet length (2 MiB), enforced on read and write.
pub const MAX_PACKET_SIZE: u64 = 2_097_152;
/// Vanilla's maximum decompressed packet data size (8 MiB).
pub const MAX_PACKET_DATA_SIZE: usize = 8_388_608;

// decrypt -> decompress -> raw
type Aes128Cfb8Dec = cfb8::Decryptor<aes::Aes128>;
type Aes128Cfb8Enc = cfb8::Encryptor<aes::Aes128>;

/// The threshold determines the minimum size of data that should be compressed.
/// Data smaller than the threshold will not be compressed.
pub type CompressionThreshold = usize;

/// The level controls the amount of compression applied to the data.
/// Higher levels generally result in higher compression ratios, but also
/// increase CPU usage.
pub type CompressionLevel = u32;

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

pub enum DecompressionReader<R: AsyncRead + Unpin> {
    Decompress(async_compression::tokio::bufread::ZlibDecoder<BufReader<R>>),
    None(R),
}

impl<R: AsyncRead + Unpin> AsyncRead for DecompressionReader<R> {
    #[inline]
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Decompress(reader) => {
                let reader = std::pin::Pin::new(reader);
                reader.poll_read(cx, buf)
            }
            Self::None(reader) => {
                let reader = std::pin::Pin::new(reader);
                reader.poll_read(cx, buf)
            }
        }
    }
}

pub enum DecryptionReader<R: AsyncRead + Unpin> {
    Decrypt(Box<StreamDecryptor<R>>),
    None(R),
}

impl<R: AsyncRead + Unpin> DecryptionReader<R> {
    #[must_use]
    pub fn upgrade(self, cipher: Aes128Cfb8Dec) -> Self {
        match self {
            Self::None(stream) => Self::Decrypt(Box::new(StreamDecryptor::new(cipher, stream))),
            Self::Decrypt(_) => self,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for DecryptionReader<R> {
    #[inline]
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Decrypt(reader) => {
                let reader = std::pin::Pin::new(reader);
                reader.poll_read(cx, buf)
            }
            Self::None(reader) => {
                let reader = std::pin::Pin::new(reader);
                reader.poll_read(cx, buf)
            }
        }
    }
}

/// Decoder: Server -> Client. Supports `ZLib` decoding/decompression and AES-128
/// encryption.
pub struct TcpNetworkDecoder<R: AsyncRead + Unpin> {
    reader: Option<DecryptionReader<R>>,
    compression: Option<CompressionThreshold>,
    payload_scratch: BytesMut,
}

impl<R: AsyncRead + Unpin> TcpNetworkDecoder<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader: Some(DecryptionReader::None(reader)),
            compression: None,
            payload_scratch: BytesMut::new(),
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
        let reader = self
            .reader
            .as_mut()
            .ok_or_else(|| PacketDecodeError::Message("Reader missing".into()))?;

        let packet_len = VarInt::decode_async(reader)
            .await
            .map_err(|err| PacketDecodeError::MalformedLength(err.to_string()))?;

        let packet_len = packet_len.0 as u64;

        if !(0..=MAX_PACKET_SIZE).contains(&packet_len) {
            Err(PacketDecodeError::OutOfBounds)?;
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
                Err(PacketDecodeError::TooLong)?;
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
                    Err(PacketDecodeError::NotCompressed)?;
                }

                expected_packet_data_len = raw_packet_length as usize;
                DecompressionReader::None(bounded_reader)
            }
        } else {
            DecompressionReader::None(bounded_reader)
        };

        let (packet_id, packet_id_len) = VarInt::decode_async_sized(&mut reader)
            .await
            .map_err(|_| PacketDecodeError::DecodeID)?;

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

        Ok(RawPacket {
            id: packet_id.0,
            payload,
        })
    }
}

pub enum EncryptionWriter<W: AsyncWrite + Unpin> {
    Encrypt(Box<StreamEncryptor<W>>),
    None(W),
}

impl<W: AsyncWrite + Unpin> EncryptionWriter<W> {
    #[must_use]
    pub fn upgrade(self, cipher: Aes128Cfb8Enc) -> Self {
        match self {
            Self::None(stream) => Self::Encrypt(Box::new(StreamEncryptor::new(cipher, stream))),
            Self::Encrypt(_) => self,
        }
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for EncryptionWriter<W> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        match self.get_mut() {
            Self::Encrypt(writer) => {
                let writer = std::pin::Pin::new(writer);
                writer.poll_write(cx, buf)
            }
            Self::None(writer) => {
                let writer = std::pin::Pin::new(writer);
                writer.poll_write(cx, buf)
            }
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        match self.get_mut() {
            Self::Encrypt(writer) => {
                let writer = std::pin::Pin::new(writer);
                writer.poll_flush(cx)
            }
            Self::None(writer) => {
                let writer = std::pin::Pin::new(writer);
                writer.poll_flush(cx)
            }
        }
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        match self.get_mut() {
            Self::Encrypt(writer) => {
                let writer = std::pin::Pin::new(writer);
                writer.poll_shutdown(cx)
            }
            Self::None(writer) => {
                let writer = std::pin::Pin::new(writer);
                writer.poll_shutdown(cx)
            }
        }
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

/// AES-128-CFB8 decrypting reader. CFB8 has a one-byte block, so decrypting
/// whatever the inner reader produced, in place, is always correct.
pub struct StreamDecryptor<R: AsyncRead + Unpin> {
    cipher: Aes128Cfb8Dec,
    read: R,
}

impl<R: AsyncRead + Unpin> StreamDecryptor<R> {
    /// Wraps `stream`, decrypting everything read from it with `cipher`.
    pub const fn new(cipher: Aes128Cfb8Dec, stream: R) -> Self {
        Self {
            cipher,
            read: stream,
        }
    }
}

/// Decrypts `data` in place with a CFB8 decryptor (one AES op per byte).
pub(crate) fn cfb8_decrypt(cipher: &mut Aes128Cfb8Dec, data: &mut [u8]) {
    use aes::cipher::{generic_array::GenericArray, BlockDecryptMut};
    for b in data.chunks_mut(1) {
        cipher.decrypt_block_mut(GenericArray::from_mut_slice(b));
    }
}

/// Encrypts `data` in place with a CFB8 encryptor.
pub(crate) fn cfb8_encrypt(cipher: &mut Aes128Cfb8Enc, data: &mut [u8]) {
    use aes::cipher::{generic_array::GenericArray, BlockEncryptMut};
    for b in data.chunks_mut(1) {
        cipher.encrypt_block_mut(GenericArray::from_mut_slice(b));
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for StreamDecryptor<R> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let original_fill = buf.filled().len();
        let poll = std::pin::Pin::new(&mut this.read).poll_read(cx, buf);
        if matches!(poll, std::task::Poll::Ready(Ok(()))) {
            cfb8_decrypt(&mut this.cipher, &mut buf.filled_mut()[original_fill..]);
        }
        poll
    }
}

/// AES-128-CFB8 encrypting writer.
///
/// HASTE: rewritten from Pumpkin's byte-at-a-time writer. Accepted bytes are
/// encrypted into an internal buffer (the cipher state advances exactly once
/// per byte) and drained to the inner writer; `poll_flush` drains first.
pub struct StreamEncryptor<W: AsyncWrite + Unpin> {
    cipher: Aes128Cfb8Enc,
    write: W,
    pending: Vec<u8>,
    pending_pos: usize,
}

impl<W: AsyncWrite + Unpin> StreamEncryptor<W> {
    /// Wraps `stream`, encrypting everything written to it with `cipher`.
    pub fn new(cipher: Aes128Cfb8Enc, stream: W) -> Self {
        Self {
            cipher,
            write: stream,
            pending: Vec::new(),
            pending_pos: 0,
        }
    }

    fn poll_drain(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        while self.pending_pos < self.pending.len() {
            let w = std::pin::Pin::new(&mut self.write);
            match w.poll_write(cx, &self.pending[self.pending_pos..]) {
                std::task::Poll::Ready(Ok(0)) => {
                    return std::task::Poll::Ready(Err(std::io::ErrorKind::WriteZero.into()))
                }
                std::task::Poll::Ready(Ok(n)) => self.pending_pos += n,
                std::task::Poll::Ready(Err(e)) => return std::task::Poll::Ready(Err(e)),
                std::task::Poll::Pending => return std::task::Poll::Pending,
            }
        }
        self.pending.clear();
        self.pending_pos = 0;
        std::task::Poll::Ready(Ok(()))
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for StreamEncryptor<W> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        let this = self.get_mut();
        match this.poll_drain(cx) {
            std::task::Poll::Ready(Ok(())) => {}
            other => return other.map(|r| r.map(|()| 0)),
        }
        let start = this.pending.len();
        this.pending.extend_from_slice(buf);
        cfb8_encrypt(&mut this.cipher, &mut this.pending[start..]);
        // The bytes are accepted (cipher state advanced); try to push them now,
        // leftovers go out on the next write or flush.
        match this.poll_drain(cx) {
            std::task::Poll::Ready(Err(e)) => std::task::Poll::Ready(Err(e)),
            _ => std::task::Poll::Ready(Ok(buf.len())),
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        let this = self.get_mut();
        match this.poll_drain(cx) {
            std::task::Poll::Ready(Ok(())) => std::pin::Pin::new(&mut this.write).poll_flush(cx),
            other => other,
        }
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        let this = self.get_mut();
        match this.poll_drain(cx) {
            std::task::Poll::Ready(Ok(())) => std::pin::Pin::new(&mut this.write).poll_shutdown(cx),
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::AsyncStreamCipher;

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
        let decryptor = cfb8::Decryptor::<aes::Aes128>::new_from_slices(key, iv)
            .map_err(|_| "Invalid key/iv")?;
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
            let encryptor = cfb8::Encryptor::<aes::Aes128>::new_from_slices(k, v)
                .map_err(|_| "Invalid key/iv")?;
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
        // boundaries, like chunk sections and small packets during a join.
        let sizes = [
            0usize, 1, 5, 100, 255, 256, 257, 300, 1_000, 4_000, 8_191, 8_192, 8_193, 16_384,
            40_000, 150_000, 300_000,
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
}
