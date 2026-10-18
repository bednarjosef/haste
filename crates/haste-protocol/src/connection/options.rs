//! Connection parameters: the account, the connect options, the server pack
//! policy and the cookie jar.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use haste_core::Identifier;
use uuid::Uuid;

use crate::auth::MsaTokens;
use crate::packets::configuration::{self as cfg};
use crate::packets::ProtocolTables;

use super::server_packs::PackPromptHook;

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
#[derive(Clone)]
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
    /// How to answer `resource_pack_push` (the server's saved pack status).
    pub resource_packs: ResourcePackPolicy,
    /// The confirm screen for pushes the policy asks about (see
    /// [`PackPrompter`]). Unset, a push that would ask is accepted: a run
    /// without a screen to ask on (headless runs, tests).
    pub pack_prompt: Option<PackPromptHook>,
    /// Where server resource packs download to (vanilla's `<game
    /// dir>/downloads`, `DownloadQueue`'s cache). Set: pushed packs really
    /// download (hash-checked) and reach the client through
    /// [`Connection::server_packs`]. Unset: the policy's canned
    /// replies only.
    pub downloads_dir: Option<std::path::PathBuf>,
    /// Cookies from `store_cookie`. Clones share the jar, so cookies survive
    /// a `transfer` (vanilla keeps them in memory for the session only).
    pub cookies: CookieJar,
}

/// How the connection answers a server resource pack offer: vanilla's
/// per-server `ServerData.ServerPackStatus` (servers.dat `acceptTextures`),
/// which `ConnectScreen` turns into `ServerPackManager.PackPromptStatus`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResourcePackPolicy {
    /// ENABLED: accept without asking. Without a downloads directory the
    /// answers are canned (accepted, downloaded, successfully loaded).
    #[default]
    Accept,
    /// DISABLED: decline without asking, except a required pack, which asks.
    Decline,
    /// PROMPT: ask with the confirm screen; the answer then settles the
    /// policy for the rest of the connection (vanilla `allowServerPacks` /
    /// `rejectServerPacks`).
    Prompt,
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
            ResourcePackPolicy::Accept | ResourcePackPolicy::Prompt => {
                &[ACCEPTED, DOWNLOADED, SUCCESSFULLY_LOADED]
            }
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

impl std::fmt::Debug for ConnectOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The wake hook is not `Debug` (and says nothing useful): its presence is enough.
        f.debug_struct("ConnectOptions")
            .field("address", &self.address)
            .field("account", &self.account)
            .field("protocol_version", &self.protocol_version)
            .field("client_information", &self.client_information)
            .field("brand", &self.brand)
            .field("tables", &self.tables)
            .field("chunks_per_tick", &self.chunks_per_tick)
            .field("resource_packs", &self.resource_packs)
            .field("pack_prompt", &self.pack_prompt.is_some())
            .field("downloads_dir", &self.downloads_dir)
            .field("cookies", &self.cookies)
            .finish()
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
            pack_prompt: None,
            downloads_dir: None,
            cookies: CookieJar::default(),
        }
    }
}
