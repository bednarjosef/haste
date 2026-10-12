//! Entity metadata ("synched entity data") decoding into
//! `haste_core::entity::MetadataEntry`.
//!
//! Wire format (minecraft.wiki "Java Edition protocol/Entity metadata"): a
//! sequence of `(index: u8, type: VarInt, value)` ended by index `0xff`. The
//! serializer type ids below follow the wiki's table order for 26.3 (0 = Byte
//! .. 43 = Dye Color). Types `haste_core::entity::MetadataValue` does not model
//! are kept as `Other { type_id, raw }` with their exact bytes, so encoding
//! reproduces the input.

use bytes::Bytes;
use haste_core::entity::{ItemStackData, MetadataEntry, MetadataValue, Pose};
use haste_core::{BlockStateId, Direction};

use crate::wire::particle::Particle;
use crate::wire::{Codec, DecodeError, DecodeResult, IdOr, ResolvableProfile, Reader, Wire, Writer};

/// Serializer type ids (wiki table order).
pub mod types {
    #![allow(missing_docs)]
    pub const BYTE: i32 = 0;
    pub const VAR_INT: i32 = 1;
    pub const VAR_LONG: i32 = 2;
    pub const FLOAT: i32 = 3;
    pub const STRING: i32 = 4;
    pub const COMPONENT: i32 = 5;
    pub const OPTIONAL_COMPONENT: i32 = 6;
    pub const ITEM_STACK: i32 = 7;
    pub const BOOLEAN: i32 = 8;
    pub const ROTATIONS: i32 = 9;
    pub const BLOCK_POS: i32 = 10;
    pub const OPTIONAL_BLOCK_POS: i32 = 11;
    pub const DIRECTION: i32 = 12;
    pub const OPTIONAL_UUID: i32 = 13;
    pub const BLOCK_STATE: i32 = 14;
    pub const OPTIONAL_BLOCK_STATE: i32 = 15;
    pub const PARTICLE: i32 = 16;
    pub const PARTICLES: i32 = 17;
    pub const VILLAGER_DATA: i32 = 18;
    pub const OPTIONAL_VAR_INT: i32 = 19;
    pub const POSE: i32 = 20;
    /// 21..=32: cat variant, cat sound variant, cow variant, cow sound
    /// variant, wolf variant, wolf sound variant, frog variant, pig variant,
    /// pig sound variant, chicken variant, chicken sound variant, zombie
    /// nautilus variant (all VarInt registry ids).
    pub const FIRST_VARIANT: i32 = 21;
    pub const LAST_VARIANT: i32 = 32;
    pub const OPTIONAL_GLOBAL_POS: i32 = 33;
    pub const PAINTING_VARIANT: i32 = 34;
    pub const SNIFFER_STATE: i32 = 35;
    pub const ARMADILLO_STATE: i32 = 36;
    pub const COPPER_GOLEM_STATE: i32 = 37;
    pub const WEATHERING_COPPER_STATE: i32 = 38;
    pub const VECTOR3: i32 = 39;
    pub const QUATERNION: i32 = 40;
    pub const RESOLVABLE_PROFILE: i32 = 41;
    pub const HUMANOID_ARM: i32 = 42;
    pub const DYE_COLOR: i32 = 43;
}

/// Inline painting variant (metadata type 34).
#[derive(Clone, Debug, PartialEq)]
struct PaintingVariant;
impl Wire for PaintingVariant {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        r.i32()?;
        r.i32()?;
        r.identifier()?;
        r.option(|r| r.component())?;
        r.option(|r| r.component())?;
        Ok(PaintingVariant)
    }
    fn write(&self, _w: &mut Writer) {}
}

/// Reads one value of serializer type `ty`.
pub fn read_value(r: &mut Reader<'_>, ty: i32) -> DecodeResult<MetadataValue> {
    use types::*;
    Ok(match ty {
        BYTE => MetadataValue::Byte(r.i8()?),
        VAR_INT => MetadataValue::VarInt(r.var_int()?),
        VAR_LONG => MetadataValue::VarLong(r.var_long()?),
        FLOAT => MetadataValue::Float(r.f32()?),
        STRING => MetadataValue::String(r.string()?),
        COMPONENT => MetadataValue::Component(r.component()?),
        OPTIONAL_COMPONENT => MetadataValue::OptionalComponent(r.option(|r| r.component())?),
        ITEM_STACK => MetadataValue::ItemStack(ItemStackData::read(r)?),
        BOOLEAN => MetadataValue::Boolean(r.bool()?),
        ROTATIONS => MetadataValue::Rotations([r.f32()?, r.f32()?, r.f32()?]),
        BLOCK_POS => MetadataValue::BlockPos(r.block_pos()?),
        OPTIONAL_BLOCK_POS => MetadataValue::OptionalBlockPos(r.option(|r| r.block_pos())?),
        DIRECTION => {
            let d = r.var_int()?;
            MetadataValue::Direction(
                u8::try_from(d).ok().and_then(Direction::from_ordinal).ok_or_else(|| DecodeError::Invalid(format!("direction {d}")))?,
            )
        }
        OPTIONAL_UUID => MetadataValue::OptionalUuid(r.option(|r| r.uuid())?),
        BLOCK_STATE => MetadataValue::BlockState(BlockStateId(r.var_int()? as u32)),
        OPTIONAL_BLOCK_STATE => {
            let v = r.var_int()?;
            MetadataValue::OptionalBlockState(if v == 0 { None } else { Some(BlockStateId(v as u32)) })
        }
        POSE => {
            let p = r.var_int()?;
            MetadataValue::Pose(Pose::from_id(p).ok_or_else(|| DecodeError::Invalid(format!("pose {p}")))?)
        }
        _ => {
            let start = r.peek_rest();
            skip_other(r, ty)?;
            let used = start.len() - r.remaining();
            MetadataValue::Other { type_id: ty, raw: r.to_bytes(&start[..used]) }
        }
    })
}

fn skip_other(r: &mut Reader<'_>, ty: i32) -> DecodeResult<()> {
    use types::*;
    match ty {
        PARTICLE => {
            Particle::read(r)?;
        }
        PARTICLES => {
            r.list(crate::wire::MAX_LIST, Particle::read)?;
        }
        VILLAGER_DATA => {
            r.var_int()?;
            r.var_int()?;
            r.var_int()?;
        }
        OPTIONAL_VAR_INT | FIRST_VARIANT..=LAST_VARIANT | SNIFFER_STATE | ARMADILLO_STATE | COPPER_GOLEM_STATE
        | WEATHERING_COPPER_STATE | HUMANOID_ARM | DYE_COLOR => {
            r.var_int()?;
        }
        OPTIONAL_GLOBAL_POS => {
            r.option(|r| {
                r.identifier()?;
                r.block_pos()
            })?;
        }
        PAINTING_VARIANT => {
            IdOr::<PaintingVariant>::read(r)?;
        }
        VECTOR3 => {
            r.take(12)?;
        }
        QUATERNION => {
            r.take(16)?;
        }
        RESOLVABLE_PROFILE => {
            ResolvableProfile::read(r)?;
        }
        _ => return Err(DecodeError::Invalid(format!("entity metadata serializer type {ty} not determined"))),
    }
    Ok(())
}

/// Writes one value with its type id.
pub fn write_value(w: &mut Writer, v: &MetadataValue) {
    use types::*;
    match v {
        MetadataValue::Byte(b) => {
            w.var_int(BYTE);
            w.i8(*b);
        }
        MetadataValue::VarInt(i) => {
            w.var_int(VAR_INT);
            w.var_int(*i);
        }
        MetadataValue::VarLong(i) => {
            w.var_int(VAR_LONG);
            w.var_long(*i);
        }
        MetadataValue::Float(f) => {
            w.var_int(FLOAT);
            w.f32(*f);
        }
        MetadataValue::String(s) => {
            w.var_int(STRING);
            w.string(s);
        }
        MetadataValue::Component(c) => {
            w.var_int(COMPONENT);
            w.component(c);
        }
        MetadataValue::OptionalComponent(c) => {
            w.var_int(OPTIONAL_COMPONENT);
            w.option(c, |w, c| w.component(c));
        }
        MetadataValue::ItemStack(s) => {
            w.var_int(ITEM_STACK);
            w.write(s);
        }
        MetadataValue::Boolean(b) => {
            w.var_int(BOOLEAN);
            w.bool(*b);
        }
        MetadataValue::Rotations(a) => {
            w.var_int(ROTATIONS);
            for f in a {
                w.f32(*f);
            }
        }
        MetadataValue::BlockPos(p) => {
            w.var_int(BLOCK_POS);
            w.block_pos(p);
        }
        MetadataValue::OptionalBlockPos(p) => {
            w.var_int(OPTIONAL_BLOCK_POS);
            w.option(p, |w, p| w.block_pos(p));
        }
        MetadataValue::Direction(d) => {
            w.var_int(DIRECTION);
            w.var_int(*d as i32);
        }
        MetadataValue::OptionalUuid(u) => {
            w.var_int(OPTIONAL_UUID);
            w.option(u, |w, u| w.uuid(u));
        }
        MetadataValue::BlockState(s) => {
            w.var_int(BLOCK_STATE);
            w.var_int(s.0 as i32);
        }
        MetadataValue::OptionalBlockState(s) => {
            w.var_int(OPTIONAL_BLOCK_STATE);
            w.var_int(s.map_or(0, |s| s.0 as i32));
        }
        MetadataValue::Pose(p) => {
            w.var_int(POSE);
            w.var_int(*p as i32);
        }
        MetadataValue::Other { type_id, raw } => {
            w.var_int(*type_id);
            w.bytes(raw);
        }
    }
}

/// The metadata list encoding (entries then `0xff`).
pub struct Metadata;
impl Codec<Vec<MetadataEntry>> for Metadata {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Vec<MetadataEntry>> {
        let mut out = Vec::new();
        loop {
            let index = r.u8()?;
            if index == 0xff {
                return Ok(out);
            }
            let ty = r.var_int()?;
            let value = read_value(r, ty).map_err(|e| DecodeError::Invalid(format!("metadata index {index} type {ty}: {e}")))?;
            out.push(MetadataEntry { index, value });
        }
    }
    fn write(v: &Vec<MetadataEntry>, w: &mut Writer) {
        for e in v {
            w.u8(e.index);
            write_value(w, &e.value);
        }
        w.u8(0xff);
    }
}

/// Convenience: raw bytes of an `Other` value (for tests and tools).
pub fn other(type_id: i32, raw: &'static [u8]) -> MetadataValue {
    MetadataValue::Other { type_id, raw: Bytes::from_static(raw) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use haste_core::{BlockPos, Component, RegistryIndex};
    use uuid::Uuid;

    #[test]
    fn all_modelled_types_round_trip() {
        let entries = vec![
            MetadataEntry { index: 0, value: MetadataValue::Byte(0x02) },
            MetadataEntry { index: 1, value: MetadataValue::VarInt(300) },
            MetadataEntry { index: 2, value: MetadataValue::VarLong(-5) },
            MetadataEntry { index: 3, value: MetadataValue::Float(1.5) },
            MetadataEntry { index: 4, value: MetadataValue::String("s".into()) },
            MetadataEntry { index: 5, value: MetadataValue::Component(Component::text("c")) },
            MetadataEntry { index: 6, value: MetadataValue::OptionalComponent(None) },
            MetadataEntry { index: 7, value: MetadataValue::ItemStack(ItemStackData { count: 1, item: Some(RegistryIndex(5)), components_raw: Bytes::from_static(&[0, 0]) }) },
            MetadataEntry { index: 8, value: MetadataValue::Boolean(true) },
            MetadataEntry { index: 9, value: MetadataValue::Rotations([1.0, 2.0, 3.0]) },
            MetadataEntry { index: 10, value: MetadataValue::BlockPos(BlockPos::new(1, -2, 3)) },
            MetadataEntry { index: 11, value: MetadataValue::OptionalBlockPos(Some(BlockPos::new(0, 0, 0))) },
            MetadataEntry { index: 12, value: MetadataValue::Direction(Direction::East) },
            MetadataEntry { index: 13, value: MetadataValue::OptionalUuid(Some(Uuid::from_u128(1))) },
            MetadataEntry { index: 14, value: MetadataValue::BlockState(BlockStateId(9)) },
            MetadataEntry { index: 15, value: MetadataValue::OptionalBlockState(None) },
            MetadataEntry { index: 16, value: MetadataValue::Pose(Pose::Crouching) },
            MetadataEntry { index: 17, value: other(types::VILLAGER_DATA, &[1, 2, 3]) },
            MetadataEntry { index: 18, value: other(types::VECTOR3, &[0; 12]) },
            MetadataEntry { index: 19, value: other(types::DYE_COLOR, &[14]) },
            MetadataEntry { index: 20, value: other(types::OPTIONAL_GLOBAL_POS, &[0]) },
        ];
        let mut w = Writer::new();
        Metadata::write(&entries, &mut w);
        let mut r = Reader::new(&w.buf);
        assert_eq!(Metadata::read(&mut r).unwrap(), entries);
        r.finish().unwrap();
    }

    #[test]
    fn unknown_type_is_an_error() {
        assert!(Metadata::read(&mut Reader::new(&[0, 99, 0, 0xff])).is_err());
        assert_eq!(Metadata::read(&mut Reader::new(&[0xff])).unwrap(), vec![]);
    }
}
