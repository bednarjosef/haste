//! Serverbound status packets.
use crate::packets::{packet, packet_enum};

packet_enum! {
    /// Serverbound status packets.
    ServerboundStatus { StatusRequest, PingRequest }
}

packet! {
    /// Asks for the status JSON.
    StatusRequest = "minecraft:status_request" {}
}

packet! {
    /// Asks for an echo of `timestamp` (latency measurement).
    PingRequest = "minecraft:ping_request" {
        /// Any value; vanilla sends milliseconds.
        timestamp: i64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::test_util::round_trip;

    #[test]
    fn round_trips() {
        round_trip(&StatusRequest {});
        round_trip(&PingRequest { timestamp: -5 });
    }
}
