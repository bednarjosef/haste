//! Handshake and connection setup: the handshake with `intent`, login and
//! configuration up to the play-state `login` packet, and following a
//! `transfer`.

use std::sync::Arc;

use tokio::net::TcpStream;

use crate::packets::handshake::serverbound::{Intention, INTENT_TRANSFER};
use crate::packets::login;
use crate::packets::play::{self, ClientboundPlay};
use crate::packets::{Decoded, Direction, Dispatch, ProtocolTables, State};
use crate::wire::GameProfile;

use super::address::resolve;
use super::configuration::configuration_state;
use super::io::{cosmetic_loss, Io};
use super::login::login_state;
use super::server_packs;
use super::{ConnectOptions, ProtocolError, Stats, SyncedRegistries};

/// A connection that reached the play state's `login`.
pub(crate) struct Established {
    pub(crate) io: Io,
    pub(crate) profile: GameProfile,
    pub(crate) regs: SyncedRegistries,
    pub(crate) login: play::clientbound::Login,
    /// Play packets that arrived before `login`.
    pub(crate) early: Vec<ClientboundPlay>,
    /// Server resource packs from configuration (the live set, the settled
    /// policy, a prompt still open).
    pub(crate) packs: server_packs::PackHandover,
}

pub(crate) enum Handshaked {
    Play(Box<Established>),
    /// The server sent `transfer` during configuration: `host:port`.
    Transfer(String),
}

/// Handshake (with `intent`), login and configuration up to the play-state
/// `login` packet.
pub(crate) async fn establish(
    stream: TcpStream,
    host: String,
    port: u16,
    opts: &ConnectOptions,
    tables: &Arc<ProtocolTables>,
    stats: &Arc<Stats>,
    intent: i32,
) -> Result<Handshaked, ProtocolError> {
    let mut io = Io::new(stream, tables.clone(), stats.clone());
    io.send(
        State::Handshake,
        &Intention {
            protocol_version: opts.protocol_version,
            server_address: host,
            server_port: port,
            intent,
        },
    )
    .await?;
    io.send(
        State::Login,
        &login::serverbound::Hello {
            name: opts.account.username().to_string(),
            uuid: opts.account.uuid(),
        },
    )
    .await?;
    let profile = login_state(&mut io, opts).await?;

    let mut regs = SyncedRegistries::default();
    let mut packs = server_packs::ConfigPacks::new(opts);
    if let Some(addr) = configuration_state(&mut io, opts, &mut regs, &mut packs).await? {
        return Ok(Handshaked::Transfer(addr));
    }
    let packs = packs.finish();

    // Play: wait for `login`, keeping anything that comes first.
    let play_dispatch = Dispatch::new(
        &tables.packets,
        State::Play,
        Direction::Clientbound,
        ClientboundPlay::DECODERS,
    );
    let mut early = Vec::new();
    let login = loop {
        let raw = io.recv().await?;
        match play_dispatch.decode(raw.id, &raw.payload, Some(tables)) {
            Decoded::Ok(ClientboundPlay::Login(l)) => break l,
            Decoded::Ok(ClientboundPlay::Disconnect(d)) => {
                return Err(ProtocolError::Disconnected(d.reason.to_plain(&|_| None)))
            }
            Decoded::Ok(p) => early.push(p),
            // A cosmetic loss waits for the play `login` too: drop it, keep
            // the connection (vanilla also tolerates these).
            Decoded::Failed(name, e) if cosmetic_loss(name) => {
                if io.warned.mark(name) {
                    tracing::warn!("Play: dropping {name}, its loss is only cosmetic: {e}");
                    super::tasks::dump_packet(name, &raw.payload);
                }
            }
            Decoded::Failed(name, e) => return Err(ProtocolError::Decode(name, e)),
            Decoded::NoDecoder => early.push(ClientboundPlay::Unknown {
                id: raw.id,
                body: raw.payload,
            }),
        }
    };
    Ok(Handshaked::Play(Box::new(Established {
        io,
        profile,
        regs,
        login,
        early,
        packs,
    })))
}

/// Connects to `addr` (`host:port`, from a `transfer`) with the transfer
/// intent, following further transfers.
pub(crate) async fn open(
    addr: &str,
    opts: &ConnectOptions,
    tables: &Arc<ProtocolTables>,
    stats: &Arc<Stats>,
) -> Result<Established, ProtocolError> {
    let mut addr = addr.to_string();
    for _ in 0..8 {
        tracing::info!("transferring to {addr}");
        let (host, port, addrs) = resolve(&addr).await?;
        let stream = TcpStream::connect(&addrs[..]).await?;
        match establish(stream, host, port, opts, tables, stats, INTENT_TRANSFER).await? {
            Handshaked::Play(e) => return Ok(*e),
            Handshaked::Transfer(next) => addr = next,
        }
    }
    Err(ProtocolError::Protocol(
        "more than 8 transfers in a row".into(),
    ))
}

/// `host:port` for a transfer target (IPv6 literals in brackets).
pub(crate) fn transfer_address(host: &str, port: i32) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}
