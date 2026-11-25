//! VarInt/VarLong (and unsigned forms) wire primitives for the Java protocol.
//!
//! Vendored from Pumpkin `pumpkin-protocol` (GPL-3.0), `src/codec/{var_int,
//! var_long,var_uint,var_ulong}.rs`; see docs/THIRD_PARTY.md. Adapted for Haste:
//! the Bedrock zig-zag `PacketRead`/`PacketWrite` impls are removed, leaving the
//! Java Edition LEB128 forms only, and the `NetworkReadExt`/`NetworkWriteExt`
//! dependency is inlined so these are self-contained.

use std::{
    io::{Read, Write},
    num::NonZero,
    ops::Deref,
};

use thiserror::Error;

/// Error reading or writing a VarInt/VarLong.
#[derive(Debug, Error)]
#[error("varint/varlong error: {0}")]
pub struct VarError(pub String);

impl VarError {
    fn too_large(what: &str) -> Self {
        Self(format!("{what} is too large (overflow)"))
    }
}

type Result<T> = std::result::Result<T, VarError>;

fn read_u8<R: Read + ?Sized>(read: &mut R) -> Result<u8> {
    let mut buf = [0u8; 1];
    read.read_exact(&mut buf)
        .map_err(|e| VarError(e.to_string()))?;
    Ok(buf[0])
}

fn write_u8<W: Write + ?Sized>(write: &mut W, b: u8) -> Result<()> {
    write.write_all(&[b]).map_err(|e| VarError(e.to_string()))
}

/// A variable-length integer type used by the Minecraft network protocol.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct VarInt(pub i32);

impl VarInt {
    pub const MAX_SIZE: NonZero<usize> = NonZero::new(5).expect("5 is non-zero");

    #[must_use]
    #[inline]
    pub const fn new(value: i32) -> Self {
        Self(value)
    }

    /// Returns the exact number of bytes this `VarInt` will write.
    #[must_use]
    #[inline]
    pub const fn written_size(&self) -> usize {
        match self.0 as u32 {
            0 => 1,
            n => (31 - n.leading_zeros() as usize) / 7 + 1,
        }
    }

    #[inline]
    pub fn encode(&self, write: &mut impl Write) -> Result<()> {
        // Must cast to u32 to prevent infinite loops on negative i32s
        let mut val = self.0 as u32;

        while val > 0x7F {
            write_u8(write, (val as u8) | 0x80)?;
            val >>= 7;
        }

        write_u8(write, val as u8)
    }

    #[inline]
    pub fn decode(read: &mut impl Read) -> Result<Self> {
        let mut val = 0;
        for i in 0..Self::MAX_SIZE.get() {
            let byte = read_u8(read)?;
            // Reject encodings that set payload bits beyond bit 31 in the final byte.
            if i == Self::MAX_SIZE.get() - 1 && byte & 0x70 != 0 {
                return Err(VarError::too_large("VarInt"));
            }
            val |= (i32::from(byte) & 0x7F) << (i * 7);
            if byte & 0x80 == 0 {
                return Ok(Self(val));
            }
        }
        Err(VarError::too_large("VarInt"))
    }

    pub async fn decode_async(read: &mut (impl tokio::io::AsyncRead + Unpin)) -> Result<Self> {
        Ok(Self::decode_async_sized(read).await?.0)
    }

    /// [`Self::decode_async`], also returning how many bytes the VarInt took
    /// on the wire. The encoding need not be canonical, so the size comes from
    /// the bytes actually read, not from the value.
    pub async fn decode_async_sized(
        read: &mut (impl tokio::io::AsyncRead + Unpin),
    ) -> Result<(Self, usize)> {
        use tokio::io::AsyncReadExt;
        let mut val = 0;
        for i in 0..Self::MAX_SIZE.get() {
            let mut buf = [0u8; 1];
            let byte = match read.read_exact(&mut buf).await {
                Ok(_) => buf[0],
                Err(e) => return Err(VarError(e.to_string())),
            };
            // Reject encodings that set payload bits beyond bit 31 in the final byte.
            if i == Self::MAX_SIZE.get() - 1 && byte & 0x70 != 0 {
                return Err(VarError::too_large("VarInt"));
            }
            val |= (i32::from(byte) & 0x7F) << (i * 7);
            if byte & 0x80 == 0 {
                return Ok((Self(val), i + 1));
            }
        }
        Err(VarError::too_large("VarInt"))
    }

    pub async fn encode_async(
        &self,
        write: &mut (impl tokio::io::AsyncWrite + Unpin),
    ) -> Result<()> {
        use tokio::io::AsyncWriteExt;
        let mut val = self.0 as u32;
        while val > 0x7F {
            write
                .write_all(&[(val as u8) | 0x80])
                .await
                .map_err(|e| VarError(e.to_string()))?;
            val >>= 7;
        }
        write
            .write_all(&[val as u8])
            .await
            .map_err(|e| VarError(e.to_string()))?;
        Ok(())
    }
}

macro_rules! gen_from {
    ($ty: ty) => {
        impl From<$ty> for VarInt {
            fn from(value: $ty) -> Self {
                VarInt(value.into())
            }
        }
    };
}

gen_from!(i8);
gen_from!(u8);
gen_from!(i16);
gen_from!(u16);
gen_from!(i32);

macro_rules! gen_try_from {
    ($ty: ty) => {
        impl TryFrom<$ty> for VarInt {
            type Error = <i32 as TryFrom<$ty>>::Error;

            fn try_from(value: $ty) -> std::result::Result<Self, Self::Error> {
                Ok(VarInt(value.try_into()?))
            }
        }
    };
}

gen_try_from!(u32);
gen_try_from!(i64);
gen_try_from!(u64);
gen_try_from!(isize);
gen_try_from!(usize);

impl AsRef<i32> for VarInt {
    fn as_ref(&self) -> &i32 {
        &self.0
    }
}

impl Deref for VarInt {
    type Target = i32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A variable-length long type used by the Minecraft network protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VarLong(pub i64);

impl VarLong {
    pub const MAX_SIZE: NonZero<usize> = NonZero::new(10).expect("10 is non-zero");

    #[must_use]
    #[inline]
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    /// Returns the exact number of bytes this `VarLong` will write.
    #[must_use]
    #[inline]
    pub const fn written_size(&self) -> usize {
        match self.0 as u64 {
            0 => 1,
            n => (63 - n.leading_zeros() as usize) / 7 + 1,
        }
    }

    #[inline]
    pub fn encode(&self, write: &mut impl Write) -> Result<()> {
        let mut val = self.0 as u64;

        while val > 0x7F {
            write_u8(write, (val as u8) | 0x80)?;
            val >>= 7;
        }

        write_u8(write, val as u8)
    }

    #[inline]
    pub fn decode(read: &mut impl Read) -> Result<Self> {
        let mut val = 0;
        for i in 0..Self::MAX_SIZE.get() {
            let byte = read_u8(read)?;
            // Reject encodings that set payload bits beyond bit 63 in the final byte.
            if i == Self::MAX_SIZE.get() - 1 && byte & 0x7E != 0 {
                return Err(VarError::too_large("VarLong"));
            }
            val |= (i64::from(byte) & 0x7F) << (i * 7);
            if byte & 0x80 == 0 {
                return Ok(Self(val));
            }
        }
        Err(VarError::too_large("VarLong"))
    }
}

macro_rules! gen_from_long {
    ($ty: ty) => {
        impl From<$ty> for VarLong {
            fn from(value: $ty) -> Self {
                VarLong(value.into())
            }
        }
    };
}

gen_from_long!(u8);
gen_from_long!(u32);
gen_from_long!(i64);

impl From<usize> for VarLong {
    fn from(value: usize) -> Self {
        Self(value as i64)
    }
}

impl From<VarLong> for i64 {
    fn from(value: VarLong) -> Self {
        value.0
    }
}

impl AsRef<i64> for VarLong {
    fn as_ref(&self) -> &i64 {
        &self.0
    }
}

impl Deref for VarLong {
    type Target = i64;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A variable-length unsigned integer type used by the Minecraft network protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VarUInt(pub u32);

impl VarUInt {
    pub const MAX_SIZE: NonZero<usize> = NonZero::new(5).expect("5 is non-zero");

    #[must_use]
    #[inline]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the exact number of bytes this `VarUInt` will write.
    #[must_use]
    #[inline]
    pub const fn written_size(&self) -> usize {
        match self.0 {
            0 => 1,
            n => (31 - n.leading_zeros() as usize) / 7 + 1,
        }
    }

    #[inline]
    pub fn encode(&self, write: &mut impl Write) -> Result<()> {
        let mut val = self.0;
        loop {
            let mut byte = (val & 0x7F) as u8;
            val >>= 7;
            if val != 0 {
                byte |= 0x80;
            }
            write_u8(write, byte)?;
            if val == 0 {
                break;
            }
        }
        Ok(())
    }

    #[inline]
    pub fn decode(read: &mut impl Read) -> Result<Self> {
        let mut val = 0;
        for i in 0..Self::MAX_SIZE.get() {
            let byte = read_u8(read)?;
            // Reject encodings that set payload bits beyond bit 31 in the final byte.
            if i == Self::MAX_SIZE.get() - 1 && byte & 0x70 != 0 {
                return Err(VarError::too_large("VarUInt"));
            }
            val |= (u32::from(byte) & 0x7F) << (i * 7);
            if byte & 0x80 == 0 {
                return Ok(Self(val));
            }
        }
        Err(VarError::too_large("VarUInt"))
    }
}

macro_rules! gen_from_uint {
    ($ty: ty) => {
        impl From<$ty> for VarUInt {
            fn from(value: $ty) -> Self {
                VarUInt(value as u32)
            }
        }
    };
}

gen_from_uint!(i8);
gen_from_uint!(u8);
gen_from_uint!(i16);
gen_from_uint!(u16);
gen_from_uint!(u32);
gen_from_uint!(i32);
gen_from_uint!(i64);
gen_from_uint!(u64);
gen_from_uint!(isize);
gen_from_uint!(usize);

/// A variable-length unsigned long type used by the Minecraft network protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VarULong(pub u64);

impl VarULong {
    pub const MAX_SIZE: NonZero<usize> = NonZero::new(10).expect("10 is non-zero");

    #[must_use]
    #[inline]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the exact number of bytes this `VarULong` will write.
    #[must_use]
    #[inline]
    pub const fn written_size(&self) -> usize {
        match self.0 {
            0 => 1,
            n => (63 - n.leading_zeros() as usize) / 7 + 1,
        }
    }

    #[inline]
    pub fn encode(&self, write: &mut impl Write) -> Result<()> {
        let mut x = self.0;
        loop {
            let byte = (x & 0x7F) as u8;
            x >>= 7;
            if x == 0 {
                write_u8(write, byte)?;
                break;
            }
            write_u8(write, byte | 0x80)?;
        }

        Ok(())
    }

    #[inline]
    pub fn decode(read: &mut impl Read) -> Result<Self> {
        let mut val = 0;
        for i in 0..Self::MAX_SIZE.get() {
            let byte = read_u8(read)?;
            // Reject encodings that set payload bits beyond bit 63 in the final byte.
            if i == Self::MAX_SIZE.get() - 1 && byte & 0x7E != 0 {
                return Err(VarError::too_large("VarULong"));
            }
            val |= (u64::from(byte) & 0b0111_1111) << (i * 7);
            if byte & 0b1000_0000 == 0 {
                return Ok(Self(val));
            }
        }
        Err(VarError::too_large("VarULong"))
    }
}

macro_rules! gen_from_ulong {
    ($ty: ty) => {
        impl From<$ty> for VarULong {
            fn from(value: $ty) -> Self {
                VarULong(value.into())
            }
        }
    };
}
gen_from_ulong!(u8);
gen_from_ulong!(u16);
gen_from_ulong!(u32);
gen_from_ulong!(u64);

macro_rules! gen_try_from_ulong {
    ($ty: ty) => {
        impl TryFrom<$ty> for VarULong {
            type Error = <u64 as TryFrom<$ty>>::Error;

            fn try_from(value: $ty) -> std::result::Result<Self, Self::Error> {
                Ok(VarULong(value.try_into()?))
            }
        }
    };
}
gen_try_from_ulong!(i32);
gen_try_from_ulong!(i64);
gen_try_from_ulong!(isize);
gen_try_from_ulong!(usize);

impl From<VarULong> for u64 {
    fn from(value: VarULong) -> Self {
        value.0
    }
}

impl AsRef<u64> for VarULong {
    fn as_ref(&self) -> &u64 {
        &self.0
    }
}

impl Deref for VarULong {
    type Target = u64;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_varint_vectors() {
        // From the wiki: https://minecraft.wiki/w/Java_Edition_protocol/Data_types
        let cases: &[(i32, &[u8])] = &[
            (0, &[0x00]),
            (1, &[0x01]),
            (2, &[0x02]),
            (127, &[0x7f]),
            (128, &[0x80, 0x01]),
            (255, &[0xff, 0x01]),
            (25_565, &[0xdd, 0xc7, 0x01]),
            (2_097_151, &[0xff, 0xff, 0x7f]),
            (2_147_483_647, &[0xff, 0xff, 0xff, 0xff, 0x07]),
            (-1, &[0xff, 0xff, 0xff, 0xff, 0x0f]),
            (-2_147_483_648, &[0x80, 0x80, 0x80, 0x80, 0x08]),
        ];
        for (value, bytes) in cases {
            let mut out = Vec::new();
            VarInt(*value).encode(&mut out).unwrap();
            assert_eq!(&out, bytes, "encoding {value}");
            let mut r: &[u8] = bytes;
            assert_eq!(
                VarInt::decode(&mut r).unwrap().0,
                *value,
                "decoding {bytes:?}"
            );
            assert!(r.is_empty());
        }
    }

    #[test]
    fn java_varlong_vectors() {
        let cases: &[(i64, &[u8])] = &[
            (0, &[0x00]),
            (1, &[0x01]),
            (127, &[0x7f]),
            (128, &[0x80, 0x01]),
            (32_767, &[0xff, 0xff, 0x01]),
            (2_147_483_647, &[0xff, 0xff, 0xff, 0xff, 0x07]),
            (
                9_223_372_036_854_775_807,
                &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f],
            ),
        ];
        for (value, bytes) in cases {
            let mut out = Vec::new();
            VarLong(*value).encode(&mut out).unwrap();
            assert_eq!(&out, bytes, "encoding {value}");
            let mut r: &[u8] = bytes;
            assert_eq!(VarLong::decode(&mut r).unwrap().0, *value);
            assert!(r.is_empty());
        }
    }

    #[test]
    fn rejects_overflowing_final_byte() {
        assert!(VarInt::decode(&mut &[0x80, 0x80, 0x80, 0x80, 0x10][..]).is_err());
        assert!(VarInt::decode(&mut &[0xFF, 0xFF, 0xFF, 0xFF, 0x7F][..]).is_err());
        let mut too_big = vec![0x80; 9];
        too_big.push(0x02);
        assert!(VarLong::decode(&mut &too_big[..]).is_err());
    }

    #[test]
    fn accepts_boundary_values() {
        assert_eq!(VarInt::decode(&mut &[0x00][..]).unwrap().0, 0);
        assert_eq!(
            VarInt::decode(&mut &[0xFF, 0xFF, 0xFF, 0xFF, 0x0F][..])
                .unwrap()
                .0,
            -1
        );
        assert_eq!(
            VarInt::decode(&mut &[0xFF, 0xFF, 0xFF, 0xFF, 0x07][..])
                .unwrap()
                .0,
            i32::MAX
        );
        let mut max_ok = vec![0xFF; 9];
        max_ok.push(0x01);
        assert_eq!(VarLong::decode(&mut &max_ok[..]).unwrap().0, -1);
    }

    #[test]
    fn written_size_matches_encode() {
        for v in [0i32, 1, 127, 128, 255, 25565, 2147483647, -1, i32::MIN] {
            let vi = VarInt(v);
            let mut out = Vec::new();
            vi.encode(&mut out).unwrap();
            assert_eq!(vi.written_size(), out.len());
        }
        for v in [0i64, 127, 128, i64::MAX, -1, i64::MIN] {
            let vl = VarLong(v);
            let mut out = Vec::new();
            vl.encode(&mut out).unwrap();
            assert_eq!(vl.written_size(), out.len());
        }
    }
}
