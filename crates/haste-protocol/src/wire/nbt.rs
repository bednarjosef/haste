//! Network NBT: one value with a type id byte and no root name and no length
//! prefix; `TAG_End` means "none". Reading measures the value's length with
//! the NBT skip helper before taking the bytes.

use bytes::Bytes;

use super::{DecodeError, DecodeResult, Reader, Wire, Writer};
use crate::nbt::{NbtCompound, NbtTag};

impl Reader<'_> {
    /// Length in bytes of the network NBT value at the cursor (type id byte +
    /// payload, no root name).
    fn nbt_len(&self) -> DecodeResult<usize> {
        use crate::nbt::deserializer::NbtReadHelper;
        let all = self.buf;
        let mut cursor = std::io::Cursor::new(all);
        let mut helper = crate::nbt::deserializer::NbtReadHelperJava::new(&mut cursor);
        let id = helper.get_u8()?;
        NbtTag::skip_data(&mut helper, id)?;
        let used = cursor.position() as usize;
        if used > all.len() {
            return Err(DecodeError::Eof {
                needed: used,
                remaining: all.len(),
            });
        }
        Ok(used)
    }

    /// One network NBT value, kept as raw bytes. A lone `TAG_End` is the
    /// single byte `[0]`.
    pub fn nbt_raw(&mut self) -> DecodeResult<Bytes> {
        let n = self.nbt_len()?;
        let s = self.take(n)?;
        Ok(self.to_bytes(s))
    }

    /// One network NBT value, parsed. `None` for `TAG_End`.
    pub fn nbt_tag(&mut self) -> DecodeResult<Option<NbtTag>> {
        use crate::nbt::deserializer::NbtReadHelper;
        let n = self.nbt_len()?;
        let s = self.take(n)?;
        let mut cursor = std::io::Cursor::new(s);
        let mut helper = crate::nbt::deserializer::NbtReadHelperJava::new(&mut cursor);
        let id = helper.get_u8()?;
        if id == crate::nbt::END_ID {
            return Ok(None);
        }
        Ok(Some(NbtTag::deserialize_data(&mut helper, id)?))
    }

    /// Network NBT that must be a compound (or `TAG_End` for none).
    pub fn nbt_compound(&mut self) -> DecodeResult<Option<NbtCompound>> {
        match self.nbt_tag()? {
            None => Ok(None),
            Some(NbtTag::Compound(c)) => Ok(Some(c)),
            Some(other) => Err(DecodeError::Invalid(format!(
                "expected NBT compound, got tag id {}",
                other.get_type_id()
            ))),
        }
    }
}

impl Writer {
    /// One network NBT value (`None` writes `TAG_End`).
    pub fn nbt_tag(&mut self, tag: Option<&NbtTag>) {
        match tag {
            None | Some(NbtTag::End) => self.u8(crate::nbt::END_ID),
            Some(t) => {
                let mut w = crate::nbt::serializer::NbtWriteHelperJava::new(&mut self.buf);
                // Writing into a Vec only fails for strings over 65535 bytes,
                // which we never build; treat it as a programming error.
                t.clone()
                    .serialize(&mut w)
                    .expect("NBT serialisation into a Vec");
            }
        }
    }
    /// Network NBT compound.
    pub fn nbt_compound(&mut self, c: Option<&NbtCompound>) {
        match c {
            None => self.u8(crate::nbt::END_ID),
            Some(c) => self.nbt_tag(Some(&NbtTag::Compound(c.clone()))),
        }
    }
}

/// Network NBT; `TAG_End` decodes to `NbtTag::End`.
impl Wire for NbtTag {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(r.nbt_tag()?.unwrap_or(NbtTag::End))
    }
    fn write(&self, w: &mut Writer) {
        w.nbt_tag(Some(self))
    }
}
