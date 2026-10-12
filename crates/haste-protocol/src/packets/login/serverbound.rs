//! Serverbound login packets.
use bytes::Bytes;
use haste_core::Identifier;
use uuid::Uuid;

use crate::packets::{packet, packet_enum};

packet_enum! {
    /// Serverbound login packets.
    ServerboundLogin { Hello, Key, CustomQueryAnswer, LoginAcknowledged, CookieResponse }
}

use crate::wire::{ByteArray, Opt, Rest, VarInt};

packet! {
    /// Login start: username and UUID (unused by vanilla servers).
    Hello = "minecraft:hello" {
        /// Username (at most 16 chars).
        name: String,
        /// Profile UUID.
        uuid: Uuid,
    }
}

packet! {
    /// Encryption response.
    Key = "minecraft:key" {
        /// Shared secret, RSA-encrypted with the server's key.
        shared_secret: Bytes as ByteArray,
        /// Verify token, RSA-encrypted.
        verify_token: Bytes as ByteArray,
    }
}

packet! {
    /// Login plugin response.
    CustomQueryAnswer = "minecraft:custom_query_answer" {
        /// The request id.
        message_id: i32 as VarInt,
        /// Payload if the client understood the request.
        data: Option<Bytes> as Opt<Rest>,
    }
}

packet! {
    /// Acknowledges `login_finished`; switches to configuration.
    LoginAcknowledged = "minecraft:login_acknowledged" {}
}

packet! {
    /// Answer to `cookie_request`.
    CookieResponse = "minecraft:cookie_response" {
        /// Cookie key.
        key: Identifier,
        /// Stored payload, if any.
        payload: Option<Bytes> as Opt<ByteArray>,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::test_util::round_trip;

    #[test]
    fn round_trips() {
        round_trip(&Hello {
            name: "haste".into(),
            uuid: Uuid::from_u128(5),
        });
        round_trip(&Key {
            shared_secret: Bytes::from_static(&[1; 128]),
            verify_token: Bytes::from_static(&[2; 128]),
        });
        round_trip(&CustomQueryAnswer {
            message_id: 1,
            data: None,
        });
        round_trip(&CustomQueryAnswer {
            message_id: 1,
            data: Some(Bytes::from_static(b"ok")),
        });
        round_trip(&LoginAcknowledged {});
        round_trip(&CookieResponse {
            key: Identifier::minecraft("k"),
            payload: Some(Bytes::from_static(b"v")),
        });
    }
}
