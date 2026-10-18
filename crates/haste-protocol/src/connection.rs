//! The client connection: address resolution, handshake, login (offline or
//! online with encryption), configuration, then play in a tokio task.
//!
//! `Connection::connect` runs everything up to and including the play-state
//! `login` packet inline, then spawns two tasks: a reader (decodes packets,
//! answers keep-alive, ping, chunk batches and reconfiguration itself, and
//! forwards everything to the game through an unbounded channel) and a
//! writer (encodes `ServerboundPlay` from the game). The game thread only
//! touches channels: `try_recv`/`recv`/`send`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use haste_core::game::{DimensionInfo, GameMode};
use haste_core::EntityId;
use tokio::io::BufReader;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use bytes::Bytes;

use crate::auth::AuthError;
use crate::packets::handshake::serverbound::INTENT_LOGIN;
use crate::packets::play::{ClientboundPlay, ServerboundPlay};
use crate::packets::{Direction, Dispatch, ProtocolTables, State};
use crate::transport::{
    PacketDecodeError, PacketEncodeError, TcpNetworkDecoder, TcpNetworkEncoder,
};
use crate::wire::{DecodeError, GameProfile};

mod address;
pub use address::resolve;
mod configuration;
use configuration::pack_headers;
mod establish;
use establish::{establish, open, Established, Handshaked};
mod io;
use io::{encode, Io};
mod login;
mod options;
pub use options::{resource_pack_status, Account, ConnectOptions, CookieJar, ResourcePackPolicy};
mod registries;
pub use registries::{Login, SyncedRegistries};
mod server_packs;
pub use status::{ping, ServerStatus};
mod status;
pub use server_packs::{
    asks as pack_push_asks, PackAnswer, PackPromptHook, PackPrompter, PendingPackPush,
    REQUIRED_PACK_DISCONNECT,
};
mod tasks;
use tasks::{reader_task, set_closed, writer_task, ReaderCtx};

/// Connection failure.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    /// Socket error.
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    /// Framing/compression/encryption error while reading.
    #[error("reading packet: {0}")]
    Read(#[from] PacketDecodeError),
    /// Error while writing.
    #[error("writing packet: {0}")]
    Write(#[from] PacketEncodeError),
    /// A packet body did not decode.
    #[error("decoding {0}: {1}")]
    Decode(&'static str, DecodeError),
    /// The server closed the connection with a reason.
    #[error("disconnected by server: {0}")]
    Disconnected(String),
    /// The player declined a required server resource pack on the confirm
    /// screen: the client closes the connection itself (vanilla's
    /// `PackConfirmScreen` callback disconnects with
    /// [`REQUIRED_PACK_DISCONNECT`]).
    #[error(
        "declined a required server resource pack ({})",
        REQUIRED_PACK_DISCONNECT
    )]
    RequiredPackDeclined,
    /// Authentication failed.
    #[error("authentication: {0}")]
    Auth(#[from] AuthError),
    /// Packet ids or tables missing.
    #[error("protocol tables: {0}")]
    Tables(String),
    /// Unexpected packet or state.
    #[error("protocol: {0}")]
    Protocol(String),
    /// The connection is closed.
    #[error("connection closed")]
    Closed,
}

/// Counters kept by the connection task.
#[derive(Debug, Default)]
struct Stats {
    packets_in: AtomicU64,
    bytes_in: AtomicU64,
    packets_out: AtomicU64,
    unknown: Mutex<BTreeMap<i32, u64>>,
    failed: Mutex<BTreeMap<&'static str, (u64, String)>>,
}

/// A snapshot of the connection counters.
#[derive(Clone, Debug, Default)]
pub struct StatsSnapshot {
    /// Packets received since connect (all states).
    pub packets_in: u64,
    /// Payload bytes received (after decompression).
    pub bytes_in: u64,
    /// Packets sent.
    pub packets_out: u64,
    /// Play packets with no decoder: `(id, name from packets.json, count)`.
    pub unknown: Vec<(i32, Option<String>, u64)>,
    /// Packets whose decoder failed: `(name, count, last error)`.
    pub decode_failures: Vec<(String, u64, String)>,
}

enum Outgoing {
    /// A play packet from the game.
    Play(Box<ServerboundPlay>),
    /// A pre-encoded packet (id + body) sent regardless of state.
    Raw(Bytes),
    /// Switch: the writer may (true) or may not (false) send play packets.
    PlayAllowed(bool),
    /// Flush and close the current connection (before following a
    /// `transfer`, as vanilla does), then acknowledge.
    CloseCurrent(tokio::sync::oneshot::Sender<()>),
    /// Continue on a new connection (after a `transfer`).
    NewEncoder(Box<Encoder>),
    /// Flush and close.
    Shutdown,
}

type Decoder = TcpNetworkDecoder<BufReader<OwnedReadHalf>>;
type Encoder = TcpNetworkEncoder<OwnedWriteHalf>;

/// The wake hook the client sets on a live connection (see [`Connection`]).
type PacketWake = Arc<dyn Fn() + Send + Sync>;
/// Where the hook lives so the reader task and the owner share it.
type PacketWakeSlot = Arc<Mutex<Option<PacketWake>>>;

/// One queued clientbound packet with the instant the reader task decoded it
/// (vanilla's packets are handled on the render thread's queue the same frame
/// they arrive; the timestamp is how the client measures that wait).
pub struct Arrived {
    pub packet: ClientboundPlay,
    /// When the reader task decoded the packet (`Instant` on the reader's
    /// clock; the game thread compares it with its own `Instant::now`).
    pub at: std::time::Instant,
}

/// A live play-state connection.
pub struct Connection {
    rx: mpsc::UnboundedReceiver<Arrived>,
    tx: mpsc::UnboundedSender<Outgoing>,
    registries: Arc<SyncedRegistries>,
    /// Registries from a reconfiguration or transfer, swapped in when the
    /// game dequeues the `login` that follows them.
    pending_registries: Arc<Mutex<Option<SyncedRegistries>>>,
    /// Server resource packs on this connection (configuration and play
    /// phase): the live set, the policy, pushes waiting for the prompt.
    packs: Arc<server_packs::ServerPacks>,
    tables: Arc<ProtocolTables>,
    stats: Arc<Stats>,
    closed: Arc<Mutex<Option<String>>>,
    profile: GameProfile,
    /// Called by the reader task after it queues a play packet, so the client
    /// can wake whatever drains them (vanilla drains the packet queue every
    /// frame; a 20 Hz tick alone leaves packets waiting up to 50 ms). Cheap
    /// to call: a condvar notify. Set through [`Self::set_packet_wake`] once
    /// the drainer (the game thread's `Shared`) exists.
    packet_arrived: PacketWakeSlot,
    reader: tokio::task::JoinHandle<()>,
    writer: tokio::task::JoinHandle<()>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("profile", &self.profile.name)
            .finish()
    }
}

impl Connection {
    /// Resolves SRV, handshakes, logs in (encryption + Mojang session for
    /// online mode), runs the configuration state (known packs, registry
    /// data, finish) and returns in the play state after the `login` packet.
    pub async fn connect(opts: ConnectOptions) -> Result<(Connection, Login), ProtocolError> {
        let tables = match &opts.tables {
            Some(t) => t.clone(),
            None => {
                Arc::new(ProtocolTables::load().map_err(|e| ProtocolError::Tables(e.to_string()))?)
            }
        };
        let (host, port, addrs) = resolve(&opts.address).await?;
        let stream = TcpStream::connect(&addrs[..]).await?;
        Self::connect_stream(stream, host, port, opts, tables).await
    }

    /// [`Connection::connect`] over an already connected stream.
    pub async fn connect_stream(
        stream: TcpStream,
        host: String,
        port: u16,
        opts: ConnectOptions,
        tables: Arc<ProtocolTables>,
    ) -> Result<(Connection, Login), ProtocolError> {
        let stats = Arc::new(Stats::default());
        let est = match establish(stream, host, port, &opts, &tables, &stats, INTENT_LOGIN).await? {
            Handshaked::Play(e) => *e,
            Handshaked::Transfer(addr) => open(&addr, &opts, &tables, &stats).await?,
        };
        Ok(Self::start(est, opts, tables, stats))
    }

    /// Spawns the reader and writer for an established connection.
    fn start(
        est: Established,
        opts: ConnectOptions,
        tables: Arc<ProtocolTables>,
        stats: Arc<Stats>,
    ) -> (Connection, Login) {
        let Established {
            io,
            profile,
            regs,
            login,
            early,
            packs,
        } = est;
        let dimension = regs.dimension_info(login.spawn.dimension_type, &login.spawn.dimension).unwrap_or_else(|| {
            tracing::warn!("dimension type {} not resolvable from synced registries; assuming overworld layout", login.spawn.dimension_type);
            DimensionInfo { name: login.spawn.dimension.clone(), ..DimensionInfo::overworld() }
        });
        let info = Login {
            entity_id: EntityId(login.entity_id),
            dimension,
            game_mode: GameMode::from_id(login.spawn.game_mode).unwrap_or(GameMode::Survival),
            view_distance: login.view_distance,
            simulation_distance: login.simulation_distance,
            hardcore: login.hardcore,
            profile: profile.clone(),
            packet: login,
        };
        let (game_tx, game_rx) = mpsc::unbounded_channel();
        let packet_arrived: PacketWakeSlot = Arc::new(Mutex::new(None));
        let start = std::time::Instant::now();
        for p in early {
            let _ = game_tx.send(Arrived {
                packet: p,
                at: start,
            });
        }
        let _ = game_tx.send(Arrived {
            packet: ClientboundPlay::Login(info.packet.clone()),
            at: start,
        });

        // Tasks.
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        let closed = Arc::new(Mutex::new(None));
        let pending_registries = Arc::new(Mutex::new(None));
        let packs = server_packs::ServerPacks::new(packs, &opts, out_tx.clone(), tables.clone());
        let Io { dec, enc, .. } = io;
        let writer = tokio::spawn(writer_task(
            enc,
            out_rx,
            tables.clone(),
            stats.clone(),
            closed.clone(),
        ));
        let play_dispatch = Arc::new(Dispatch::new(
            &tables.packets,
            State::Play,
            Direction::Clientbound,
            ClientboundPlay::DECODERS,
        ));
        let reader = tokio::spawn(reader_task(ReaderCtx {
            dec,
            out: out_tx.clone(),
            game: game_tx,
            dispatch: play_dispatch,
            tables: tables.clone(),
            stats: stats.clone(),
            closed: closed.clone(),
            packs: packs.clone(),
            opts,
            regs: regs.clone(),
            pending_registries: pending_registries.clone(),
            packet_arrived: packet_arrived.clone(),
        }));
        (
            Connection {
                rx: game_rx,
                tx: out_tx,
                registries: Arc::new(regs),
                pending_registries,
                packs,
                tables,
                stats,
                closed,
                profile,
                packet_arrived,
                reader,
                writer,
            },
            info,
        )
    }

    /// Next packet without waiting (for the game tick).
    ///
    /// A `login` after the first one follows a reconfiguration
    /// (`start_configuration`) or a `transfer`; when it is returned,
    /// [`Self::registries`] already holds the registries that came with it.
    pub fn try_recv(&mut self) -> Option<ClientboundPlay> {
        let a = self.rx.try_recv().ok()?;
        self.on_dequeue(&a.packet);
        Some(a.packet)
    }

    /// Like [`Self::try_recv`], keeping the arrival timestamp (for the client's
    /// arrival-to-apply timing log).
    pub fn try_recv_arrived(&mut self) -> Option<Arrived> {
        let a = self.rx.try_recv().ok()?;
        self.on_dequeue(&a.packet);
        Some(a)
    }

    /// The connection's live set of server resource packs (vanilla's
    /// `ServerPackManager.packs`): pushes add (a re-push of an id replaces
    /// its download), pops remove. The caller re-reads it whenever
    /// [`Self::server_packs_version`] changes.
    pub fn server_packs(&self) -> Vec<crate::serverpack::DownloadedPack> {
        self.packs.live()
    }

    /// Bumped every time [`Self::server_packs`] changed (a download finished,
    /// a re-push replaced a pack, or a pop arrived).
    pub fn server_packs_version(&self) -> u64 {
        self.packs.version()
    }

    /// The pushes waiting for the player's answer on the confirm screen
    /// (play or reconfiguration; oldest first), empty while none waits. The
    /// client shows them with its [`PackPrompter`] and answers through
    /// [`Self::answer_server_packs`].
    pub fn pending_pack_pushes(&self) -> Vec<PendingPackPush> {
        self.packs.pending()
    }

    /// The connection's live pack policy (a prompt's answer settles it).
    pub fn resource_pack_policy(&self) -> ResourcePackPolicy {
        self.packs.policy()
    }

    /// The player answered the confirm screen for every pending push
    /// (vanilla's `PackConfirmScreen` callback): yes accepts and downloads
    /// them, no declines them; declining a required pack closes the
    /// connection from this side, with [`REQUIRED_PACK_DISCONNECT`] (the
    /// client shows it). `None` if no push was pending.
    pub fn answer_server_packs(&self, accepted: bool) -> Option<PackAnswer> {
        let answer = self.packs.answer(accepted)?;
        if answer.disconnects() {
            set_closed(
                &self.closed,
                ProtocolError::RequiredPackDeclined.to_string(),
            );
            let _ = self.tx.send(Outgoing::Shutdown);
            self.reader.abort();
        }
        Some(answer)
    }

    /// Next packet with its arrival timestamp; `None` once the connection is
    /// closed and drained.
    pub async fn recv_arrived(&mut self) -> Option<Arrived> {
        let a = self.rx.recv().await?;
        self.on_dequeue(&a.packet);
        Some(a)
    }

    /// Next packet; `None` once the connection is closed and drained.
    pub async fn recv(&mut self) -> Option<ClientboundPlay> {
        self.recv_arrived().await.map(|a| a.packet)
    }

    fn on_dequeue(&mut self, p: &ClientboundPlay) {
        if matches!(p, ClientboundPlay::Login(_)) {
            if let Some(r) = self
                .pending_registries
                .lock()
                .ok()
                .and_then(|mut g| g.take())
            {
                self.registries = Arc::new(r);
            }
        }
    }

    /// Queues a packet for sending.
    pub fn send(&self, p: ServerboundPlay) -> Result<(), ProtocolError> {
        self.tx
            .send(Outgoing::Play(Box::new(p)))
            .map_err(|_| ProtocolError::Closed)
    }

    /// Sets the hook the reader task calls after queueing each play packet
    /// (see [`Self::packet_arrived`]). Call once, right after connect.
    pub fn set_packet_wake(&self, wake: PacketWake) {
        if let Ok(mut g) = self.packet_arrived.lock() {
            *g = Some(wake);
        }
    }

    /// Registries from the configuration state.
    pub fn registries(&self) -> &SyncedRegistries {
        &self.registries
    }

    /// Packet id and registry tables in use.
    pub fn tables(&self) -> &Arc<ProtocolTables> {
        &self.tables
    }

    /// The profile the server confirmed in `login_finished`.
    pub fn profile(&self) -> &GameProfile {
        &self.profile
    }

    /// Why the connection closed, once it has.
    pub fn close_reason(&self) -> Option<String> {
        self.closed.lock().ok().and_then(|g| g.clone())
    }

    /// Whether the reader has stopped (server closed, error or disconnect).
    pub fn is_closed(&self) -> bool {
        self.reader.is_finished()
    }

    /// Counters, including unknown packet ids and decode failures.
    pub fn stats(&self) -> StatsSnapshot {
        snapshot(&self.stats, &self.tables)
    }

    /// Closes the connection (flushes queued packets, then TCP FIN).
    pub fn disconnect(self) {
        let _ = self.tx.send(Outgoing::Shutdown);
        self.reader.abort();
        drop(self.writer);
    }
}

fn snapshot(s: &Stats, tables: &ProtocolTables) -> StatsSnapshot {
    let unknown = s
        .unknown
        .lock()
        .map(|m| {
            m.iter()
                .map(|(id, n)| {
                    (
                        *id,
                        tables
                            .packets
                            .name(State::Play, Direction::Clientbound, *id)
                            .map(str::to_string),
                        *n,
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let decode_failures = s
        .failed
        .lock()
        .map(|m| {
            m.iter()
                .map(|(k, (n, e))| (k.to_string(), *n, e.clone()))
                .collect()
        })
        .unwrap_or_default();
    StatsSnapshot {
        packets_in: s.packets_in.load(Ordering::Relaxed),
        bytes_in: s.bytes_in.load(Ordering::Relaxed),
        packets_out: s.packets_out.load(Ordering::Relaxed),
        unknown,
        decode_failures,
    }
}

#[cfg(test)]
mod pack_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod test_imports {
    // Names the old single-file tests reached through `super::*`.
    pub(crate) use super::address::split_host_port;
    pub(crate) use super::io::{decode_with, WarnedOnce, COSMETIC_LOSS_PACKETS};
}
