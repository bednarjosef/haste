//! Clientbound login packets.
use bytes::Bytes;
use haste_core::{Component, Identifier};
use uuid::Uuid;

use crate::packets::{packet, packet_enum};

packet_enum! {
    /// Clientbound login packets.
    ClientboundLogin { LoginDisconnect, Hello, LoginFinished, LoginCompression, CustomQuery, CookieRequest }
}

use crate::wire::{ByteArray, GameProfile, Json, Rest, VarInt};

packet! {
    /// Login refused; the reason is a JSON text component.
    LoginDisconnect = "minecraft:login_disconnect" {
        /// Why.
        reason: Component as Json,
    }
}

packet! {
    /// Encryption request.
    Hello = "minecraft:hello" {
        /// Server id string (empty on vanilla since 1.7).
        server_id: String,
        /// The server's RSA public key, DER `SubjectPublicKeyInfo`.
        public_key: Bytes as ByteArray,
        /// Random bytes to send back encrypted.
        verify_token: Bytes as ByteArray,
        /// Whether the client must join through the Mojang session server.
        should_authenticate: bool,
    }
}

packet! {
    /// Login succeeded; the client answers `login_acknowledged` and enters
    /// the configuration state.
    LoginFinished = "minecraft:login_finished" {
        /// The player's profile as the server sees it.
        profile: GameProfile,
        /// Session id (26.x addition per the wiki).
        session_id: Uuid,
    }
}

packet! {
    /// Enables compression for every following packet.
    LoginCompression = "minecraft:login_compression" {
        /// Packets of at least this many bytes are compressed; negative disables.
        threshold: i32 as VarInt,
    }
}

packet! {
    /// Login plugin request (vanilla answers "not understood").
    CustomQuery = "minecraft:custom_query" {
        /// Request id to echo.
        message_id: i32 as VarInt,
        /// Channel.
        channel: Identifier,
        /// Channel-specific data.
        data: Bytes as Rest,
    }
}

packet! {
    /// Asks for a stored cookie.
    CookieRequest = "minecraft:cookie_request" {
        /// Cookie key.
        key: Identifier,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::test_util::round_trip;
    use crate::wire::ProfileProperty;

    #[test]
    fn round_trips() {
        round_trip(&LoginDisconnect { reason: Component::text("bye") });
        round_trip(&Hello { server_id: String::new(), public_key: Bytes::from_static(&[1, 2, 3]), verify_token: Bytes::from_static(&[9; 4]), should_authenticate: true });
        round_trip(&LoginFinished {
            profile: GameProfile { uuid: Uuid::from_u128(7), name: "Steve".into(), properties: vec![ProfileProperty { name: "textures".into(), value: "e30=".into(), signature: Some("sig".into()) }] },
            session_id: Uuid::from_u128(99),
        });
        round_trip(&LoginCompression { threshold: 256 });
        round_trip(&CustomQuery { message_id: 3, channel: Identifier::new("velocity", "player_info"), data: Bytes::from_static(b"xyz") });
        round_trip(&CookieRequest { key: Identifier::minecraft("k") });
    }
}
