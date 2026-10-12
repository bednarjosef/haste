//! Clientbound status packets.
use crate::packets::{packet, packet_enum};

packet_enum! {
    /// Clientbound status packets.
    ClientboundStatus { StatusResponse, PongResponse }
}

packet! {
    /// The server list ping JSON (description, players, version, favicon).
    StatusResponse = "minecraft:status_response" {
        /// JSON document.
        json: String,
    }
}

packet! {
    /// Echo of `ping_request`.
    PongResponse = "minecraft:pong_response" {
        /// The timestamp the client sent.
        timestamp: i64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::test_util::round_trip;

    #[test]
    fn round_trips() {
        round_trip(&StatusResponse {
            json: r#"{"version":{"name":"26.3","protocol":777}}"#.into(),
        });
        round_trip(&PongResponse {
            timestamp: 123456789,
        });
    }
}
