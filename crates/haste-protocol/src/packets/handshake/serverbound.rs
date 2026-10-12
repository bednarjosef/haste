//! Serverbound handshake packets.
use crate::packets::{packet, packet_enum};

packet_enum! {
    /// Serverbound handshake packets.
    ServerboundHandshake { Intention }
}

use crate::wire::VarInt;

/// `intention` value for the status state.
pub const INTENT_STATUS: i32 = 1;
/// `intention` value for the login state.
pub const INTENT_LOGIN: i32 = 2;
/// `intention` value for a login caused by a `transfer` packet.
pub const INTENT_TRANSFER: i32 = 3;

packet! {
    /// Handshake: protocol version, address, port, next state.
    Intention = "minecraft:intention" {
        /// Protocol version (777 for 26.3).
        protocol_version: i32 as VarInt,
        /// Host the client connected to (after SRV resolution).
        server_address: String,
        /// Port the client connected to.
        server_port: u16,
        /// [`INTENT_STATUS`], [`INTENT_LOGIN`] or [`INTENT_TRANSFER`].
        intent: i32 as VarInt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::test_util::round_trip;

    #[test]
    fn round_trips() {
        round_trip(&Intention { protocol_version: 777, server_address: "localhost".into(), server_port: 25565, intent: INTENT_LOGIN });
    }
}
