//! Server data tests: the PNG header check of `ServerData.validateIcon`, and a `server_data`
//! packet storing the MOTD and replacing the icon only when the new one validates.

use std::sync::Arc;

use bytes::Bytes;
use haste_core::{Component, EntityId};
use haste_protocol::haste_core::game::{DimensionInfo, GameMode};
use haste_protocol::packets::play::clientbound as cb;

use super::*;
use crate::net::Session;

fn session() -> Session {
    Session::new(
        EntityId(7),
        GameMode::Creative,
        DimensionInfo::overworld(),
        Arc::new(Vec::new()),
    )
}

/// The start of a PNG file: the signature, the IHDR chunk and its width and height.
fn png_header(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = PNG_SIGNATURE.to_vec();
    bytes.extend_from_slice(&[0, 0, 0, 13]);
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes
}

#[test]
fn an_icon_within_1024_pixels_is_valid() {
    assert!(valid_icon(&png_header(64, 64)));
    assert!(valid_icon(&png_header(1024, 1024)));
}

#[test]
fn an_icon_over_1024_pixels_on_either_side_is_invalid() {
    assert!(!valid_icon(&png_header(1025, 64)));
    assert!(!valid_icon(&png_header(64, 1025)));
}

#[test]
fn bytes_that_are_not_a_png_header_are_invalid() {
    assert!(!valid_icon(&[]));
    assert!(!valid_icon(&png_header(64, 64)[..23]));
    let mut bad_signature = png_header(64, 64);
    bad_signature[1] = b'X';
    assert!(!valid_icon(&bad_signature));
    let mut bad_chunk = png_header(64, 64);
    bad_chunk[12..16].copy_from_slice(b"IDAT");
    assert!(!valid_icon(&bad_chunk));
}

#[test]
fn a_server_data_packet_stores_the_motd_and_a_valid_icon() {
    let mut s = session();
    assert_eq!(s.server_motd, None);
    assert_eq!(s.server_icon, None);
    s.set_server_data(&cb::ServerData {
        motd: Component::text("A Minecraft Server"),
        icon: Some(Bytes::from(png_header(64, 64))),
    });
    assert_eq!(s.server_motd, Some(Component::text("A Minecraft Server")));
    assert_eq!(s.server_icon, Some(png_header(64, 64)));
}

#[test]
fn an_absent_or_invalid_icon_keeps_the_stored_one() {
    let mut s = session();
    s.set_server_data(&cb::ServerData {
        motd: Component::text("first"),
        icon: Some(Bytes::from(png_header(64, 64))),
    });
    s.set_server_data(&cb::ServerData {
        motd: Component::text("second"),
        icon: None,
    });
    s.set_server_data(&cb::ServerData {
        motd: Component::text("third"),
        icon: Some(Bytes::from(png_header(2000, 64))),
    });
    assert_eq!(s.server_motd, Some(Component::text("third")));
    assert_eq!(s.server_icon, Some(png_header(64, 64)));
}
