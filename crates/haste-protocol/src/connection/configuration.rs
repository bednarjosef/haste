//! The configuration state: known packs, registry data, resource packs,
//! cookies, transfers and the finish (vanilla's configuration packet
//! listener).

use std::sync::atomic::Ordering;

use bytes::Bytes;
use haste_core::Identifier;

use crate::packets::configuration::{self as cfg, ClientboundConfiguration};
use crate::packets::{Direction, Dispatch, ProtocolTables, State};

use super::establish::transfer_address;
use super::io::{brand_payload, decode_with, encode, Io};
use super::server_packs;
use super::{ConnectOptions, ProtocolError, SyncedRegistries};

/// What a configuration packet asks the connection to do.
pub(crate) enum ConfigStep {
    Reply(Vec<Bytes>),
    Finished(Bytes),
    Disconnected(String),
    /// Reconnect to `host:port` with the transfer intent.
    Transfer(String),
    Nothing,
}

/// Handles one configuration packet, updating `regs`.
pub(crate) fn configuration_step(
    p: ClientboundConfiguration,
    tables: &ProtocolTables,
    opts: &ConnectOptions,
    regs: &mut SyncedRegistries,
) -> Result<ConfigStep, ProtocolError> {
    use cfg::serverbound as sb;
    let s = State::Configuration;
    Ok(match p {
        ClientboundConfiguration::SelectKnownPacks(k) => {
            // Select none so the server sends full registry data.
            regs.server_packs = k.packs;
            ConfigStep::Reply(vec![encode(
                tables,
                s,
                &sb::SelectKnownPacks { packs: vec![] },
            )?])
        }
        ClientboundConfiguration::RegistryData(r) => {
            regs.insert(r.registry, r.entries);
            ConfigStep::Nothing
        }
        ClientboundConfiguration::UpdateTags(t) => {
            regs.apply_tags(t.registries);
            ConfigStep::Nothing
        }
        ClientboundConfiguration::UpdateEnabledFeatures(f) => {
            regs.features = f.features;
            ConfigStep::Nothing
        }
        ClientboundConfiguration::KeepAlive(k) => {
            ConfigStep::Reply(vec![encode(tables, s, &sb::KeepAlive { id: k.id })?])
        }
        ClientboundConfiguration::Ping(p) => {
            ConfigStep::Reply(vec![encode(tables, s, &sb::Pong { id: p.id })?])
        }
        ClientboundConfiguration::CookieRequest(c) => {
            let payload = opts.cookies.get(&c.key);
            ConfigStep::Reply(vec![encode(
                tables,
                s,
                &sb::CookieResponse {
                    key: c.key,
                    payload,
                },
            )?])
        }
        ClientboundConfiguration::StoreCookie(c) => {
            opts.cookies.store(c.key, c.payload);
            ConfigStep::Nothing
        }
        ClientboundConfiguration::Transfer(t) => {
            ConfigStep::Transfer(transfer_address(&t.host, t.port))
        }
        ClientboundConfiguration::ServerLinks(l) => {
            regs.server_links = l.links;
            ConfigStep::Nothing
        }
        ClientboundConfiguration::CustomReportDetails(d) => {
            regs.report_details = d.details;
            ConfigStep::Nothing
        }
        ClientboundConfiguration::CodeOfConduct(c) => {
            tracing::info!("server code of conduct (auto-accepted): {}", c.text);
            ConfigStep::Reply(vec![encode(tables, s, &sb::AcceptCodeOfConduct {})?])
        }
        ClientboundConfiguration::FinishConfiguration(_) => {
            ConfigStep::Finished(encode(tables, s, &sb::FinishConfiguration {})?)
        }
        ClientboundConfiguration::Disconnect(d) => {
            ConfigStep::Disconnected(d.reason.to_plain(&|_| None))
        }
        _ => ConfigStep::Nothing,
    })
}

/// The download headers for a server pack (vanilla `createDownloadHeaders`): the
/// player's name and UUID, this client's version and its resource pack format.
pub(crate) fn pack_headers(opts: &ConnectOptions) -> Vec<(String, String)> {
    crate::serverpack::download_headers(
        opts.account.username(),
        &opts.account.uuid().to_string(),
        haste_core::paths::MC_VERSION,
        haste_core::paths::MC_VERSION,
        crate::serverpack::RESOURCE_PACK_FORMAT_MAJOR,
    )
}

/// The brand and client information packets sent at the start of the
/// configuration state (and again after `start_configuration`).
pub(crate) fn configuration_hello(
    tables: &ProtocolTables,
    opts: &ConnectOptions,
) -> Result<Vec<Bytes>, ProtocolError> {
    // Order as in the wiki's login sequence: brand, then client information.
    Ok(vec![
        encode(
            tables,
            State::Configuration,
            &cfg::serverbound::CustomPayload {
                channel: Identifier::minecraft("brand"),
                data: brand_payload(&opts.brand),
            },
        )?,
        encode(tables, State::Configuration, &opts.client_information)?,
    ])
}

/// Runs the configuration state; `Some(host:port)` if the server
/// transferred us instead of finishing.
pub(crate) async fn configuration_state(
    io: &mut Io,
    opts: &ConnectOptions,
    regs: &mut SyncedRegistries,
    packs: &mut server_packs::ConfigPacks<'_>,
) -> Result<Option<String>, ProtocolError> {
    let tables = io.tables.clone();
    for b in configuration_hello(&tables, opts)? {
        io.enc.write_packet(b).await?;
    }
    io.enc.flush().await?;
    let d = Dispatch::new(
        &tables.packets,
        State::Configuration,
        Direction::Clientbound,
        ClientboundConfiguration::DECODERS,
    );
    loop {
        let raw = server_packs::recv_answering(io, packs).await?;
        let Some(p) = decode_with(&d, &raw, &tables, State::Configuration, &mut io.warned)? else {
            continue;
        };
        // Server packs (vanilla `handleResourcePackPush` / `handleResourcePackPop`):
        // an accepted pack downloads inline, so it is in the cache before play.
        if let ClientboundConfiguration::ResourcePackPush(r) = &p {
            let mut out = server_packs::StatusWriter {
                enc: &mut io.enc,
                stats: &io.stats,
                tables: &tables,
            };
            packs.on_push(&mut out, r).await?;
            continue;
        }
        if let ClientboundConfiguration::ResourcePackPop(pop) = &p {
            packs.pop(pop.id);
            continue;
        }
        match configuration_step(p, &tables, opts, regs)? {
            ConfigStep::Reply(v) => {
                for b in v {
                    io.enc.write_packet(b).await?;
                    io.stats.packets_out.fetch_add(1, Ordering::Relaxed);
                }
                io.enc.flush().await?;
            }
            ConfigStep::Finished(ack) => {
                io.enc.write_packet(ack).await?;
                io.enc.flush().await?;
                return Ok(None);
            }
            ConfigStep::Transfer(addr) => return Ok(Some(addr)),
            ConfigStep::Disconnected(r) => return Err(ProtocolError::Disconnected(r)),
            ConfigStep::Nothing => {}
        }
    }
}
