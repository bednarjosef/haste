//! Fixed-size primitives and varints (minecraft.wiki "Data types": Bool,
//! Byte, Short, Int, Long, Float, Double, VarInt, VarLong), plus UUID as two
//! big-endian longs.

use uuid::Uuid;

use super::varint;
use super::{DecodeError, DecodeResult, Reader, Wire, Writer};

impl Reader<'_> {
    /// Unsigned byte.
    pub fn u8(&mut self) -> DecodeResult<u8> {
        Ok(self.take(1)?[0])
    }
    /// Signed byte.
    pub fn i8(&mut self) -> DecodeResult<i8> {
        Ok(self.take(1)?[0] as i8)
    }
    /// Boolean. Strict: bytes other than 0 and 1 are an error. Vanilla
    /// accepts any non-zero byte, but servers only send 0/1, so a stray value
    /// almost always means we misread the layout.
    pub fn bool(&mut self) -> DecodeResult<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            b => Err(DecodeError::Invalid(format!("boolean byte {b}"))),
        }
    }
    /// Big-endian u16.
    pub fn u16(&mut self) -> DecodeResult<u16> {
        Ok(u16::from_be_bytes(self.array()?))
    }
    /// Big-endian i16.
    pub fn i16(&mut self) -> DecodeResult<i16> {
        Ok(i16::from_be_bytes(self.array()?))
    }
    /// Big-endian i32 (`Int`).
    pub fn i32(&mut self) -> DecodeResult<i32> {
        Ok(i32::from_be_bytes(self.array()?))
    }
    /// Big-endian u32.
    pub fn u32(&mut self) -> DecodeResult<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    /// Big-endian i64 (`Long`).
    pub fn i64(&mut self) -> DecodeResult<i64> {
        Ok(i64::from_be_bytes(self.array()?))
    }
    /// Big-endian u64.
    pub fn u64(&mut self) -> DecodeResult<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    /// IEEE float.
    pub fn f32(&mut self) -> DecodeResult<f32> {
        Ok(f32::from_be_bytes(self.array()?))
    }
    /// IEEE double.
    pub fn f64(&mut self) -> DecodeResult<f64> {
        Ok(f64::from_be_bytes(self.array()?))
    }

    /// VarInt.
    pub fn var_int(&mut self) -> DecodeResult<i32> {
        Ok(varint::VarInt::decode(&mut self.buf)?.0)
    }
    /// VarLong.
    pub fn var_long(&mut self) -> DecodeResult<i64> {
        Ok(varint::VarLong::decode(&mut self.buf)?.0)
    }
    /// A VarInt used as a length or count; rejects negatives and values over `max`.
    pub fn len_prefix(&mut self, max: usize) -> DecodeResult<usize> {
        let n = self.var_int()?;
        if n < 0 {
            return Err(DecodeError::Invalid(format!("negative length {n}")));
        }
        let n = n as usize;
        if n > max {
            return Err(DecodeError::TooLarge(format!("length {n} > {max}")));
        }
        Ok(n)
    }

    /// UUID (two big-endian longs).
    pub fn uuid(&mut self) -> DecodeResult<Uuid> {
        Ok(Uuid::from_bytes(self.array()?))
    }
}

impl Writer {
    /// Raw bytes.
    pub fn bytes(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
    /// Unsigned byte.
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    /// Signed byte.
    pub fn i8(&mut self, v: i8) {
        self.buf.push(v as u8);
    }
    /// Boolean.
    pub fn bool(&mut self, v: bool) {
        self.buf.push(v as u8);
    }
    /// Big-endian u16.
    pub fn u16(&mut self, v: u16) {
        self.bytes(&v.to_be_bytes());
    }
    /// Big-endian i16.
    pub fn i16(&mut self, v: i16) {
        self.bytes(&v.to_be_bytes());
    }
    /// Big-endian i32.
    pub fn i32(&mut self, v: i32) {
        self.bytes(&v.to_be_bytes());
    }
    /// Big-endian u32.
    pub fn u32(&mut self, v: u32) {
        self.bytes(&v.to_be_bytes());
    }
    /// Big-endian i64.
    pub fn i64(&mut self, v: i64) {
        self.bytes(&v.to_be_bytes());
    }
    /// Big-endian u64.
    pub fn u64(&mut self, v: u64) {
        self.bytes(&v.to_be_bytes());
    }
    /// Float.
    pub fn f32(&mut self, v: f32) {
        self.bytes(&v.to_be_bytes());
    }
    /// Double.
    pub fn f64(&mut self, v: f64) {
        self.bytes(&v.to_be_bytes());
    }
    /// VarInt.
    pub fn var_int(&mut self, v: i32) {
        let mut v = v as u32;
        while v > 0x7F {
            self.buf.push((v as u8) | 0x80);
            v >>= 7;
        }
        self.buf.push(v as u8);
    }
    /// VarLong.
    pub fn var_long(&mut self, v: i64) {
        let mut v = v as u64;
        while v > 0x7F {
            self.buf.push((v as u8) | 0x80);
            v >>= 7;
        }
        self.buf.push(v as u8);
    }
    /// A length as VarInt.
    pub fn len_prefix(&mut self, n: usize) {
        self.var_int(n as i32);
    }
    /// UUID.
    pub fn uuid(&mut self, u: &Uuid) {
        self.bytes(u.as_bytes());
    }
}

macro_rules! prim {
    ($t:ty, $r:ident, $w:ident) => {
        impl Wire for $t {
            fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
                r.$r()
            }
            fn write(&self, w: &mut Writer) {
                w.$w(*self)
            }
        }
    };
}
prim!(u8, u8, u8);
prim!(i8, i8, i8);
prim!(bool, bool, bool);
prim!(u16, u16, u16);
prim!(i16, i16, i16);
prim!(i32, i32, i32);
prim!(i64, i64, i64);
prim!(f32, f32, f32);
prim!(f64, f64, f64);

impl Wire for Uuid {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        r.uuid()
    }
    fn write(&self, w: &mut Writer) {
        w.uuid(self)
    }
}
