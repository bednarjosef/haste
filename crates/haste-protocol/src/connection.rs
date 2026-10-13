//! The client connection: address resolution, handshake, login (offline or
//! online with encryption), configuration, then play in a tokio task.
//!
//! `Connection::connect` runs everything up to and including the play-state
//! `login` packet inline, then spawns two tasks: a reader (decodes packets,
//! answers keep-alive, ping, chunk batches and reconfiguration itself, and
//! forwards everything to the game through an unbounded channel) and a
//! writer (encodes `ServerboundPlay` from the game). The game thread only
//! touches channels: `try_recv`/`recv`/`send`.

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use haste_core::game::{DimensionInfo, GameMode};
use haste_core::{EntityId, Identifier};
use tokio::io::BufReader;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::auth::{self, AuthError, MsaTokens};
use crate::nbt::NbtCompound;
use crate::packets::configuration::clientbound::{
    KnownPack, RegistryEntry, RegistryTags, ReportDetail, ServerLink,
};
use crate::packets::configuration::{self as cfg, ClientboundConfiguration};
use crate::packets::handshake::serverbound::{
    Intention, INTENT_LOGIN, INTENT_STATUS, INTENT_TRANSFER,
};
use crate::packets::login::{self, ClientboundLogin};
use crate::packets::play::{self, ClientboundPlay, ServerboundPlay};
use crate::packets::status::{self, ClientboundStatus};
use crate::packets::{Decoded, Direction, Dispatch, Packet, ProtocolTables, State};
use crate::transport::{
    PacketDecodeError, PacketEncodeError, TcpNetworkDecoder, TcpNetworkEncoder,
};
use crate::wire::{DecodeError, GameProfile, Reader, Writer};

/// Who we log in as.
#[derive(Clone, Debug)]
pub enum Account {
    /// Offline mode: any name, no authentication.
    Offline {
        /// Username (at most 16 chars).
        username: String,
    },
    /// Online mode with Microsoft-derived Minecraft credentials.
    Microsoft(MsaTokens),
}

impl Account {
    /// The username sent in login start.
    pub fn username(&self) -> &str {
        match self {
            Account::Offline { username } => username,
            Account::Microsoft(t) => &t.username,
        }
    }
    /// The UUID sent in login start (offline: nil; vanilla servers ignore it).
    pub fn uuid(&self) -> Uuid {
        match self {
            Account::Offline { .. } => Uuid::nil(),
            Account::Microsoft(t) => t.profile_uuid,
        }
    }
}

/// Connection parameters.
#[derive(Clone, Debug)]
pub struct ConnectOptions {
    /// `host`, `host:port` or `ip:port`. Without a port, `_minecraft._tcp`
    /// SRV is tried, then port 25565.
    pub address: String,
    /// Account.
    pub account: Account,
    /// Protocol version sent in the handshake (777 for 26.3).
    pub protocol_version: i32,
    /// Sent at the start of configuration.
    pub client_information: cfg::serverbound::ClientInformation,
    /// Client brand sent on `minecraft:brand`.
    pub brand: String,
    /// Packet ids and registry tables; `None` loads them from the reports in
    /// the cache (`ProtocolTables::load`).
    pub tables: Option<Arc<ProtocolTables>>,
    /// Chunks per tick requested in `chunk_batch_received`.
    pub chunks_per_tick: f32,
    /// How to answer `resource_pack_push`.
    pub resource_packs: ResourcePackPolicy,
    /// Cookies from `store_cookie`. Clones share the jar, so cookies survive
    /// a `transfer` (vanilla keeps them in memory for the session only).
    pub cookies: CookieJar,
}

/// How the connection answers a server resource pack offer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResourcePackPolicy {
    /// Answer accepted, downloaded, successfully loaded (what a vanilla client
    /// that accepts sends). The pack is not downloaded.
    #[default]
    Accept,
    /// Answer declined (a forced pack then gets us kicked, as in vanilla).
    Decline,
}

/// `resource_pack` result values (serverbound, configuration and play).
pub mod resource_pack_status {
    /// Successfully loaded.
    pub const SUCCESSFULLY_LOADED: i32 = 0;
    /// Declined by the user.
    pub const DECLINED: i32 = 1;
    /// Download failed.
    pub const FAILED_DOWNLOAD: i32 = 2;
    /// Accepted (download starts).
    pub const ACCEPTED: i32 = 3;
    /// Downloaded.
    pub const DOWNLOADED: i32 = 4;
    /// Invalid URL.
    pub const INVALID_URL: i32 = 5;
    /// Reload failed.
    pub const FAILED_RELOAD: i32 = 6;
    /// Discarded.
    pub const DISCARDED: i32 = 7;
}

impl ResourcePackPolicy {
    /// The statuses sent for one offered pack, in order.
    pub fn replies(self) -> &'static [i32] {
        use resource_pack_status::*;
        match self {
            ResourcePackPolicy::Accept => &[ACCEPTED, DOWNLOADED, SUCCESSFULLY_LOADED],
            ResourcePackPolicy::Decline => &[DECLINED],
        }
    }
}

/// Cookies stored by servers, shared between clones.
#[derive(Clone, Debug, Default)]
pub struct CookieJar(Arc<Mutex<HashMap<Identifier, Bytes>>>);

impl CookieJar {
    /// The cookie stored under `key`.
    pub fn get(&self, key: &Identifier) -> Option<Bytes> {
        self.0.lock().ok()?.get(key).cloned()
    }
    /// Stores (replaces) a cookie.
    pub fn store(&self, key: Identifier, payload: Bytes) {
        if let Ok(mut m) = self.0.lock() {
            m.insert(key, payload);
        }
    }
}

impl ConnectOptions {
    /// Defaults: protocol 777, `en_us`, view distance 10, brand `vanilla`
    /// (what the vanilla client sends; servers cannot tell us apart by it),
    /// resource packs accepted.
    pub fn new(address: impl Into<String>, account: Account) -> Self {
        Self {
            address: address.into(),
            account,
            protocol_version: crate::PROTOCOL,
            client_information: cfg::serverbound::ClientInformation::defaults("en_us", 10),
            brand: "vanilla".into(),
            tables: None,
            chunks_per_tick: 64.0,
            resource_packs: ResourcePackPolicy::Accept,
            cookies: CookieJar::default(),
        }
    }
}

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

/// What the play-state `login` told us, plus resolved dimension info.
#[derive(Clone, Debug)]
pub struct Login {
    /// The player's entity id.
    pub entity_id: EntityId,
    /// Dimension (min y, height, skylight from the synced dimension type).
    pub dimension: DimensionInfo,
    /// Game mode.
    pub game_mode: GameMode,
    /// Server view distance.
    pub view_distance: i32,
    /// Server simulation distance.
    pub simulation_distance: i32,
    /// Hardcore.
    pub hardcore: bool,
    /// The profile from `login_finished`.
    pub profile: GameProfile,
    /// The full packet.
    pub packet: play::clientbound::Login,
}

/// Registries, tags and feature flags from the configuration state. Entry
/// data is kept as raw network NBT; entry order defines numeric ids.
#[derive(Clone, Debug, Default)]
pub struct SyncedRegistries {
    registries: Vec<(Identifier, Vec<RegistryEntry>)>,
    /// Tags per registry (`update_tags`), latest packet wins per registry.
    pub tags: Vec<RegistryTags>,
    /// Enabled feature flags.
    pub features: Vec<Identifier>,
    /// Packs the server announced in `select_known_packs`.
    pub server_packs: Vec<KnownPack>,
    /// Pause-menu links from `server_links` (configuration state; later
    /// updates arrive as play packets).
    pub server_links: Vec<ServerLink>,
    /// Crash report lines from `custom_report_details`.
    pub report_details: Vec<ReportDetail>,
}

impl SyncedRegistries {
    /// Stores one `registry_data` packet (replacing an earlier one for the
    /// same registry).
    pub fn insert(&mut self, registry: Identifier, entries: Vec<RegistryEntry>) {
        if let Some(r) = self.registries.iter_mut().find(|(id, _)| *id == registry) {
            r.1 = entries;
        } else {
            self.registries.push((registry, entries));
        }
    }
    /// Entries of a registry in id order.
    pub fn entries(&self, registry: &Identifier) -> Option<&[RegistryEntry]> {
        self.registries
            .iter()
            .find(|(id, _)| id == registry)
            .map(|(_, e)| e.as_slice())
    }
    /// Registry names received.
    pub fn names(&self) -> impl Iterator<Item = &Identifier> {
        self.registries.iter().map(|(id, _)| id)
    }
    /// Numeric id of an entry.
    pub fn index_of(&self, registry: &Identifier, entry: &Identifier) -> Option<usize> {
        self.entries(registry)?.iter().position(|e| &e.id == entry)
    }
    /// Parsed NBT of an entry (`None` if absent or not a compound).
    pub fn entry_nbt(&self, registry: &Identifier, index: usize) -> Option<NbtCompound> {
        let data = self.entries(registry)?.get(index)?.data.as_ref()?;
        Reader::new(data).nbt_compound().ok().flatten()
    }
    /// Dimension info for a `dimension_type` index and dimension name, from
    /// the synced `minecraft:dimension_type` NBT (`min_y`, `height`,
    /// `has_skylight`).
    pub fn dimension_info(
        &self,
        dimension_type: i32,
        dimension: &Identifier,
    ) -> Option<DimensionInfo> {
        let reg = Identifier::minecraft("dimension_type");
        let idx = usize::try_from(dimension_type).ok()?;
        let entry = self.entries(&reg)?.get(idx)?;
        let nbt = self.entry_nbt(&reg, idx)?;
        Some(DimensionInfo {
            name: dimension.clone(),
            dimension_type: entry.id.clone(),
            min_y: nbt.get_int("min_y")?,
            height: nbt.get_int("height")?,
            has_skylight: nbt
                .get_bool("has_skylight")
                .or_else(|| nbt.get_byte("has_skylight").map(|b| b != 0))?,
        })
    }
    fn apply_tags(&mut self, tags: Vec<RegistryTags>) {
        for t in tags {
            if let Some(old) = self.tags.iter_mut().find(|o| o.registry == t.registry) {
                *old = t;
            } else {
                self.tags.push(t);
            }
        }
    }
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

/// A live play-state connection.
pub struct Connection {
    rx: mpsc::UnboundedReceiver<ClientboundPlay>,
    tx: mpsc::UnboundedSender<Outgoing>,
    registries: Arc<SyncedRegistries>,
    /// Registries from a reconfiguration or transfer, swapped in when the
    /// game dequeues the `login` that follows them.
    pending_registries: Arc<Mutex<Option<SyncedRegistries>>>,
    tables: Arc<ProtocolTables>,
    stats: Arc<Stats>,
    closed: Arc<Mutex<Option<String>>>,
    profile: GameProfile,
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

/// Resolves `address` to a socket address and the host/port to put in the
/// handshake. Without a port: SRV `_minecraft._tcp.<host>`, else 25565.
pub async fn resolve(address: &str) -> Result<(String, u16, Vec<SocketAddr>), ProtocolError> {
    let (host, port) = split_host_port(address);
    if let Some(port) = port {
        let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), port))
            .await?
            .collect();
        return Ok((host, port, addrs));
    }
    if host.parse::<std::net::IpAddr>().is_err() {
        if let Ok(resolver) = trust_dns_resolver::TokioAsyncResolver::tokio_from_system_conf() {
            if let Ok(srv) = resolver.srv_lookup(format!("_minecraft._tcp.{host}")).await {
                if let Some(rec) = srv.iter().next() {
                    let target = rec.target().to_utf8().trim_end_matches('.').to_string();
                    let addrs: Vec<SocketAddr> =
                        tokio::net::lookup_host((target.as_str(), rec.port()))
                            .await?
                            .collect();
                    if !addrs.is_empty() {
                        return Ok((target, rec.port(), addrs));
                    }
                }
            }
        }
    }
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 25565))
        .await?
        .collect();
    Ok((host, 25565, addrs))
}

fn split_host_port(address: &str) -> (String, Option<u16>) {
    // [v6]:port, host:port, v6 without port, host.
    if let Some(rest) = address.strip_prefix('[') {
        if let Some((h, p)) = rest.split_once("]:") {
            return (h.to_string(), p.parse().ok());
        }
        return (rest.trim_end_matches(']').to_string(), None);
    }
    match address.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => (h.to_string(), p.parse().ok()),
        _ => (address.to_string(), None),
    }
}

/// The two halves of a framed connection during the pre-play states.
struct Io {
    dec: Decoder,
    enc: Encoder,
    tables: Arc<ProtocolTables>,
    stats: Arc<Stats>,
}

impl Io {
    fn new(stream: TcpStream, tables: Arc<ProtocolTables>, stats: Arc<Stats>) -> Self {
        let _ = stream.set_nodelay(true);
        let (r, w) = stream.into_split();
        Self {
            dec: TcpNetworkDecoder::new(BufReader::with_capacity(64 * 1024, r)),
            enc: TcpNetworkEncoder::new(w),
            tables,
            stats,
        }
    }

    async fn send<P: Packet>(&mut self, state: State, p: &P) -> Result<(), ProtocolError> {
        let bytes = encode(&self.tables, state, p)?;
        self.enc.write_packet(bytes).await?;
        self.enc.flush().await?;
        self.stats.packets_out.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn recv(&mut self) -> Result<crate::transport::RawPacket, ProtocolError> {
        let p = self.dec.get_raw_packet().await?;
        self.stats.packets_in.fetch_add(1, Ordering::Relaxed);
        self.stats
            .bytes_in
            .fetch_add(p.payload.len() as u64, Ordering::Relaxed);
        Ok(p)
    }
}

fn encode<P: Packet>(tables: &ProtocolTables, state: State, p: &P) -> Result<Bytes, ProtocolError> {
    crate::packets::encode_with_id(&tables.packets, state, Direction::Serverbound, p).ok_or_else(
        || ProtocolError::Tables(format!("no serverbound id for {} in {state:?}", P::NAME)),
    )
}

fn decode_with<T>(
    d: &Dispatch<T>,
    raw: &crate::transport::RawPacket,
    tables: &ProtocolTables,
    state: State,
) -> Result<Option<T>, ProtocolError> {
    match d.decode(raw.id, &raw.payload, Some(tables)) {
        Decoded::Ok(p) => Ok(Some(p)),
        Decoded::NoDecoder => {
            tracing::debug!(
                "{state:?}: ignoring packet id {} ({:?})",
                raw.id,
                tables.packets.name(state, Direction::Clientbound, raw.id)
            );
            Ok(None)
        }
        Decoded::Failed(name, e) => Err(ProtocolError::Decode(name, e)),
    }
}

/// Encodes a `minecraft:brand` payload (a String).
fn brand_payload(brand: &str) -> Bytes {
    let mut w = Writer::new();
    w.string(brand);
    w.into_inner().into()
}

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
            decode_with(&d, &raw, &tables, State::Status)?
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
            decode_with(&d, &raw, &tables, State::Status)?
        {
            break;
        }
    }
    Ok(ServerStatus {
        json,
        latency: t0.elapsed(),
    })
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
        for p in early {
            let _ = game_tx.send(p);
        }
        let _ = game_tx.send(ClientboundPlay::Login(info.packet.clone()));

        // Tasks.
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        let closed = Arc::new(Mutex::new(None));
        let pending_registries = Arc::new(Mutex::new(None));
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
            opts,
            regs: regs.clone(),
            pending_registries: pending_registries.clone(),
        }));
        (
            Connection {
                rx: game_rx,
                tx: out_tx,
                registries: Arc::new(regs),
                pending_registries,
                tables,
                stats,
                closed,
                profile,
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
        let p = self.rx.try_recv().ok()?;
        self.on_dequeue(&p);
        Some(p)
    }

    /// Next packet; `None` once the connection is closed and drained.
    pub async fn recv(&mut self) -> Option<ClientboundPlay> {
        let p = self.rx.recv().await?;
        self.on_dequeue(&p);
        Some(p)
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

/// A connection that reached the play state's `login`.
struct Established {
    io: Io,
    profile: GameProfile,
    regs: SyncedRegistries,
    login: play::clientbound::Login,
    /// Play packets that arrived before `login`.
    early: Vec<ClientboundPlay>,
}

enum Handshaked {
    Play(Box<Established>),
    /// The server sent `transfer` during configuration: `host:port`.
    Transfer(String),
}

/// Handshake (with `intent`), login and configuration up to the play-state
/// `login` packet.
async fn establish(
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
    if let Some(addr) = configuration_state(&mut io, opts, &mut regs).await? {
        return Ok(Handshaked::Transfer(addr));
    }

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
    })))
}

/// Connects to `addr` (`host:port`, from a `transfer`) with the transfer
/// intent, following further transfers.
async fn open(
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
fn transfer_address(host: &str, port: i32) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

async fn login_state(io: &mut Io, opts: &ConnectOptions) -> Result<GameProfile, ProtocolError> {
    let tables = io.tables.clone();
    let d = Dispatch::new(
        &tables.packets,
        State::Login,
        Direction::Clientbound,
        ClientboundLogin::DECODERS,
    );
    loop {
        let raw = io.recv().await?;
        let Some(p) = decode_with(&d, &raw, &tables, State::Login)? else {
            continue;
        };
        match p {
            ClientboundLogin::Hello(req) => {
                let secret = auth::new_shared_secret();
                if req.should_authenticate {
                    match &opts.account {
                        Account::Microsoft(t) => {
                            let hash = auth::server_hash(&req.server_id, &secret, &req.public_key);
                            auth::join_session(t, &hash).await?;
                        }
                        Account::Offline { .. } => {
                            return Err(ProtocolError::Protocol(
                                "server is in online mode; an offline account cannot join".into(),
                            ))
                        }
                    }
                }
                let key = login::serverbound::Key {
                    shared_secret: auth::rsa_encrypt(&req.public_key, &secret)?.into(),
                    verify_token: auth::rsa_encrypt(&req.public_key, &req.verify_token)?.into(),
                };
                io.send(State::Login, &key).await?;
                io.enc.set_encryption(&secret)?;
                io.dec.set_encryption(&secret)?;
            }
            ClientboundLogin::LoginCompression(c) => {
                if c.threshold >= 0 {
                    io.enc.set_compression((c.threshold as usize, 6));
                    io.dec.set_compression(c.threshold as usize);
                }
            }
            ClientboundLogin::LoginFinished(f) => {
                io.send(State::Login, &login::serverbound::LoginAcknowledged {})
                    .await?;
                return Ok(f.profile);
            }
            ClientboundLogin::LoginDisconnect(d) => {
                return Err(ProtocolError::Disconnected(d.reason.to_plain(&|_| None)))
            }
            ClientboundLogin::CustomQuery(q) => {
                io.send(
                    State::Login,
                    &login::serverbound::CustomQueryAnswer {
                        message_id: q.message_id,
                        data: None,
                    },
                )
                .await?;
            }
            ClientboundLogin::CookieRequest(c) => {
                let payload = opts.cookies.get(&c.key);
                io.send(
                    State::Login,
                    &login::serverbound::CookieResponse {
                        key: c.key,
                        payload,
                    },
                )
                .await?;
            }
            ClientboundLogin::Unknown { .. } => {}
        }
    }
}

/// What a configuration packet asks the connection to do.
enum ConfigStep {
    Reply(Vec<Bytes>),
    Finished(Bytes),
    Disconnected(String),
    /// Reconnect to `host:port` with the transfer intent.
    Transfer(String),
    Nothing,
}

/// Handles one configuration packet, updating `regs`.
fn configuration_step(
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
        ClientboundConfiguration::ResourcePackPush(r) => {
            tracing::info!(
                "server resource pack {} ({}), forced: {}: {:?}",
                r.id,
                r.url,
                r.forced,
                opts.resource_packs
            );
            let mut v = Vec::new();
            for &result in opts.resource_packs.replies() {
                v.push(encode(tables, s, &sb::ResourcePack { id: r.id, result })?);
            }
            ConfigStep::Reply(v)
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

fn configuration_hello(
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
async fn configuration_state(
    io: &mut Io,
    opts: &ConnectOptions,
    regs: &mut SyncedRegistries,
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
        let raw = io.recv().await?;
        let Some(p) = decode_with(&d, &raw, &tables, State::Configuration)? else {
            continue;
        };
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

struct ReaderCtx {
    dec: Decoder,
    out: mpsc::UnboundedSender<Outgoing>,
    game: mpsc::UnboundedSender<ClientboundPlay>,
    dispatch: Arc<Dispatch<ClientboundPlay>>,
    tables: Arc<ProtocolTables>,
    stats: Arc<Stats>,
    closed: Arc<Mutex<Option<String>>>,
    opts: ConnectOptions,
    regs: SyncedRegistries,
    pending_registries: Arc<Mutex<Option<SyncedRegistries>>>,
}

fn set_closed(closed: &Mutex<Option<String>>, reason: String) {
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

async fn reader_task(mut c: ReaderCtx) {
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
            // Reconfiguration.
            let step = match cfg_dispatch.decode(raw.id, &raw.payload, Some(&c.tables)) {
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
                }
                Ok(ConfigStep::Transfer(addr)) => {
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
            ClientboundPlay::ResourcePackPush(r) => {
                tracing::info!(
                    "server resource pack {} ({}), forced: {}: {:?}",
                    r.id,
                    r.url,
                    r.forced,
                    c.opts.resource_packs
                );
                for &result in c.opts.resource_packs.replies() {
                    reply(
                        &c.out,
                        &c.tables,
                        ServerboundPlay::ResourcePack(sb::ResourcePack { id: r.id, result }),
                    );
                }
            }
            ClientboundPlay::Transfer(t) => transfer_to = Some(transfer_address(&t.host, t.port)),
            ClientboundPlay::StartConfiguration(_) => {
                let _ = c.out.send(Outgoing::PlayAllowed(false));
                reply(
                    &c.out,
                    &c.tables,
                    ServerboundPlay::ConfigurationAcknowledged(sb::ConfigurationAcknowledged {}),
                );
                in_play = false;
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
        if c.game.send(packet).is_err() || is_disconnect {
            break;
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
    match open(addr, &c.opts, &c.tables, &c.stats).await {
        Ok(est) => {
            let Established {
                io,
                regs,
                login,
                early,
                ..
            } = est;
            let Io { dec, enc, .. } = io;
            c.dec = dec;
            let _ = c.out.send(Outgoing::NewEncoder(Box::new(enc)));
            let _ = c.out.send(Outgoing::PlayAllowed(true));
            if let Ok(mut g) = c.pending_registries.lock() {
                *g = Some(regs.clone());
            }
            c.regs = regs;
            let _ = c.game.send(ClientboundPlay::Login(login));
            for p in early {
                let _ = c.game.send(p);
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
fn dump_packet(name: &str, body: &[u8]) {
    if let Some(dir) = std::env::var_os("HASTE_PROTOCOL_DUMP") {
        let file =
            std::path::Path::new(&dir).join(format!("{}.bin", name.replace([':', '/'], "_")));
        if let Err(e) = std::fs::write(&file, body) {
            tracing::warn!("dump {}: {e}", file.display());
        }
    }
}

async fn writer_task(
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

#[cfg(test)]
mod tests;
