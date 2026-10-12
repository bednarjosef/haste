//! Packet structs, one per `minecraft:` packet in `packets.json`, grouped by
//! state and direction. Every struct implements [`Packet`] (decode and encode,
//! so each one round-trips in tests). Per-state enums ([`play::ClientboundPlay`]
//! and friends) dispatch on the runtime id table from [`ids::PacketIds`].
//!
//! Field layouts follow the minecraft.wiki page "Java Edition protocol/Packets"
//! for 26.3 (protocol 777); where the page and the server disagree, NOTES.md
//! says what was verified against the local server.

use bytes::Bytes;

use crate::wire::{DecodeError, DecodeResult, Reader, Writer};

pub mod configuration;
pub mod handshake;
pub mod ids;
pub mod login;
pub mod play;
pub mod status;

pub use ids::{Direction, PacketIds, ProtocolTables, State};

/// A packet body codec.
pub trait Packet: Sized {
    /// The packet's name in `packets.json`, e.g. `minecraft:login`.
    const NAME: &'static str;
    /// Decodes the body (after the packet id).
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self>;
    /// Encodes the body (without the packet id).
    fn encode(&self, w: &mut Writer);

    /// Decodes a complete body and fails if bytes are left over.
    fn decode_exact(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let p = Self::decode(r)?;
        r.finish()?;
        Ok(p)
    }
}

/// Defines a packet struct whose fields are encoded in order. Each field is
/// `name: Type` (canonical [`crate::wire::Wire`] encoding) or
/// `name: Type as Codec` (explicit [`crate::wire::Codec`]).
macro_rules! packet {
    ($(#[$m:meta])* $name:ident = $id:literal { $( $(#[$fm:meta])* $f:ident : $t:ty $(as $c:ty)? ),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Debug, PartialEq)]
        pub struct $name { $( $(#[$fm])* pub $f: $t, )* }
        impl $crate::packets::Packet for $name {
            const NAME: &'static str = $id;
            #[allow(unused_variables)]
            fn decode(r: &mut $crate::wire::Reader<'_>) -> $crate::wire::DecodeResult<Self> {
                Ok(Self { $( $f: packet!(@read r, $t $(, $c)?), )* })
            }
            #[allow(unused_variables)]
            fn encode(&self, w: &mut $crate::wire::Writer) {
                $( packet!(@write w, &self.$f, $t $(, $c)?); )*
            }
        }
    };
    (@read $r:ident, $t:ty) => { <$t as $crate::wire::Wire>::read($r)? };
    (@read $r:ident, $t:ty, $c:ty) => { <$c as $crate::wire::Codec<$t>>::read($r)? };
    (@write $w:ident, $v:expr, $t:ty) => { <$t as $crate::wire::Wire>::write($v, $w) };
    (@write $w:ident, $v:expr, $t:ty, $c:ty) => { <$c as $crate::wire::Codec<$t>>::write($v, $w) };
}
pub(crate) use packet;

/// Defines the enum of all packets of one state and direction, with an
/// `Unknown { id, body }` variant, and its dispatch helpers.
macro_rules! packet_enum {
    ($(#[$m:meta])* $enum:ident { $( $variant:ident ),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Debug, PartialEq)]
        #[allow(clippy::large_enum_variant)]
        pub enum $enum {
            $(
                #[doc = concat!("`", stringify!($variant), "`.")]
                $variant($variant),
            )*
            /// A packet we have no decoder for, or whose decode failed; kept raw.
            Unknown {
                /// Wire id.
                id: i32,
                /// Body after the id.
                body: bytes::Bytes,
            },
        }
        impl $enum {
            /// `(name, decoder)` for every packet of this enum.
            pub const DECODERS: &'static [(&'static str, $crate::packets::DecodeFn<$enum>)] = &[
                $( (<$variant as $crate::packets::Packet>::NAME, |r| Ok($enum::$variant(<$variant as $crate::packets::Packet>::decode_exact(r)?))), )*
            ];
            /// The `minecraft:` name, or `None` for `Unknown`.
            pub fn name(&self) -> Option<&'static str> {
                match self {
                    $( $enum::$variant(_) => Some(<$variant as $crate::packets::Packet>::NAME), )*
                    $enum::Unknown { .. } => None,
                }
            }
            /// Writes the body (without id). `Unknown` writes its raw body.
            pub fn encode_body(&self, w: &mut $crate::wire::Writer) {
                match self {
                    $( $enum::$variant(p) => <$variant as $crate::packets::Packet>::encode(p, w), )*
                    $enum::Unknown { body, .. } => w.bytes(body),
                }
            }
        }
        $(
            impl From<$variant> for $enum {
                fn from(p: $variant) -> Self { $enum::$variant(p) }
            }
        )*
    };
}
pub(crate) use packet_enum;

/// A decoder function stored in a dispatch table.
pub type DecodeFn<T> = fn(&mut Reader<'_>) -> DecodeResult<T>;

/// Id-indexed decoder table for one packet enum, built from [`PacketIds`].
#[derive(Clone)]
pub struct Dispatch<T> {
    by_id: Vec<Option<(&'static str, DecodeFn<T>)>>,
}

impl<T> std::fmt::Debug for Dispatch<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Dispatch({} ids)",
            self.by_id.iter().filter(|x| x.is_some()).count()
        )
    }
}

/// The outcome of decoding one packet through a [`Dispatch`].
pub enum Decoded<T> {
    /// Decoded.
    Ok(T),
    /// The id is not in `packets.json` or we have no struct for it.
    NoDecoder,
    /// We have a struct but the body did not match it.
    Failed(&'static str, DecodeError),
}

impl<T> Dispatch<T> {
    /// Builds the table for `state`/`dir` from the ids. Names listed in
    /// `packets.json` without a decoder in `decoders` stay `Unknown`.
    pub fn new(
        ids: &PacketIds,
        state: State,
        dir: Direction,
        decoders: &'static [(&'static str, DecodeFn<T>)],
    ) -> Self {
        let mut by_id = Vec::new();
        for (id, name) in ids.iter(state, dir) {
            if let Some((n, f)) = decoders.iter().find(|(n, _)| *n == name) {
                let i = id as usize;
                if by_id.len() <= i {
                    by_id.resize(i + 1, None);
                }
                by_id[i] = Some((*n, *f));
            }
        }
        Self { by_id }
    }

    /// The packet name for an id, if it has a decoder.
    pub fn name(&self, id: i32) -> Option<&'static str> {
        usize::try_from(id)
            .ok()
            .and_then(|i| self.by_id.get(i).copied().flatten())
            .map(|(n, _)| n)
    }

    /// Decodes `body` as packet `id`.
    pub fn decode(&self, id: i32, body: &Bytes, tables: Option<&ProtocolTables>) -> Decoded<T> {
        let Some((name, f)) = usize::try_from(id)
            .ok()
            .and_then(|i| self.by_id.get(i).copied().flatten())
        else {
            return Decoded::NoDecoder;
        };
        let mut r = Reader::from_bytes(body);
        if let Some(t) = tables {
            r = r.with_tables(t);
        }
        match f(&mut r) {
            Ok(p) => Decoded::Ok(p),
            Err(e) => Decoded::Failed(name, e),
        }
    }
}

/// Encodes a packet body with its id from the table.
pub fn encode_with_id<P: Packet>(
    ids: &PacketIds,
    state: State,
    dir: Direction,
    p: &P,
) -> Option<Bytes> {
    let id = ids.id(state, dir, P::NAME)?;
    let mut w = Writer::new();
    w.var_int(id);
    p.encode(&mut w);
    Some(w.into_inner().into())
}

#[cfg(test)]
pub(crate) mod test_util {
    use super::*;

    /// Encodes then decodes `p` and checks equality and that nothing is left.
    pub fn round_trip<P: Packet + PartialEq + std::fmt::Debug>(p: &P) {
        let mut w = Writer::new();
        p.encode(&mut w);
        let b = Bytes::from(w.into_inner());
        let mut r = Reader::from_bytes(&b);
        let back = P::decode_exact(&mut r).unwrap_or_else(|e| panic!("{}: {e}", P::NAME));
        assert_eq!(&back, p, "{}", P::NAME);
    }
}
