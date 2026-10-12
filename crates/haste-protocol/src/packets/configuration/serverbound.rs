//! Serverbound configuration packets. Several are shared with play.
use bytes::Bytes;
use haste_core::Identifier;
use uuid::Uuid;

pub use super::clientbound::KnownPack;
use crate::packets::{packet, packet_enum};

packet_enum! {
    /// Serverbound configuration packets.
    ServerboundConfiguration { ClientInformation, CookieResponse, CustomPayload, FinishConfiguration, KeepAlive, Pong, ResourcePack, SelectKnownPacks, CustomClickAction, AcceptCodeOfConduct }
}

use crate::wire::{ByteArray, Opt, Rest, VarInt};

packet! {
    /// Client settings, sent at the start of configuration and on change.
    ClientInformation = "minecraft:client_information" {
        /// Locale, e.g. `en_us`.
        locale: String,
        /// Render distance in chunks.
        view_distance: i8,
        /// 0 enabled, 1 commands only, 2 hidden.
        chat_mode: i32 as VarInt,
        /// Chat colours.
        chat_colors: bool,
        /// Skin part bit mask (0x7f = all).
        displayed_skin_parts: u8,
        /// 0 left, 1 right.
        main_hand: i32 as VarInt,
        /// Text filtering.
        enable_text_filtering: bool,
        /// Show in server listings.
        allow_server_listings: bool,
        /// 0 all, 1 decreased, 2 minimal.
        particle_status: i32 as VarInt,
    }
}

impl ClientInformation {
    /// Vanilla defaults with the given locale and view distance.
    pub fn defaults(locale: &str, view_distance: i8) -> Self {
        Self {
            locale: locale.into(),
            view_distance,
            chat_mode: 0,
            chat_colors: true,
            displayed_skin_parts: 0x7f,
            main_hand: 1,
            enable_text_filtering: false,
            allow_server_listings: true,
            particle_status: 0,
        }
    }
}

packet! {
    /// Answer to `cookie_request`.
    CookieResponse = "minecraft:cookie_response" {
        /// Cookie key.
        key: Identifier,
        /// Payload, if any.
        payload: Option<Bytes> as Opt<ByteArray>,
    }
}

packet! {
    /// Plugin message.
    CustomPayload = "minecraft:custom_payload" {
        /// Channel.
        channel: Identifier,
        /// Data.
        data: Bytes as Rest,
    }
}

packet! {
    /// Acknowledges `finish_configuration`; switches to play.
    FinishConfiguration = "minecraft:finish_configuration" {}
}

packet! {
    /// Echo of the clientbound keep-alive.
    KeepAlive = "minecraft:keep_alive" {
        /// The id received.
        id: i64,
    }
}

packet! {
    /// Answer to `ping`.
    Pong = "minecraft:pong" {
        /// The id received.
        id: i32,
    }
}

packet! {
    /// Resource pack status.
    ResourcePack = "minecraft:resource_pack" {
        /// Pack id.
        id: Uuid,
        /// 0 loaded, 1 declined, 2 failed, 3 accepted, 4 downloaded, 5 invalid URL, 6 reload failed, 7 discarded.
        result: i32 as VarInt,
    }
}

packet! {
    /// The subset of the server's packs the client knows (empty: send all data).
    SelectKnownPacks = "minecraft:select_known_packs" {
        /// Known packs.
        packs: Vec<KnownPack>,
    }
}

packet! {
    /// Click on a `minecraft:custom` text action. Approximate: the wiki lists
    /// a VarInt payload size then NBT; kept as a length-prefixed byte array.
    CustomClickAction = "minecraft:custom_click_action" {
        /// Action id.
        id: Identifier,
        /// Payload bytes.
        payload: Bytes as ByteArray,
    }
}

packet! {
    /// Accepts the code of conduct.
    AcceptCodeOfConduct = "minecraft:accept_code_of_conduct" {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::test_util::round_trip;

    #[test]
    fn round_trips() {
        round_trip(&ClientInformation::defaults("en_us", 10));
        round_trip(&CookieResponse { key: Identifier::minecraft("k"), payload: None });
        round_trip(&CustomPayload { channel: Identifier::minecraft("brand"), data: Bytes::from_static(b"\x05haste") });
        round_trip(&FinishConfiguration {});
        round_trip(&KeepAlive { id: 1 });
        round_trip(&Pong { id: 2 });
        round_trip(&ResourcePack { id: Uuid::from_u128(3), result: 1 });
        round_trip(&SelectKnownPacks { packs: vec![] });
        round_trip(&CustomClickAction { id: Identifier::minecraft("a"), payload: Bytes::from_static(&[0]) });
        round_trip(&AcceptCodeOfConduct {});
    }
}
