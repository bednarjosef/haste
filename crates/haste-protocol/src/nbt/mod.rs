//! NBT (Named Binary Tag) reading and writing for the Java Edition wire format.
//!
//! Vendored and adapted from Pumpkin's `pumpkin-nbt` crate (GPL-3.0; Pumpkin's
//! files carry no per-file header, each vendored file names its origin).
//! Changes for Haste: the gzip and codec-ops modules are not vendored, a
//! missing `TAG_End` is an error, and it lives as a module of
//! `haste-protocol` instead of a standalone crate (edition 2021).
//! See THIRD_PARTY.md.
//!
//! Reading, writing, and manipulating Minecraft's Named Binary Tag (NBT) data.
//!
//! The crate supports the standard Java Edition representation and unnamed network
//! NBT. Data is handled directly via [`Nbt`], [`NbtCompound`], and [`NbtTag`].

use std::{
    io::{self, Write},
    ops::Deref,
};

use bytes::Bytes;
use deserializer::NbtReadHelper;
use serializer::{NbtWriteHelper, NbtWriteHelperJava};
use thiserror::Error;

/// Compound-tag storage and construction helpers.
pub mod compound;
/// Low-level NBT deserialization support.
pub mod deserializer;
/// Low-level NBT serialization support.
pub mod serializer;
/// The individual NBT tag types.
pub mod tag;

pub use compound::NbtCompound;
pub use tag::NbtTag;

/// Numeric identifier for an end tag.
pub const END_ID: u8 = 0x00;
/// Numeric identifier for a byte tag.
pub const BYTE_ID: u8 = 0x01;
/// Numeric identifier for a short tag.
pub const SHORT_ID: u8 = 0x02;
/// Numeric identifier for an integer tag.
pub const INT_ID: u8 = 0x03;
/// Numeric identifier for a long tag.
pub const LONG_ID: u8 = 0x04;
/// Numeric identifier for a float tag.
pub const FLOAT_ID: u8 = 0x05;
/// Numeric identifier for a double tag.
pub const DOUBLE_ID: u8 = 0x06;
/// Numeric identifier for a byte-array tag.
pub const BYTE_ARRAY_ID: u8 = 0x07;
/// Numeric identifier for a string tag.
pub const STRING_ID: u8 = 0x08;
/// Numeric identifier for a list tag.
pub const LIST_ID: u8 = 0x09;
/// Numeric identifier for a compound tag.
pub const COMPOUND_ID: u8 = 0x0A;
/// Numeric identifier for an integer-array tag.
pub const INT_ARRAY_ID: u8 = 0x0B;
/// Numeric identifier for a long-array tag.
pub const LONG_ARRAY_ID: u8 = 0x0C;

/// Maximum number of elements accepted when decoding a list or array.
pub const MAX_ARRAY_LENGTH: usize = 512_000;
/// Maximum nesting depth allowed when decoding NBT compound or list tags.
pub const MAX_NBT_DEPTH: usize = 512;

/// Errors produced while reading, writing, or converting NBT data.
#[derive(Error, Debug)]
pub enum Error {
    /// The root tag was not a compound tag and contains the reported tag ID.
    #[error("The root tag of the NBT file is not a compound tag. Received tag id: {0}")]
    NoRootCompound(u8),
    /// A tag ID not defined by the NBT format was encountered.
    #[error("Encountered an unknown NBT tag id: {0}.")]
    UnknownTagId(u8),
    /// A Java CESU-8 string could not be decoded.
    #[error("Failed to Cesu 8 Decode")]
    Cesu8DecodingError,
    /// A string could not be decoded as UTF-8.
    #[error("Failed to UTF-8 Decode")]
    Utf8DecodingError,
    /// Serde reported an invalid value or serializer state.
    #[error("Serde error: {0}")]
    SerdeError(String),
    /// The requested Rust type has no NBT representation.
    #[error("NBT doesn't support this type: {0}")]
    UnsupportedType(String),
    /// The underlying reader or writer returned an I/O error.
    #[error("NBT reading was cut short: {0}")]
    Incomplete(io::Error),
    /// A list or array declared a negative element count.
    #[error("Negative list length: {0}")]
    NegativeLength(i32),
    /// A string, list, or array exceeded the supported length.
    #[error("Length too large: {0}")]
    LargeLength(usize),
    /// A Bedrock variable-length integer exceeded its maximum encoded size.
    #[error("Failed to decode varint - value too large")]
    VarIntTooLarge,
    /// A Bedrock variable-length long exceeded its maximum encoded size.
    #[error("Failed to decode varlong - value too large")]
    VarLongTooLarge,
    /// NBT nesting depth exceeded the maximum allowed limit.
    #[error("NBT depth exceeded maximum allowed limit")]
    MaxDepthExceeded,
    /// A list tag specified an invalid element tag type.
    #[error("Invalid element tag type for list: {0}")]
    InvalidListTag(u8),
}

/// A complete NBT document containing a named root compound.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Nbt {
    /// Name stored alongside the root compound.
    pub name: String,
    /// Root compound containing the document's tags.
    pub root_tag: NbtCompound,
}

impl Nbt {
    /// Creates a document from a root name and compound.
    #[must_use]
    pub const fn new(name: String, tag: NbtCompound) -> Self {
        Self {
            name,
            root_tag: tag,
        }
    }

    /// Reads a named NBT document from a format-specific reader.
    ///
    /// Returns [`Error::NoRootCompound`] when the first tag is not a compound.
    pub fn read<'a, R: NbtReadHelper<'a>>(reader: &mut R) -> Result<Self, Error> {
        let tag_type_id = reader.get_u8()?;

        if tag_type_id != COMPOUND_ID {
            return Err(Error::NoRootCompound(tag_type_id));
        }

        Ok(Self {
            name: reader.get_string()?.into_owned(),
            root_tag: NbtCompound::deserialize_content(reader)?,
        })
    }

    /// Reads an NBT document that omits the root compound's name.
    ///
    /// The returned document has an empty [`Self::name`].
    pub fn read_unnamed<'a, R: NbtReadHelper<'a>>(reader: &mut R) -> Result<Self, Error> {
        let tag_type_id = reader.get_u8()?;

        if tag_type_id != COMPOUND_ID {
            return Err(Error::NoRootCompound(tag_type_id));
        }

        Ok(Self {
            name: String::new(),
            root_tag: NbtCompound::deserialize_content(reader)?,
        })
    }

    /// Serializes this document using the Java Edition NBT representation.
    #[must_use]
    pub fn write(self) -> Bytes {
        let mut bytes = Vec::new();
        let mut writer = NbtWriteHelperJava::new(&mut bytes);
        if writer.write_u8(COMPOUND_ID).is_ok()
            && NbtTag::String(self.name.into())
                .serialize_data(&mut writer)
                .is_ok()
        {
            let _ = self.root_tag.serialize_content(&mut writer);
        }

        bytes.into()
    }

    /// Writes this document in the Java Edition representation.
    pub fn write_to_writer<W: Write>(self, mut writer: W) -> Result<(), io::Error> {
        writer.write_all(&self.write())?;
        Ok(())
    }

    /// Serializes this document without the root compound's name.
    #[must_use]
    pub fn write_unnamed(self) -> Bytes {
        let mut bytes = Vec::new();
        let mut writer = NbtWriteHelperJava::new(&mut bytes);

        if writer.write_u8(COMPOUND_ID).is_ok() {
            let _ = self.root_tag.serialize_content(&mut writer);
        }

        bytes.into()
    }

    /// Writes this document without the root compound's name.
    pub fn write_unnamed_to_writer<W: Write>(self, mut writer: W) -> Result<(), io::Error> {
        writer.write_all(&self.write_unnamed())?;
        Ok(())
    }
}

impl Deref for Nbt {
    type Target = NbtCompound;

    fn deref(&self) -> &Self::Target {
        &self.root_tag
    }
}

impl From<NbtCompound> for Nbt {
    fn from(value: NbtCompound) -> Self {
        Self::new(String::new(), value)
    }
}

impl<T> AsRef<T> for Nbt
where
    T: ?Sized,
    <Self as Deref>::Target: AsRef<T>,
{
    fn as_ref(&self) -> &T {
        self.deref().as_ref()
    }
}

impl AsMut<NbtCompound> for Nbt {
    fn as_mut(&mut self) -> &mut NbtCompound {
        &mut self.root_tag
    }
}

#[cfg(test)]
mod haste_tests {
    use super::*;
    use crate::wire::{Reader, Writer};

    #[test]
    fn every_tag_type_round_trips_as_network_nbt() {
        let mut inner = NbtCompound::new();
        inner.put_string("name", "stone ✓".into());
        let mut c = NbtCompound::new();
        c.put_byte("b", -3);
        c.put_short("s", 300);
        c.put_int("i", -70000);
        c.put_long("l", 1 << 40);
        c.put_float("f", 1.5);
        c.put_double("d", -2.25);
        c.put("ba", NbtTag::ByteArray(vec![1i8, -1].into_boxed_slice()));
        c.put_list("list", vec![NbtTag::Int(1), NbtTag::Int(2)]);
        c.put("cmp", NbtTag::Compound(inner));
        c.put("ia", NbtTag::IntArray(vec![5, 6]));
        c.put("la", NbtTag::LongArray(vec![7, 8]));
        let mut w = Writer::new();
        w.nbt_compound(Some(&c));
        // Unnamed root: type byte then content directly (no name length).
        assert_eq!(w.buf[0], COMPOUND_ID);
        let mut r = Reader::new(&w.buf);
        assert_eq!(r.nbt_compound().unwrap(), Some(c));
        r.finish().unwrap();
    }

    #[test]
    fn named_document_round_trip() {
        let mut c = NbtCompound::new();
        c.put_int("x", 1);
        let bytes = Nbt::new("root".into(), c.clone()).write();
        let mut cursor = std::io::Cursor::new(&bytes[..]);
        let mut helper = deserializer::NbtReadHelperJava::new(&mut cursor);
        let doc = Nbt::read(&mut helper).unwrap();
        assert_eq!(doc.name, "root");
        assert_eq!(doc.root_tag, c);
    }
}
