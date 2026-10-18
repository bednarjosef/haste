//! AES-128-CFB8 stream encryption and decryption.
//!
//! Vendored from Pumpkin `pumpkin-protocol` (GPL-3.0; see THIRD_PARTY.md), moved here out of
//! `transport/mod.rs`.

// decrypt -> decompress -> raw
pub(crate) type Aes128Cfb8Dec = cfb8::Decryptor<aes::Aes128>;
pub(crate) type Aes128Cfb8Enc = cfb8::Encryptor<aes::Aes128>;

use tokio::io::{AsyncRead, AsyncWrite};

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
