//! The framed connection halves used before the play state (the shared
//! send/receive plumbing): `Io`, packet encode, decode with the cosmetic-loss
//! rules, and the `minecraft:brand` payload.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use bytes::Bytes;
use tokio::io::BufReader;
use tokio::net::TcpStream;

use crate::packets::{Decoded, Direction, Dispatch, Packet, ProtocolTables, State};
use crate::transport::{TcpNetworkDecoder, TcpNetworkEncoder};
use crate::wire::Writer;

use super::{Decoder, Encoder, ProtocolError, Stats};

/// The two halves of a framed connection during the pre-play states.
pub(crate) struct Io {
    pub(crate) dec: Decoder,
    pub(crate) enc: Encoder,
    pub(crate) tables: Arc<ProtocolTables>,
    pub(crate) stats: Arc<Stats>,
    /// Which [cosmetic-loss packets](COSMETIC_LOSS_PACKETS) have already
    /// warned on this connection.
    pub(crate) warned: WarnedOnce,
}

impl Io {
    pub(crate) fn new(
        stream: TcpStream,
        tables: Arc<ProtocolTables>,
        stats: Arc<Stats>,
    ) -> Self {
        let _ = stream.set_nodelay(true);
        let (r, w) = stream.into_split();
        Self {
            dec: TcpNetworkDecoder::new(BufReader::with_capacity(64 * 1024, r)),
            enc: TcpNetworkEncoder::new(w),
            tables,
            stats,
            warned: WarnedOnce::default(),
        }
    }

    pub(crate) async fn send<P: Packet>(
        &mut self,
        state: State,
        p: &P,
    ) -> Result<(), ProtocolError> {
        let bytes = encode(&self.tables, state, p)?;
        self.enc.write_packet(bytes).await?;
        self.enc.flush().await?;
        self.stats.packets_out.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    pub(crate) async fn recv(&mut self) -> Result<crate::transport::RawPacket, ProtocolError> {
        let p = self.dec.get_raw_packet().await?;
        self.stats.packets_in.fetch_add(1, Ordering::Relaxed);
        self.stats
            .bytes_in
            .fetch_add(p.payload.len() as u64, Ordering::Relaxed);
        Ok(p)
    }
}

pub(crate) fn encode<P: Packet>(
    tables: &ProtocolTables,
    state: State,
    p: &P,
) -> Result<Bytes, ProtocolError> {
    crate::packets::encode_with_id(&tables.packets, state, Direction::Serverbound, p).ok_or_else(
        || ProtocolError::Tables(format!("no serverbound id for {} in {state:?}", P::NAME)),
    )
}

/// Packets whose loss the player only notices as a missing particle or a
/// silent sound, so a body we cannot decode is dropped instead of ending the
/// connection. Vanilla clients already ignore what they cannot decode here,
/// and a server-side mod can register a particle type or sound that Haste's
/// tables have never seen; `minecraft:level_particles`'s particle is the first
/// field and has no length prefix, so the rest of the packet cannot be found
/// either.
///
/// Everything else keeps ending the connection: `minecraft:explode` is
/// deliberately absent, since the same particle field failing there also loses
/// the knockback of an explosion that moves the player.
const COSMETIC_LOSS_PACKETS: [&str; 4] = [
    "minecraft:level_particles",
    "minecraft:sound",
    "minecraft:sound_entity",
    "minecraft:stop_sound",
];

const _: () = assert!(COSMETIC_LOSS_PACKETS.len() <= u32::BITS as usize);

/// Whether losing the packet named `name` is only cosmetic (it is in
/// [`COSMETIC_LOSS_PACKETS`]).
pub(crate) fn cosmetic_loss(name: &str) -> bool {
    COSMETIC_LOSS_PACKETS.contains(&name)
}

/// Which of [`COSMETIC_LOSS_PACKETS`] have already warned on one connection,
/// as one bit per name. The set of names is fixed and tiny, so this needs no
/// allocation on the packet path and no map per packet.
#[derive(Default)]
pub(crate) struct WarnedOnce(u32);

impl WarnedOnce {
    /// `true` the first time `name` in [`COSMETIC_LOSS_PACKETS`] is seen, and
    /// `false` after: names outside the list have no bit and always `false`.
    pub(crate) fn mark(&mut self, name: &str) -> bool {
        let Some(i) = COSMETIC_LOSS_PACKETS.iter().position(|n| *n == name) else {
            return false;
        };
        let bit = 1u32 << i;
        if self.0 & bit != 0 {
            return false;
        }
        self.0 |= bit;
        true
    }
}

pub(crate) fn decode_with<T>(
    d: &Dispatch<T>,
    raw: &crate::transport::RawPacket,
    tables: &ProtocolTables,
    state: State,
    warned: &mut WarnedOnce,
) -> Result<Option<T>, ProtocolError> {
    match d.decode(raw.id, &raw.payload, Some(tables)) {
        Decoded::Ok(p) => Ok(Some(p)),
        Decoded::NoDecoder => {
            tracing::debug!(
                "{state:?}: ignoring packet id {} ({:?})",
                raw.id,
                tables.packets.name(state, Direction::Clientbound, raw.id)
            );
            Ok(None)
        }
        Decoded::Failed(name, e) => {
            if !cosmetic_loss(name) {
                return Err(ProtocolError::Decode(name, e));
            }
            // Cosmetic loss: warn once per name and drop the packet. The rest
            // of the body cannot be skipped either (`level_particles`'
            // particle has no length prefix), so the packet is lost whole.
            if warned.mark(name) {
                tracing::warn!("{state:?}: dropping {name}, its loss is only cosmetic: {e}");
                super::dump_packet(name, &raw.payload);
            }
            Ok(None)
        }
    }
}

/// Encodes a `minecraft:brand` payload (a String).
pub(crate) fn brand_payload(brand: &str) -> Bytes {
    let mut w = Writer::new();
    w.string(brand);
    w.into_inner().into()
}
