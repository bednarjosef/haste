//! Wire primitives: the data types that appear inside packet bodies.
//!
//! [`Reader`] reads from a byte slice (zero-copy for large payloads when it is
//! built from a [`Bytes`]); [`Writer`] appends to a `Vec<u8>` and cannot fail.
//! [`Wire`] is implemented by every type with one canonical encoding; where
//! the same Rust type has several encodings (an `i32` as `Int` or `VarInt`, an
//! `f32` as a float or an angle byte) a [`Codec`] marker type selects one.
//!
//! Layouts follow the minecraft.wiki "Java Edition protocol/Data types" page
//! (26.3, protocol 777). VarInt/VarLong decoding comes from Pumpkin (`varint`).

pub mod codecs;
pub mod collections;
pub mod component;
pub mod components;
pub mod nbt;
pub mod particle;
pub mod positions;
pub mod primitives;
pub mod slot;
pub mod strings;
pub mod text_component;
pub mod varint;

pub use codecs::{
    Angle, BitSet, ByteArray, Codec, GameProfile, IdOr, IdSet, Json, List, LpVec3, Native, Opt,
    ProfileProperty, RawNbt, ResolvableProfile, ResolvableProfileKind, Rest, SoundEvent, VarInt,
    VarLong, Wire,
};

use bytes::Bytes;

/// Vanilla's default maximum string length, in UTF-16 code units.
pub const MAX_STRING: usize = 32767;
/// Maximum length of a JSON text component string.
pub const MAX_JSON_COMPONENT: usize = 262_144;
/// Sanity cap for prefixed array lengths.
pub const MAX_LIST: usize = 1 << 21;

/// Error decoding a packet body.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// The body ended early.
    #[error("unexpected end of packet: needed {needed} bytes, {remaining} remain")]
    Eof {
        /// Bytes the read needed.
        needed: usize,
        /// Bytes that were left.
        remaining: usize,
    },
    /// A length exceeded its protocol limit.
    #[error("too large: {0}")]
    TooLarge(String),
    /// A value is not valid for its type.
    #[error("invalid value: {0}")]
    Invalid(String),
    /// The packet decoded but bytes were left over (layout mismatch).
    #[error("{0} trailing bytes after packet body")]
    Trailing(usize),
    /// NBT payload error.
    #[error("NBT: {0}")]
    Nbt(#[from] crate::nbt::Error),
    /// JSON payload error.
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
}

impl From<varint::VarError> for DecodeError {
    fn from(e: varint::VarError) -> Self {
        DecodeError::Invalid(e.to_string())
    }
}

/// Result of a decode.
pub type DecodeResult<T> = Result<T, DecodeError>;

/// A cursor over a packet body. Every read advances it.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    buf: &'a [u8],
    /// The owning buffer, when known, so byte fields can be sliced out
    /// without copying.
    src: Option<&'a Bytes>,
    tables: Option<&'a crate::packets::ids::ProtocolTables>,
}

impl<'a> Reader<'a> {
    /// Reads from a plain slice; byte fields are copied out.
    pub fn new(buf: &'a [u8]) -> Self {
        Self {
            buf,
            src: None,
            tables: None,
        }
    }

    /// Reads from a `Bytes`; byte fields are zero-copy slices of it.
    pub fn from_bytes(b: &'a Bytes) -> Self {
        Self {
            buf: b,
            src: Some(b),
            tables: None,
        }
    }

    /// Attaches registry tables (needed to decode item stacks with data
    /// components, whose layouts depend on the component type's name).
    pub fn with_tables(mut self, t: &'a crate::packets::ids::ProtocolTables) -> Self {
        self.tables = Some(t);
        self
    }

    /// The registry tables, if attached.
    pub fn tables(&self) -> Option<&'a crate::packets::ids::ProtocolTables> {
        self.tables
    }

    /// Bytes not yet consumed.
    pub fn remaining(&self) -> usize {
        self.buf.len()
    }

    /// The bytes not yet consumed, without consuming them.
    pub fn peek_rest(&self) -> &'a [u8] {
        self.buf
    }

    /// Fails with [`DecodeError::Trailing`] if bytes are left.
    pub fn finish(&self) -> DecodeResult<()> {
        if self.buf.is_empty() {
            Ok(())
        } else {
            Err(DecodeError::Trailing(self.buf.len()))
        }
    }

    /// Consumes `len` bytes.
    pub fn take(&mut self, len: usize) -> DecodeResult<&'a [u8]> {
        if self.buf.len() < len {
            return Err(DecodeError::Eof {
                needed: len,
                remaining: self.buf.len(),
            });
        }
        let (head, tail) = self.buf.split_at(len);
        self.buf = tail;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> DecodeResult<[u8; N]> {
        let s = self.take(N)?;
        let mut a = [0u8; N];
        a.copy_from_slice(s);
        Ok(a)
    }

    /// Turns a slice previously taken from this reader into `Bytes`
    /// (zero-copy if the reader was built from `Bytes`).
    pub fn to_bytes(&self, slice: &'a [u8]) -> Bytes {
        match self.src {
            Some(src) if slice.is_empty() => src.slice(0..0),
            Some(src) => src.slice_ref(slice),
            None => Bytes::copy_from_slice(slice),
        }
    }

    /// Consumes everything left.
    pub fn rest(&mut self) -> Bytes {
        let s = self.buf;
        self.buf = &[];
        self.to_bytes(s)
    }

    /// Reads a value with its canonical encoding.
    pub fn read<T: Wire>(&mut self) -> DecodeResult<T> {
        T::read(self)
    }

    /// Reads a value with an explicit codec.
    pub fn read_as<T, C: Codec<T>>(&mut self) -> DecodeResult<T> {
        C::read(self)
    }
}

/// Appends packet-body data to a buffer. Infallible: limits are the caller's
/// job (serverbound packets are built by us).
#[derive(Debug, Default, Clone)]
pub struct Writer {
    /// The bytes written so far.
    pub buf: Vec<u8>,
}

impl Writer {
    /// An empty writer.
    pub fn new() -> Self {
        Self::default()
    }
    /// The bytes written.
    pub fn into_inner(self) -> Vec<u8> {
        self.buf
    }
    /// Writes a value with its canonical encoding.
    pub fn write<T: Wire>(&mut self, v: &T) {
        v.write(self)
    }
    /// Writes a value with an explicit codec.
    pub fn write_as<T, C: Codec<T>>(&mut self, v: &T) {
        C::write(v, self)
    }
}

#[cfg(test)]
mod tests;
