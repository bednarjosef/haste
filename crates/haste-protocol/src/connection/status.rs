//! The server list ping: handshake with intent status, status request, ping
//! (vanilla's status state).

use std::sync::Arc;

use tokio::net::TcpStream;

use crate::packets::handshake::serverbound::{Intention, INTENT_STATUS};
use crate::packets::status::{self, ClientboundStatus};
use crate::packets::{Direction, Dispatch, ProtocolTables, State};

use super::address::resolve;
use super::io::{decode_with, Io};
use super::ProtocolError;

/// Server list ping result.
#[derive(Clone, Debug)]
pub struct ServerStatus {
    /// The status JSON (description, players, version, favicon).
    pub json: serde_json::Value,
    /// Round-trip time of the ping request.
    pub latency: std::time::Duration,
}

/// Server list ping: handshake with intent status, status request, ping.
pub async fn ping(
    address: &str,
    tables: Option<Arc<ProtocolTables>>,
) -> Result<ServerStatus, ProtocolError> {
    let tables = match tables {
        Some(t) => t,
        None => Arc::new(ProtocolTables::load().map_err(|e| ProtocolError::Tables(e.to_string()))?),
    };
    let (host, port, addrs) = resolve(address).await?;
    let stream = TcpStream::connect(&addrs[..]).await?;
    let mut io = Io::new(stream, tables.clone(), Arc::default());
    io.send(
        State::Handshake,
        &Intention {
            protocol_version: crate::PROTOCOL,
            server_address: host,
            server_port: port,
            intent: INTENT_STATUS,
        },
    )
    .await?;
    io.send(State::Status, &status::serverbound::StatusRequest {})
        .await?;
    let d = Dispatch::new(
        &tables.packets,
        State::Status,
        Direction::Clientbound,
        ClientboundStatus::DECODERS,
    );
    let json = loop {
        let raw = io.recv().await?;
        if let Some(ClientboundStatus::StatusResponse(s)) =
            decode_with(&d, &raw, &tables, State::Status, &mut io.warned)?
        {
            break serde_json::from_str(&s.json)
                .map_err(|e| ProtocolError::Protocol(format!("status JSON: {e}")))?;
        }
    };
    let t0 = std::time::Instant::now();
    io.send(
        State::Status,
        &status::serverbound::PingRequest { timestamp: 1 },
    )
    .await?;
    loop {
        let raw = io.recv().await?;
        if let Some(ClientboundStatus::PongResponse(_)) =
            decode_with(&d, &raw, &tables, State::Status, &mut io.warned)?
        {
            break;
        }
    }
    Ok(ServerStatus {
        json,
        latency: t0.elapsed(),
    })
}
