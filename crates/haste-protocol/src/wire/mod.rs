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

pub mod component;
pub mod slot;
pub mod varint;

use bytes::Bytes;
use haste_core::{BlockPos, Component, DVec3, Identifier};
use uuid::Uuid;

use crate::nbt::{NbtCompound, NbtTag};

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
        Self { buf, src: None, tables: None }
    }

    /// Reads from a `Bytes`; byte fields are zero-copy slices of it.
    pub fn from_bytes(b: &'a Bytes) -> Self {
        Self { buf: b, src: Some(b), tables: None }
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
        if self.buf.is_empty() { Ok(()) } else { Err(DecodeError::Trailing(self.buf.len())) }
    }

    /// Consumes `len` bytes.
    pub fn take(&mut self, len: usize) -> DecodeResult<&'a [u8]> {
        if self.buf.len() < len {
            return Err(DecodeError::Eof { needed: len, remaining: self.buf.len() });
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

    /// Unsigned byte.
    pub fn u8(&mut self) -> DecodeResult<u8> { Ok(self.take(1)?[0]) }
    /// Signed byte.
    pub fn i8(&mut self) -> DecodeResult<i8> { Ok(self.take(1)?[0] as i8) }
    /// Boolean (any non-zero byte is true).
    pub fn bool(&mut self) -> DecodeResult<bool> { Ok(self.u8()? != 0) }
    /// Big-endian u16.
    pub fn u16(&mut self) -> DecodeResult<u16> { Ok(u16::from_be_bytes(self.array()?)) }
    /// Big-endian i16.
    pub fn i16(&mut self) -> DecodeResult<i16> { Ok(i16::from_be_bytes(self.array()?)) }
    /// Big-endian i32 (`Int`).
    pub fn i32(&mut self) -> DecodeResult<i32> { Ok(i32::from_be_bytes(self.array()?)) }
    /// Big-endian u32.
    pub fn u32(&mut self) -> DecodeResult<u32> { Ok(u32::from_be_bytes(self.array()?)) }
    /// Big-endian i64 (`Long`).
    pub fn i64(&mut self) -> DecodeResult<i64> { Ok(i64::from_be_bytes(self.array()?)) }
    /// Big-endian u64.
    pub fn u64(&mut self) -> DecodeResult<u64> { Ok(u64::from_be_bytes(self.array()?)) }
    /// IEEE float.
    pub fn f32(&mut self) -> DecodeResult<f32> { Ok(f32::from_be_bytes(self.array()?)) }
    /// IEEE double.
    pub fn f64(&mut self) -> DecodeResult<f64> { Ok(f64::from_be_bytes(self.array()?)) }

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

    /// String with a limit of `max` UTF-16 code units (vanilla counts Java chars).
    pub fn string_max(&mut self, max: usize) -> DecodeResult<String> {
        let len = self.len_prefix(max.saturating_mul(3))?;
        let bytes = self.take(len)?;
        let s = std::str::from_utf8(bytes).map_err(|_| DecodeError::Invalid("string is not UTF-8".into()))?;
        if s.len() > max && s.encode_utf16().nth(max).is_some() {
            return Err(DecodeError::TooLarge(format!("string longer than {max} chars")));
        }
        Ok(s.to_owned())
    }
    /// String (32767).
    pub fn string(&mut self) -> DecodeResult<String> { self.string_max(MAX_STRING) }

    /// Identifier (`ns:path`, namespace defaults to `minecraft`).
    pub fn identifier(&mut self) -> DecodeResult<Identifier> {
        let s = self.string()?;
        Identifier::parse(&s).map_err(|e| DecodeError::Invalid(e.to_string()))
    }

    /// UUID (two big-endian longs).
    pub fn uuid(&mut self) -> DecodeResult<Uuid> { Ok(Uuid::from_bytes(self.array()?)) }

    /// Position (packed long).
    pub fn block_pos(&mut self) -> DecodeResult<BlockPos> { Ok(BlockPos::from_packed(self.i64()?)) }

    /// Angle: 1/256 of a turn, returned in degrees in `[0, 360)`.
    pub fn angle(&mut self) -> DecodeResult<f32> { Ok(self.u8()? as f32 * (360.0 / 256.0)) }

    /// Prefixed array of bytes, at most `max` long.
    pub fn byte_array(&mut self, max: usize) -> DecodeResult<Bytes> {
        let n = self.len_prefix(max)?;
        let s = self.take(n)?;
        Ok(self.to_bytes(s))
    }

    /// Prefixed Optional.
    pub fn option<T>(&mut self, f: impl FnOnce(&mut Self) -> DecodeResult<T>) -> DecodeResult<Option<T>> {
        if self.bool()? { Ok(Some(f(self)?)) } else { Ok(None) }
    }

    /// Prefixed Array with at most `max` elements.
    pub fn list<T>(&mut self, max: usize, mut f: impl FnMut(&mut Self) -> DecodeResult<T>) -> DecodeResult<Vec<T>> {
        let n = self.len_prefix(max)?;
        // Never trust the count for the allocation.
        let mut v = Vec::with_capacity(n.min(self.remaining()));
        for _ in 0..n {
            v.push(f(self)?);
        }
        Ok(v)
    }

    /// BitSet: prefixed array of longs.
    pub fn bit_set(&mut self) -> DecodeResult<Vec<u64>> {
        let n = self.len_prefix(MAX_LIST)?;
        let mut v = Vec::with_capacity(n.min(self.remaining() / 8));
        for _ in 0..n {
            v.push(self.u64()?);
        }
        Ok(v)
    }

    /// Fixed BitSet of `bits` bits (`ceil(bits/8)` bytes).
    pub fn fixed_bit_set(&mut self, bits: usize) -> DecodeResult<Vec<u8>> {
        Ok(self.take(bits.div_ceil(8))?.to_vec())
    }

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
            return Err(DecodeError::Eof { needed: used, remaining: all.len() });
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
            Some(other) => Err(DecodeError::Invalid(format!("expected NBT compound, got tag id {}", other.get_type_id()))),
        }
    }

    /// Text Component (network NBT form).
    pub fn component(&mut self) -> DecodeResult<Component> {
        match self.nbt_tag()? {
            None => Ok(Component::default()),
            Some(tag) => component::from_nbt(&tag),
        }
    }

    /// JSON Text Component (a string holding JSON).
    pub fn json_component(&mut self) -> DecodeResult<Component> {
        let s = self.string_max(MAX_JSON_COMPONENT)?;
        component::from_json_str(&s)
    }

    /// LpVec3: a low-precision vector (velocities), see the Data types page.
    pub fn lp_vec3(&mut self) -> DecodeResult<DVec3> {
        let b1 = self.u8()? as u64;
        if b1 == 0 {
            return Ok(DVec3::ZERO);
        }
        let b2 = self.u8()? as u64;
        let rest = self.u32()? as u64;
        let packed = (rest << 16) | (b2 << 8) | b1;
        let mut scale = b1 & 3;
        if b1 & 4 != 0 {
            scale |= (self.var_int()? as u32 as u64) << 2;
        }
        let scale = scale as f64;
        fn unpack(v: u64) -> f64 {
            ((v & 32767) as f64).min(32766.0) * 2.0 / 32766.0 - 1.0
        }
        Ok(DVec3::new(unpack(packed >> 3) * scale, unpack(packed >> 18) * scale, unpack(packed >> 33) * scale))
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
    pub fn new() -> Self { Self::default() }
    /// The bytes written.
    pub fn into_inner(self) -> Vec<u8> { self.buf }
    /// Raw bytes.
    pub fn bytes(&mut self, b: &[u8]) { self.buf.extend_from_slice(b); }
    /// Unsigned byte.
    pub fn u8(&mut self, v: u8) { self.buf.push(v); }
    /// Signed byte.
    pub fn i8(&mut self, v: i8) { self.buf.push(v as u8); }
    /// Boolean.
    pub fn bool(&mut self, v: bool) { self.buf.push(v as u8); }
    /// Big-endian u16.
    pub fn u16(&mut self, v: u16) { self.bytes(&v.to_be_bytes()); }
    /// Big-endian i16.
    pub fn i16(&mut self, v: i16) { self.bytes(&v.to_be_bytes()); }
    /// Big-endian i32.
    pub fn i32(&mut self, v: i32) { self.bytes(&v.to_be_bytes()); }
    /// Big-endian u32.
    pub fn u32(&mut self, v: u32) { self.bytes(&v.to_be_bytes()); }
    /// Big-endian i64.
    pub fn i64(&mut self, v: i64) { self.bytes(&v.to_be_bytes()); }
    /// Big-endian u64.
    pub fn u64(&mut self, v: u64) { self.bytes(&v.to_be_bytes()); }
    /// Float.
    pub fn f32(&mut self, v: f32) { self.bytes(&v.to_be_bytes()); }
    /// Double.
    pub fn f64(&mut self, v: f64) { self.bytes(&v.to_be_bytes()); }
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
    pub fn len_prefix(&mut self, n: usize) { self.var_int(n as i32); }
    /// String.
    pub fn string(&mut self, s: &str) {
        self.len_prefix(s.len());
        self.bytes(s.as_bytes());
    }
    /// Identifier.
    pub fn identifier(&mut self, id: &Identifier) { self.string(&id.to_string()); }
    /// UUID.
    pub fn uuid(&mut self, u: &Uuid) { self.bytes(u.as_bytes()); }
    /// Position.
    pub fn block_pos(&mut self, p: &BlockPos) { self.i64(p.to_packed()); }
    /// Angle from degrees (wraps).
    pub fn angle(&mut self, deg: f32) { self.u8((deg * 256.0 / 360.0).floor() as i32 as u8); }
    /// Prefixed byte array.
    pub fn byte_array(&mut self, b: &[u8]) {
        self.len_prefix(b.len());
        self.bytes(b);
    }
    /// Prefixed Optional.
    pub fn option<T>(&mut self, v: &Option<T>, f: impl FnOnce(&mut Self, &T)) {
        match v {
            Some(x) => {
                self.bool(true);
                f(self, x);
            }
            None => self.bool(false),
        }
    }
    /// Prefixed Array.
    pub fn list<T>(&mut self, v: &[T], mut f: impl FnMut(&mut Self, &T)) {
        self.len_prefix(v.len());
        for x in v {
            f(self, x);
        }
    }
    /// BitSet.
    pub fn bit_set(&mut self, longs: &[u64]) {
        self.len_prefix(longs.len());
        for l in longs {
            self.u64(*l);
        }
    }
    /// One network NBT value (`None` writes `TAG_End`).
    pub fn nbt_tag(&mut self, tag: Option<&NbtTag>) {
        match tag {
            None | Some(NbtTag::End) => self.u8(crate::nbt::END_ID),
            Some(t) => {
                let mut w = crate::nbt::serializer::NbtWriteHelperJava::new(&mut self.buf);
                // Writing into a Vec only fails for strings over 65535 bytes,
                // which we never build; treat it as a programming error.
                t.clone().serialize(&mut w).expect("NBT serialisation into a Vec");
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
    /// Text Component in network NBT form.
    pub fn component(&mut self, c: &Component) {
        let tag = component::to_nbt(c);
        self.nbt_tag(Some(&tag));
    }
    /// JSON Text Component.
    pub fn json_component(&mut self, c: &Component) {
        self.string(&component::to_json_string(c));
    }
    /// LpVec3.
    pub fn lp_vec3(&mut self, v: DVec3) {
        let max = v.x.abs().max(v.y.abs()).max(v.z.abs());
        if max.is_nan() || max < 1.0 / 32766.0 {
            self.u8(0);
            return;
        }
        let scale = max.ceil() as u64;
        let cont = (scale & 3) != scale;
        let packed_scale = if cont { (scale & 3) | 4 } else { scale };
        fn pack(v: f64) -> u64 {
            ((v * 0.5 + 0.5) * 32766.0).round() as u64
        }
        let s = scale as f64;
        let packed = (pack(v.z / s) << 33) | (pack(v.y / s) << 18) | (pack(v.x / s) << 3) | packed_scale;
        self.u8(packed as u8);
        self.u8((packed >> 8) as u8);
        self.u32((packed >> 16) as u32);
        if cont {
            self.var_int((scale >> 2) as i32);
        }
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

/// A type with one canonical wire encoding.
pub trait Wire: Sized {
    /// Reads the value.
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self>;
    /// Writes the value.
    fn write(&self, w: &mut Writer);
}

/// An encoding of `T` chosen explicitly. Used by the packet macro:
/// `field: i32 as VarInt`.
pub trait Codec<T> {
    /// Reads a `T` in this encoding.
    fn read(r: &mut Reader<'_>) -> DecodeResult<T>;
    /// Writes a `T` in this encoding.
    fn write(v: &T, w: &mut Writer);
}

/// The canonical encoding ([`Wire`]).
pub struct Native;
impl<T: Wire> Codec<T> for Native {
    fn read(r: &mut Reader<'_>) -> DecodeResult<T> { T::read(r) }
    fn write(v: &T, w: &mut Writer) { v.write(w) }
}

macro_rules! prim {
    ($t:ty, $r:ident, $w:ident) => {
        impl Wire for $t {
            fn read(r: &mut Reader<'_>) -> DecodeResult<Self> { r.$r() }
            fn write(&self, w: &mut Writer) { w.$w(*self) }
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

impl Wire for String {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> { r.string() }
    fn write(&self, w: &mut Writer) { w.string(self) }
}
impl Wire for Identifier {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> { r.identifier() }
    fn write(&self, w: &mut Writer) { w.identifier(self) }
}
impl Wire for Uuid {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> { r.uuid() }
    fn write(&self, w: &mut Writer) { w.uuid(self) }
}
impl Wire for BlockPos {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> { r.block_pos() }
    fn write(&self, w: &mut Writer) { w.block_pos(self) }
}
impl Wire for Component {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> { r.component() }
    fn write(&self, w: &mut Writer) { w.component(self) }
}
/// Three doubles.
impl Wire for DVec3 {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> { Ok(DVec3::new(r.f64()?, r.f64()?, r.f64()?)) }
    fn write(&self, w: &mut Writer) {
        w.f64(self.x);
        w.f64(self.y);
        w.f64(self.z);
    }
}
/// Network NBT; `TAG_End` decodes to `NbtTag::End`.
impl Wire for NbtTag {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> { Ok(r.nbt_tag()?.unwrap_or(NbtTag::End)) }
    fn write(&self, w: &mut Writer) { w.nbt_tag(Some(self)) }
}
/// Prefixed Optional.
impl<T: Wire> Wire for Option<T> {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> { r.option(T::read) }
    fn write(&self, w: &mut Writer) { w.option(self, |w, v| v.write(w)) }
}
/// Prefixed Array.
impl<T: Wire> Wire for Vec<T> {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> { r.list(MAX_LIST, T::read) }
    fn write(&self, w: &mut Writer) { w.list(self, |w, v| v.write(w)) }
}

/// VarInt encoding of an `i32` (or `u32`).
pub struct VarInt;
impl Codec<i32> for VarInt {
    fn read(r: &mut Reader<'_>) -> DecodeResult<i32> { r.var_int() }
    fn write(v: &i32, w: &mut Writer) { w.var_int(*v) }
}
impl Codec<u32> for VarInt {
    fn read(r: &mut Reader<'_>) -> DecodeResult<u32> { Ok(r.var_int()? as u32) }
    fn write(v: &u32, w: &mut Writer) { w.var_int(*v as i32) }
}
/// VarLong encoding of an `i64`.
pub struct VarLong;
impl Codec<i64> for VarLong {
    fn read(r: &mut Reader<'_>) -> DecodeResult<i64> { r.var_long() }
    fn write(v: &i64, w: &mut Writer) { w.var_long(*v) }
}
/// Angle byte encoding of degrees.
pub struct Angle;
impl Codec<f32> for Angle {
    fn read(r: &mut Reader<'_>) -> DecodeResult<f32> { r.angle() }
    fn write(v: &f32, w: &mut Writer) { w.angle(*v) }
}
/// LpVec3 encoding of a vector.
pub struct LpVec3;
impl Codec<DVec3> for LpVec3 {
    fn read(r: &mut Reader<'_>) -> DecodeResult<DVec3> { r.lp_vec3() }
    fn write(v: &DVec3, w: &mut Writer) { w.lp_vec3(*v) }
}
/// JSON string encoding of a text component.
pub struct Json;
impl Codec<Component> for Json {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Component> { r.json_component() }
    fn write(v: &Component, w: &mut Writer) { w.json_component(v) }
}
/// Everything up to the end of the packet, raw.
pub struct Rest;
impl Codec<Bytes> for Rest {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Bytes> { Ok(r.rest()) }
    fn write(v: &Bytes, w: &mut Writer) { w.bytes(v) }
}
/// Prefixed array of bytes.
pub struct ByteArray;
impl Codec<Bytes> for ByteArray {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Bytes> { r.byte_array(crate::transport::MAX_PACKET_DATA_SIZE) }
    fn write(v: &Bytes, w: &mut Writer) { w.byte_array(v) }
}
/// One network NBT value kept as raw bytes.
pub struct RawNbt;
impl Codec<Bytes> for RawNbt {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Bytes> { r.nbt_raw() }
    fn write(v: &Bytes, w: &mut Writer) { w.bytes(v) }
}
/// BitSet (prefixed longs).
pub struct BitSet;
impl Codec<Vec<u64>> for BitSet {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Vec<u64>> { r.bit_set() }
    fn write(v: &Vec<u64>, w: &mut Writer) { w.bit_set(v) }
}
/// Prefixed Array with an element codec.
pub struct List<C>(std::marker::PhantomData<C>);
impl<T, C: Codec<T>> Codec<Vec<T>> for List<C> {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Vec<T>> { r.list(MAX_LIST, C::read) }
    fn write(v: &Vec<T>, w: &mut Writer) { w.list(v, |w, x| C::write(x, w)) }
}
/// Prefixed Optional with an element codec.
pub struct Opt<C>(std::marker::PhantomData<C>);
impl<T, C: Codec<T>> Codec<Option<T>> for Opt<C> {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Option<T>> { r.option(C::read) }
    fn write(v: &Option<T>, w: &mut Writer) { w.option(v, |w, x| C::write(x, w)) }
}

/// `ID or X`: a registry reference (`Id(n)`, sent as n+1) or an inline value.
#[derive(Clone, Debug, PartialEq)]
pub enum IdOr<T> {
    /// Registry id.
    Id(i32),
    /// Inline definition.
    Inline(T),
}
impl<T: Wire> Wire for IdOr<T> {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let id = r.var_int()?;
        if id == 0 { Ok(IdOr::Inline(T::read(r)?)) } else { Ok(IdOr::Id(id - 1)) }
    }
    fn write(&self, w: &mut Writer) {
        match self {
            IdOr::Id(i) => w.var_int(i + 1),
            IdOr::Inline(v) => {
                w.var_int(0);
                v.write(w);
            }
        }
    }
}

/// `ID Set`: a tag reference or an inline list of registry ids.
#[derive(Clone, Debug, PartialEq)]
pub enum IdSet {
    /// A named tag.
    Tag(Identifier),
    /// Inline ids.
    Ids(Vec<i32>),
}
impl Wire for IdSet {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let t = r.var_int()?;
        if t == 0 {
            return Ok(IdSet::Tag(r.identifier()?));
        }
        if t < 0 || t as usize > MAX_LIST {
            return Err(DecodeError::Invalid(format!("ID set size {t}")));
        }
        let mut v = Vec::with_capacity(((t - 1) as usize).min(r.remaining()));
        for _ in 0..t - 1 {
            v.push(r.var_int()?);
        }
        Ok(IdSet::Ids(v))
    }
    fn write(&self, w: &mut Writer) {
        match self {
            IdSet::Tag(t) => {
                w.var_int(0);
                w.identifier(t);
            }
            IdSet::Ids(v) => {
                w.var_int(v.len() as i32 + 1);
                for i in v {
                    w.var_int(*i);
                }
            }
        }
    }
}

/// Inline `Sound Event` definition.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundEvent {
    /// Sound name.
    pub name: Identifier,
    /// Fixed range, if the sound has one.
    pub fixed_range: Option<f32>,
}
impl Wire for SoundEvent {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> { Ok(Self { name: r.identifier()?, fixed_range: r.read()? }) }
    fn write(&self, w: &mut Writer) {
        w.identifier(&self.name);
        w.write(&self.fixed_range);
    }
}

/// A profile property (skin textures and the like).
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileProperty {
    /// Property name (`textures`).
    pub name: String,
    /// Base64 value.
    pub value: String,
    /// Yggdrasil signature, if any.
    pub signature: Option<String>,
}
impl Wire for ProfileProperty {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self { name: r.string_max(64)?, value: r.string()?, signature: r.option(|r| r.string_max(1024))? })
    }
    fn write(&self, w: &mut Writer) {
        w.string(&self.name);
        w.string(&self.value);
        w.option(&self.signature, |w, s| w.string(s));
    }
}

/// `Game Profile`.
#[derive(Clone, Debug, PartialEq)]
pub struct GameProfile {
    /// Player UUID.
    pub uuid: Uuid,
    /// Username.
    pub name: String,
    /// Properties.
    pub properties: Vec<ProfileProperty>,
}
impl Wire for GameProfile {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self { uuid: r.uuid()?, name: r.string_max(16)?, properties: r.list(16, ProfileProperty::read)? })
    }
    fn write(&self, w: &mut Writer) {
        w.uuid(&self.uuid);
        w.string(&self.name);
        w.write(&self.properties);
    }
}

/// `Resolvable Profile` (player heads, mannequins).
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvableProfile {
    /// Partial (name/uuid optional) or complete profile.
    pub profile: ResolvableProfileKind,
    /// Skin texture override.
    pub body: Option<Identifier>,
    /// Cape override.
    pub cape: Option<Identifier>,
    /// Elytra override.
    pub elytra: Option<Identifier>,
    /// Model: 0 wide, 1 slim.
    pub model: Option<i32>,
}
/// The two profile forms of [`ResolvableProfile`].
#[derive(Clone, Debug, PartialEq)]
pub enum ResolvableProfileKind {
    /// Kind 0.
    Partial {
        /// Name, if known.
        name: Option<String>,
        /// UUID, if known.
        uuid: Option<Uuid>,
        /// Properties.
        properties: Vec<ProfileProperty>,
    },
    /// Kind 1.
    Complete(GameProfile),
}
impl Wire for ResolvableProfile {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let profile = match r.var_int()? {
            0 => ResolvableProfileKind::Partial {
                name: r.option(|r| r.string_max(16))?,
                uuid: r.read()?,
                properties: r.list(16, ProfileProperty::read)?,
            },
            1 => ResolvableProfileKind::Complete(r.read()?),
            k => return Err(DecodeError::Invalid(format!("resolvable profile kind {k}"))),
        };
        Ok(Self { profile, body: r.read()?, cape: r.read()?, elytra: r.read()?, model: r.option(|r| r.var_int())? })
    }
    fn write(&self, w: &mut Writer) {
        match &self.profile {
            ResolvableProfileKind::Partial { name, uuid, properties } => {
                w.var_int(0);
                w.write(name);
                w.write(uuid);
                w.write(properties);
            }
            ResolvableProfileKind::Complete(p) => {
                w.var_int(1);
                w.write(p);
            }
        }
        w.write(&self.body);
        w.write(&self.cape);
        w.write(&self.elytra);
        w.option(&self.model, |w, m| w.var_int(*m));
    }
}

/// Light data as in `level_chunk_with_light` and `light_update`.
impl Wire for haste_core::chunk::LightData {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let sky_mask = r.bit_set()?;
        let block_mask = r.bit_set()?;
        let empty_sky_mask = r.bit_set()?;
        let empty_block_mask = r.bit_set()?;
        let sky_arrays = r.list(4096, |r| r.byte_array(2048))?;
        let block_arrays = r.list(4096, |r| r.byte_array(2048))?;
        Ok(Self { sky_mask, block_mask, empty_sky_mask, empty_block_mask, sky_arrays, block_arrays })
    }
    fn write(&self, w: &mut Writer) {
        w.bit_set(&self.sky_mask);
        w.bit_set(&self.block_mask);
        w.bit_set(&self.empty_sky_mask);
        w.bit_set(&self.empty_block_mask);
        w.list(&self.sky_arrays, |w, a| w.byte_array(a));
        w.list(&self.block_arrays, |w, a| w.byte_array(a));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn varint_bytes(v: i32) -> Vec<u8> {
        let mut w = Writer::new();
        w.var_int(v);
        w.into_inner()
    }

    #[test]
    fn varint_wiki_vectors() {
        // minecraft.wiki "VarInt and VarLong" sample table.
        let cases: &[(i32, &[u8])] = &[
            (0, &[0x00]),
            (1, &[0x01]),
            (2, &[0x02]),
            (127, &[0x7f]),
            (128, &[0x80, 0x01]),
            (255, &[0xff, 0x01]),
            (25565, &[0xdd, 0xc7, 0x01]),
            (2097151, &[0xff, 0xff, 0x7f]),
            (2147483647, &[0xff, 0xff, 0xff, 0xff, 0x07]),
            (-1, &[0xff, 0xff, 0xff, 0xff, 0x0f]),
            (-2147483648, &[0x80, 0x80, 0x80, 0x80, 0x08]),
        ];
        for (v, bytes) in cases {
            assert_eq!(varint_bytes(*v), *bytes, "encode {v}");
            assert_eq!(Reader::new(bytes).var_int().unwrap(), *v, "decode {v}");
        }
        assert!(Reader::new(&[0xff, 0xff, 0xff, 0xff, 0xff, 0x01]).var_int().is_err());
        assert!(Reader::new(&[0x80]).var_int().is_err());
    }

    #[test]
    fn varlong_wiki_vectors() {
        let cases: &[(i64, &[u8])] = &[
            (0, &[0x00]),
            (127, &[0x7f]),
            (128, &[0x80, 0x01]),
            (2147483647, &[0xff, 0xff, 0xff, 0xff, 0x07]),
            (9223372036854775807, &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f]),
            (-1, &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]),
            (-2147483648, &[0x80, 0x80, 0x80, 0x80, 0xf8, 0xff, 0xff, 0xff, 0xff, 0x01]),
            (-9223372036854775808, &[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01]),
        ];
        for (v, bytes) in cases {
            let mut w = Writer::new();
            w.var_long(*v);
            assert_eq!(w.buf, *bytes, "encode {v}");
            assert_eq!(Reader::new(bytes).var_long().unwrap(), *v, "decode {v}");
        }
    }

    #[test]
    fn position_wiki_example() {
        // Data types page: x=18357644, z=-20882616, y=831.
        let p = BlockPos::new(18357644, 831, -20882616);
        let mut w = Writer::new();
        w.block_pos(&p);
        let v = i64::from_be_bytes(w.buf.clone().try_into().unwrap());
        assert_eq!(v as u64, 0b0100011000000111011000110010110000010101101101001000001100111111);
        assert_eq!(Reader::new(&w.buf).block_pos().unwrap(), p);
    }

    #[test]
    fn lp_vec3_wiki_samples() {
        type Case = ((f64, f64, f64), &'static [u8]);
        let cases: &[Case] = &[
            ((0.0, 0.0, 0.0), &[0]),
            ((1.0, 0.0, -1.0), &[0xF1, 0xFF, 0x00, 0x00, 0xFF, 0xFF]),
            ((10.0, 0.2, -5.0), &[0xF6, 0xFF, 0x40, 0x01, 0x05, 0x1F, 0x02]),
            ((123457.0, 15.071, 0.0), &[0xF5, 0xFF, 0x7F, 0xFF, 0x00, 0x07, 0x90, 0xF1, 0x01]),
        ];
        for ((x, y, z), bytes) in cases {
            let mut w = Writer::new();
            w.lp_vec3(DVec3::new(*x, *y, *z));
            assert_eq!(w.buf, *bytes, "encode {x},{y},{z}");
            let v = Reader::new(bytes).lp_vec3().unwrap();
            let tol = (x.abs().max(y.abs()).max(z.abs())).ceil() / 16000.0 + 1e-9;
            assert!((v.x - x).abs() <= tol && (v.y - y).abs() <= tol && (v.z - z).abs() <= tol, "{v:?}");
        }
    }

    #[test]
    fn strings_and_limits() {
        let mut w = Writer::new();
        w.string("héllo");
        let mut r = Reader::new(&w.buf);
        assert_eq!(r.string().unwrap(), "héllo");
        r.finish().unwrap();
        let mut w = Writer::new();
        w.string("abcdef");
        assert!(Reader::new(&w.buf).string_max(5).is_err());
        assert!(Reader::new(&[5, b'a']).string().is_err());
    }

    #[test]
    fn options_lists_bitsets_uuid_angle() {
        let mut w = Writer::new();
        w.write(&Some(5i32));
        w.write(&None::<i32>);
        w.write(&vec![1i16, 2, 3]);
        w.bit_set(&[1, u64::MAX]);
        let u = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
        w.uuid(&u);
        w.angle(90.0);
        let mut r = Reader::new(&w.buf);
        assert_eq!(r.read::<Option<i32>>().unwrap(), Some(5));
        assert_eq!(r.read::<Option<i32>>().unwrap(), None);
        assert_eq!(r.read::<Vec<i16>>().unwrap(), vec![1, 2, 3]);
        assert_eq!(r.bit_set().unwrap(), vec![1, u64::MAX]);
        assert_eq!(r.uuid().unwrap(), u);
        assert_eq!(r.angle().unwrap(), 90.0);
        r.finish().unwrap();
    }

    #[test]
    fn nbt_raw_and_parsed() {
        let mut c = NbtCompound::new();
        c.put_int("a", 7);
        c.put_string("s", "x".into());
        let mut w = Writer::new();
        w.nbt_compound(Some(&c));
        w.nbt_compound(None);
        w.u8(42);
        let b = Bytes::from(w.into_inner());
        let mut r = Reader::from_bytes(&b);
        let raw = r.nbt_raw().unwrap();
        assert_eq!(raw[0], crate::nbt::COMPOUND_ID);
        assert_eq!(r.nbt_compound().unwrap(), None);
        assert_eq!(r.u8().unwrap(), 42);
        let parsed = Reader::new(&raw).nbt_compound().unwrap().unwrap();
        assert_eq!(parsed, c);
        // Truncated NBT is an error, not a silent over-read.
        assert!(Reader::new(&raw[..raw.len() - 1]).nbt_raw().is_err());
    }

    #[test]
    fn id_or_and_id_set() {
        let mut w = Writer::new();
        w.write(&IdOr::<SoundEvent>::Id(4));
        w.write(&IdOr::Inline(SoundEvent { name: Identifier::minecraft("a"), fixed_range: Some(2.0) }));
        w.write(&IdSet::Tag(Identifier::minecraft("logs")));
        w.write(&IdSet::Ids(vec![1, 2]));
        let mut r = Reader::new(&w.buf);
        assert_eq!(r.read::<IdOr<SoundEvent>>().unwrap(), IdOr::Id(4));
        assert!(matches!(r.read::<IdOr<SoundEvent>>().unwrap(), IdOr::Inline(_)));
        assert_eq!(r.read::<IdSet>().unwrap(), IdSet::Tag(Identifier::minecraft("logs")));
        assert_eq!(r.read::<IdSet>().unwrap(), IdSet::Ids(vec![1, 2]));
        r.finish().unwrap();
    }
}
