//! The reader and writer tokio tasks of a live connection: decoding and
//! answering packets in the play state (and reconfigurations), the transfer
//! follow, and the outgoing queue flush.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use tokio::sync::mpsc;

use crate::packets::configuration::ClientboundConfiguration;
use crate::packets::play::{self, ClientboundPlay, ServerboundPlay};
use crate::packets::{Decoded, Direction, Dispatch, ProtocolTables, State};
use crate::wire::Writer;

use super::configuration::{configuration_hello, configuration_step, ConfigStep};
use super::establish::{open, transfer_address, Established};
use super::io::Io;
use super::server_packs;
use super::{
    Arrived, ConnectOptions, Decoder, Encoder, Outgoing, PacketWakeSlot, Stats, SyncedRegistries,
};

pub(crate) struct ReaderCtx {
    pub(crate) dec: Decoder,
    pub(crate) out: mpsc::UnboundedSender<Outgoing>,
    pub(crate) game: mpsc::UnboundedSender<Arrived>,
    pub(crate) dispatch: Arc<Dispatch<ClientboundPlay>>,
    pub(crate) tables: Arc<ProtocolTables>,
    pub(crate) stats: Arc<Stats>,
    pub(crate) closed: Arc<Mutex<Option<String>>>,
    pub(crate) packs: Arc<server_packs::ServerPacks>,
    pub(crate) opts: ConnectOptions,
    pub(crate) regs: SyncedRegistries,
    pub(crate) pending_registries: Arc<Mutex<Option<SyncedRegistries>>>,
    pub(crate) packet_arrived: PacketWakeSlot,
}

pub(crate) fn set_closed(closed: &Mutex<Option<String>>, reason: String) {
    if let Ok(mut g) = closed.lock() {
        if g.is_none() {
            *g = Some(reason);
        }
    }
}

/// Encodes a play packet as `Outgoing::Raw` (sent even while the game's play
/// packets are held back).
fn reply(out: &mpsc::UnboundedSender<Outgoing>, tables: &ProtocolTables, p: ServerboundPlay) {
    let mut w = Writer::new();
    if let Some(name) = p.name() {
        if let Some(id) = tables.packets.id(State::Play, Direction::Serverbound, name) {
            w.var_int(id);
            p.encode_body(&mut w);
            let _ = out.send(Outgoing::Raw(w.into_inner().into()));
        }
    }
}

pub(crate) async fn reader_task(mut c: ReaderCtx) {
    let mut in_play = true;
    let cfg_dispatch = Dispatch::new(
        &c.tables.packets,
        State::Configuration,
        Direction::Clientbound,
        ClientboundConfiguration::DECODERS,
    );
    loop {
        let raw = match c.dec.get_raw_packet().await {
            Ok(r) => r,
            Err(e) => {
                set_closed(&c.closed, format!("connection lost: {e}"));
                break;
            }
        };
        c.stats.packets_in.fetch_add(1, Ordering::Relaxed);
        c.stats
            .bytes_in
            .fetch_add(raw.payload.len() as u64, Ordering::Relaxed);
        if !in_play {
            // Reconfiguration. Server packs work as in play (one handler for
            // both listeners in vanilla), statuses in the configuration state.
            let step = match cfg_dispatch.decode(raw.id, &raw.payload, Some(&c.tables)) {
                Decoded::Ok(ClientboundConfiguration::ResourcePackPush(r)) => {
                    c.packs.on_push(&r);
                    Ok(ConfigStep::Nothing)
                }
                Decoded::Ok(ClientboundConfiguration::ResourcePackPop(p)) => {
                    c.packs.pop(p.id);
                    Ok(ConfigStep::Nothing)
                }
                Decoded::Ok(p) => configuration_step(p, &c.tables, &c.opts, &mut c.regs),
                Decoded::NoDecoder => Ok(ConfigStep::Nothing),
                Decoded::Failed(name, e) => {
                    set_closed(
                        &c.closed,
                        format!("decoding {name} during reconfiguration: {e}"),
                    );
                    break;
                }
            };
            match step {
                Ok(ConfigStep::Reply(v)) => v.into_iter().for_each(|b| {
                    let _ = c.out.send(Outgoing::Raw(b));
                }),
                Ok(ConfigStep::Finished(ack)) => {
                    let _ = c.out.send(Outgoing::Raw(ack));
                    let _ = c.out.send(Outgoing::PlayAllowed(true));
                    if let Ok(mut g) = c.pending_registries.lock() {
                        *g = Some(c.regs.clone());
                    }
                    in_play = true;
                    c.packs.set_in_play(true);
                }
                Ok(ConfigStep::Transfer(addr)) => {
                    c.packs.set_in_play(true);
                    if !transfer(&mut c, &addr).await {
                        break;
                    }
                    in_play = true;
                }
                Ok(ConfigStep::Disconnected(r)) => {
                    set_closed(&c.closed, format!("disconnected by server: {r}"));
                    break;
                }
                Ok(ConfigStep::Nothing) => {}
                Err(e) => {
                    set_closed(&c.closed, e.to_string());
                    break;
                }
            }
            continue;
        }
        let packet = match c.dispatch.decode(raw.id, &raw.payload, Some(&c.tables)) {
            Decoded::Ok(p) => p,
            Decoded::NoDecoder => {
                if let Ok(mut m) = c.stats.unknown.lock() {
                    *m.entry(raw.id).or_default() += 1;
                }
                ClientboundPlay::Unknown {
                    id: raw.id,
                    body: raw.payload,
                }
            }
            Decoded::Failed(name, e) => {
                if let Ok(mut m) = c.stats.failed.lock() {
                    let ent = m.entry(name).or_insert((0, String::new()));
                    if ent.0 == 0 {
                        tracing::warn!("failed to decode {name}: {e}");
                        dump_packet(name, &raw.payload);
                    }
                    ent.0 += 1;
                    ent.1 = e.to_string();
                }
                ClientboundPlay::Unknown {
                    id: raw.id,
                    body: raw.payload,
                }
            }
        };
        use play::serverbound as sb;
        let mut transfer_to = None;
        match &packet {
            ClientboundPlay::KeepAlive(k) => reply(
                &c.out,
                &c.tables,
                ServerboundPlay::KeepAlive(sb::KeepAlive { id: k.id }),
            ),
            ClientboundPlay::Ping(p) => reply(
                &c.out,
                &c.tables,
                ServerboundPlay::Pong(sb::Pong { id: p.id }),
            ),
            ClientboundPlay::ChunkBatchFinished(_) => reply(
                &c.out,
                &c.tables,
                ServerboundPlay::ChunkBatchReceived(sb::ChunkBatchReceived {
                    chunks_per_tick: c.opts.chunks_per_tick,
                }),
            ),
            ClientboundPlay::CookieRequest(r) => reply(
                &c.out,
                &c.tables,
                ServerboundPlay::CookieResponse(sb::CookieResponse {
                    key: r.key.clone(),
                    payload: c.opts.cookies.get(&r.key),
                }),
            ),
            ClientboundPlay::StoreCookie(s) => {
                c.opts.cookies.store(s.key.clone(), s.payload.clone())
            }
            ClientboundPlay::ResourcePackPush(r) => c.packs.on_push(r),
            // Vanilla `handleResourcePackPop`: the client re-applies without it.
            ClientboundPlay::ResourcePackPop(p) => c.packs.pop(p.id),
            ClientboundPlay::Transfer(t) => transfer_to = Some(transfer_address(&t.host, t.port)),
            ClientboundPlay::StartConfiguration(_) => {
                let _ = c.out.send(Outgoing::PlayAllowed(false));
                reply(
                    &c.out,
                    &c.tables,
                    ServerboundPlay::ConfigurationAcknowledged(sb::ConfigurationAcknowledged {}),
                );
                in_play = false;
                c.packs.set_in_play(false);
                // The server sends every registry again.
                c.regs = SyncedRegistries::default();
                // Whether vanilla sends brand and client information again
                // here is not determined; servers accept it.
                if let Ok(v) = configuration_hello(&c.tables, &c.opts) {
                    v.into_iter().for_each(|b| {
                        let _ = c.out.send(Outgoing::Raw(b));
                    });
                }
            }
            ClientboundPlay::Disconnect(d) => {
                set_closed(
                    &c.closed,
                    format!("disconnected by server: {}", d.reason.to_plain(&|_| None)),
                );
            }
            _ => {}
        }
        let is_disconnect = matches!(packet, ClientboundPlay::Disconnect(_));
        if c.game
            .send(Arrived {
                packet,
                at: std::time::Instant::now(),
            })
            .is_err()
            || is_disconnect
        {
            break;
        }
        // After the send, so a woken drainer always sees the packet.
        if let Ok(g) = c.packet_arrived.lock() {
            if let Some(wake) = g.as_ref() {
                wake();
            }
        }
        if let Some(addr) = transfer_to {
            if !transfer(&mut c, &addr).await {
                break;
            }
        }
    }
    set_closed(&c.closed, "connection closed".into());
}

/// Follows a `transfer`: connects to `addr` with the transfer intent (cookies
/// kept), swaps the reader's decoder and the writer's encoder, and hands the
/// new `login` (and the registries for it) to the game. The game sees the
/// `transfer` packet first, then that `login`. Returns false if it failed
/// (the connection is then closed).
async fn transfer(c: &mut ReaderCtx, addr: &str) -> bool {
    let _ = c.out.send(Outgoing::PlayAllowed(false));
    // Close the old connection first; connecting while it is open makes the
    // server kick the old session ("logged in from another location").
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let _ = c.out.send(Outgoing::CloseCurrent(done_tx));
    let _ = done_rx.await;
    // The new server starts from the pack status as it stands now (vanilla
    // reconnects with the same `ServerData`, which an answer updated).
    c.opts.resource_packs = c.packs.policy();
    match open(addr, &c.opts, &c.tables, &c.stats).await {
        Ok(est) => {
            let Established {
                io,
                regs,
                login,
                early,
                packs,
                ..
            } = est;
            c.packs.set_policy(packs.policy);
            let Io { dec, enc, .. } = io;
            c.dec = dec;
            let _ = c.out.send(Outgoing::NewEncoder(Box::new(enc)));
            let _ = c.out.send(Outgoing::PlayAllowed(true));
            if let Ok(mut g) = c.pending_registries.lock() {
                *g = Some(regs.clone());
            }
            c.regs = regs;
            let at = std::time::Instant::now();
            let _ = c.game.send(Arrived {
                packet: ClientboundPlay::Login(login),
                at,
            });
            for p in early {
                let _ = c.game.send(Arrived { packet: p, at });
            }
            true
        }
        Err(e) => {
            set_closed(&c.closed, format!("transfer to {addr} failed: {e}"));
            false
        }
    }
}

/// With `HASTE_PROTOCOL_DUMP=<dir>`, writes the body of the first packet of
/// each name that failed to decode to `<dir>/<name>.bin` (layout debugging).
pub(crate) fn dump_packet(name: &str, body: &[u8]) {
    if let Some(dir) = std::env::var_os("HASTE_PROTOCOL_DUMP") {
        let file =
            std::path::Path::new(&dir).join(format!("{}.bin", name.replace([':', '/'], "_")));
        if let Err(e) = std::fs::write(&file, body) {
            tracing::warn!("dump {}: {e}", file.display());
        }
    }
}

pub(crate) async fn writer_task(
    enc: Encoder,
    mut rx: mpsc::UnboundedReceiver<Outgoing>,
    tables: Arc<ProtocolTables>,
    stats: Arc<Stats>,
    closed: Arc<Mutex<Option<String>>>,
) {
    // `None` between closing the old connection and a transfer's new one.
    let mut enc = Some(enc);
    let mut play_allowed = true;
    while let Some(first) = rx.recv().await {
        let mut next = Some(first);
        // Write everything queued, then flush once.
        while let Some(o) = next.take() {
            let bytes = match o {
                Outgoing::Play(p) if play_allowed => {
                    let Some(id) = p
                        .name()
                        .and_then(|n| tables.packets.id(State::Play, Direction::Serverbound, n))
                    else {
                        tracing::warn!("no serverbound play id for {:?}", p.name());
                        next = rx.try_recv().ok();
                        continue;
                    };
                    let mut w = Writer::new();
                    w.var_int(id);
                    p.encode_body(&mut w);
                    Some(Bytes::from(w.into_inner()))
                }
                Outgoing::Play(_) => None,
                Outgoing::Raw(b) => Some(b),
                Outgoing::PlayAllowed(a) => {
                    play_allowed = a;
                    None
                }
                Outgoing::CloseCurrent(done) => {
                    if let Some(mut e) = enc.take() {
                        let _ = e.flush().await;
                        // Dropping the write half sends FIN.
                    }
                    let _ = done.send(());
                    None
                }
                Outgoing::NewEncoder(e) => {
                    enc = Some(*e);
                    None
                }
                Outgoing::Shutdown => {
                    if let Some(e) = enc.as_mut() {
                        let _ = e.flush().await;
                    }
                    return;
                }
            };
            if let (Some(b), Some(e)) = (bytes, enc.as_mut()) {
                if let Err(err) = e.write_packet(b).await {
                    set_closed(&closed, format!("write failed: {err}"));
                    return;
                }
                stats.packets_out.fetch_add(1, Ordering::Relaxed);
            }
            next = rx.try_recv().ok();
        }
        if let Some(e) = enc.as_mut() {
            if let Err(err) = e.flush().await {
                set_closed(&closed, format!("flush failed: {err}"));
                return;
            }
        }
    }
}
