//! The [`Wire`] and [`Codec`] traits, the codec marker types that select an
//! encoding (`VarInt`, `Angle`, `Opt`, ...) and the composite wire types
//! vanilla defines as StreamCodecs (ID or inline, ID Set, profiles, the
//! light data record).

use bytes::Bytes;
use uuid::Uuid;

use haste_core::{Component, DVec3, Identifier};

use super::{DecodeError, DecodeResult, Reader, Writer, MAX_LIST};

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
    fn read(r: &mut Reader<'_>) -> DecodeResult<T> {
        T::read(r)
    }
    fn write(v: &T, w: &mut Writer) {
        v.write(w)
    }
}

/// VarInt encoding of an `i32` (or `u32`).
pub struct VarInt;
impl Codec<i32> for VarInt {
    fn read(r: &mut Reader<'_>) -> DecodeResult<i32> {
        r.var_int()
    }
    fn write(v: &i32, w: &mut Writer) {
        w.var_int(*v)
    }
}
impl Codec<u32> for VarInt {
    fn read(r: &mut Reader<'_>) -> DecodeResult<u32> {
        Ok(r.var_int()? as u32)
    }
    fn write(v: &u32, w: &mut Writer) {
        w.var_int(*v as i32)
    }
}
/// VarLong encoding of an `i64`.
pub struct VarLong;
impl Codec<i64> for VarLong {
    fn read(r: &mut Reader<'_>) -> DecodeResult<i64> {
        r.var_long()
    }
    fn write(v: &i64, w: &mut Writer) {
        w.var_long(*v)
    }
}
/// Angle byte encoding of degrees.
pub struct Angle;
impl Codec<f32> for Angle {
    fn read(r: &mut Reader<'_>) -> DecodeResult<f32> {
        r.angle()
    }
    fn write(v: &f32, w: &mut Writer) {
        w.angle(*v)
    }
}
/// LpVec3 encoding of a vector.
pub struct LpVec3;
impl Codec<DVec3> for LpVec3 {
    fn read(r: &mut Reader<'_>) -> DecodeResult<DVec3> {
        r.lp_vec3()
    }
    fn write(v: &DVec3, w: &mut Writer) {
        w.lp_vec3(*v)
    }
}
/// JSON string encoding of a text component.
pub struct Json;
impl Codec<Component> for Json {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Component> {
        r.json_component()
    }
    fn write(v: &Component, w: &mut Writer) {
        w.json_component(v)
    }
}
/// Everything up to the end of the packet, raw.
pub struct Rest;
impl Codec<Bytes> for Rest {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Bytes> {
        Ok(r.rest())
    }
    fn write(v: &Bytes, w: &mut Writer) {
        w.bytes(v)
    }
}
/// Prefixed array of bytes.
pub struct ByteArray;
impl Codec<Bytes> for ByteArray {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Bytes> {
        r.byte_array(crate::transport::MAX_PACKET_DATA_SIZE)
    }
    fn write(v: &Bytes, w: &mut Writer) {
        w.byte_array(v)
    }
}
/// One network NBT value kept as raw bytes.
pub struct RawNbt;
impl Codec<Bytes> for RawNbt {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Bytes> {
        r.nbt_raw()
    }
    fn write(v: &Bytes, w: &mut Writer) {
        w.bytes(v)
    }
}
/// BitSet (26.3 byte form, see [`Reader::bit_set`]).
pub struct BitSet;
impl Codec<Vec<u64>> for BitSet {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Vec<u64>> {
        r.bit_set()
    }
    fn write(v: &Vec<u64>, w: &mut Writer) {
        w.bit_set(v)
    }
}
/// Prefixed Array with an element codec.
pub struct List<C>(std::marker::PhantomData<C>);
impl<T, C: Codec<T>> Codec<Vec<T>> for List<C> {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Vec<T>> {
        r.list(MAX_LIST, C::read)
    }
    fn write(v: &Vec<T>, w: &mut Writer) {
        w.list(v, |w, x| C::write(x, w))
    }
}
/// Prefixed Optional with an element codec.
pub struct Opt<C>(std::marker::PhantomData<C>);
impl<T, C: Codec<T>> Codec<Option<T>> for Opt<C> {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Option<T>> {
        r.option(C::read)
    }
    fn write(v: &Option<T>, w: &mut Writer) {
        w.option(v, |w, x| C::write(x, w))
    }
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
        if id == 0 {
            Ok(IdOr::Inline(T::read(r)?))
        } else {
            Ok(IdOr::Id(id - 1))
        }
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
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            name: r.identifier()?,
            fixed_range: r.read()?,
        })
    }
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
        Ok(Self {
            name: r.string_max(64)?,
            value: r.string()?,
            signature: r.option(|r| r.string_max(1024))?,
        })
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
        Ok(Self {
            uuid: r.uuid()?,
            name: r.string_max(16)?,
            properties: r.list(16, ProfileProperty::read)?,
        })
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
        Ok(Self {
            profile,
            body: r.read()?,
            cape: r.read()?,
            elytra: r.read()?,
            model: r.option(|r| r.var_int())?,
        })
    }
    fn write(&self, w: &mut Writer) {
        match &self.profile {
            ResolvableProfileKind::Partial {
                name,
                uuid,
                properties,
            } => {
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
        // 26.3 BitSets are byte arrays (see `Reader::bit_set`).
        let sky_mask = r.bit_set()?;
        let block_mask = r.bit_set()?;
        let empty_sky_mask = r.bit_set()?;
        let empty_block_mask = r.bit_set()?;
        let sky_arrays = r.list(4096, |r| r.byte_array(2048))?;
        let block_arrays = r.list(4096, |r| r.byte_array(2048))?;
        Ok(Self {
            sky_mask,
            block_mask,
            empty_sky_mask,
            empty_block_mask,
            sky_arrays,
            block_arrays,
        })
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
