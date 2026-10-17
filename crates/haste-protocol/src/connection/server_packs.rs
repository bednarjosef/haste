//! Server resource packs on a connection: vanilla's
//! `ClientCommonPacketListenerImpl.handleResourcePackPush` and
//! `handleResourcePackPop`, its `PackConfirmScreen` (the accept/decline
//! prompt), and the prompt status of `ServerPackManager` behind
//! `DownloadedPackSource`, with the `minecraft:resource_pack` statuses in
//! vanilla's order.
//!
//! The download is [`crate::serverpack`]'s; the confirm screen is the
//! client's ([`PackPrompter`], the real vanilla `ConfirmScreen`).
//!
//! The order a push is handled in (vanilla `handleResourcePackPush`): an
//! invalid URL answers INVALID_URL at once and never asks; then the server's
//! saved status decides: enabled pushes, disabled declines (DECLINED), and
//! prompt asks; a required pack asks even when disabled. The answer covers
//! every push waiting on the screen and settles the policy for the rest of
//! the connection (`allowServerPacks` / `rejectServerPacks`). Declining a
//! required pack closes the connection from the client's side, with
//! [`REQUIRED_PACK_DISCONNECT`].

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use bytes::Bytes;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::{Encoder, Outgoing, ProtocolError, ResourcePackPolicy, Stats};
use crate::packets::configuration as cfg;
use crate::packets::play;
use crate::packets::{Direction, ProtocolTables, State};
use crate::resource_pack_status as status;
use crate::serverpack::{self, DownloadedPack};

/// The reason a declined required pack disconnects with (vanilla's
/// `PackConfirmScreen` callback), a translation key.
pub const REQUIRED_PACK_DISCONNECT: &str = "multiplayer.requiredTexturePrompt.disconnect";

/// One `resource_pack_push` waiting for the player's answer (vanilla's
/// `PackConfirmScreen.PendingRequest`, plus the push's required flag and
/// prompt the screen words itself after).
#[derive(Clone, Debug, PartialEq)]
pub struct PendingPackPush {
    /// Pack id.
    pub id: Uuid,
    /// Download URL (http or https; checked before asking).
    pub url: String,
    /// SHA-1 hex, or anything else for none.
    pub hash: String,
    /// Whether the server requires the pack.
    pub forced: bool,
    /// The server's prompt text, if any.
    pub prompt: Option<haste_core::Component>,
}

impl PendingPackPush {
    fn from_packet(r: &cfg::clientbound::ResourcePackPush) -> Self {
        Self {
            id: r.id,
            url: r.url.clone(),
            hash: r.hash.clone(),
            forced: r.forced,
            prompt: r.prompt.clone(),
        }
    }
}

/// The client's confirm screen for pushed packs.
pub trait PackPrompter: Send + Sync {
    /// Opens the confirm screen for `pushes` (every push waiting, oldest
    /// first) over whatever is open, or rebuilds the open one when another
    /// push joined them (vanilla `addOrUpdatePackPrompt` and
    /// `PackConfirmScreen.update`: the screen is worded after the newest
    /// push's required flag and prompt). Must not block.
    fn show(&self, pushes: &[PendingPackPush]);
    /// The player's answer since the last [`Self::show`], once (true = yes).
    fn take_answer(&self) -> Option<bool>;
    /// The answer was applied: remember it per server (vanilla's callback
    /// sets the `ServerData` pack status and saves the server list).
    fn answered(&self, _answer: PackAnswer) {}
}

/// How the connection reaches the client's confirm screen.
pub type PackPromptHook = Arc<dyn PackPrompter>;

/// The player's answer to the confirm screen and what follows from it
/// (vanilla's `PackConfirmScreen` callback).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackAnswer {
    /// Yes / Proceed.
    pub accepted: bool,
    /// The screen's required wording (the newest push's flag).
    pub required: bool,
}

impl PackAnswer {
    /// The answer for the pushes the screen showed.
    pub fn for_pushes(accepted: bool, pushes: &[PendingPackPush]) -> Self {
        Self {
            accepted,
            required: pushes.last().is_some_and(|p| p.forced),
        }
    }

    /// The connection's policy from now on (`allowServerPacks` /
    /// `rejectServerPacks`).
    pub fn policy(self) -> ResourcePackPolicy {
        if self.accepted {
            ResourcePackPolicy::Accept
        } else {
            ResourcePackPolicy::Decline
        }
    }

    /// Whether the client closes the connection (a required pack declined).
    pub fn disconnects(self) -> bool {
        !self.accepted && self.required
    }

    /// The server's new saved pack status (servers.dat `acceptTextures`):
    /// `Some(Some(true))` enabled after yes, `Some(Some(false))` disabled
    /// after no, `None` (left as it was) after declining a required pack.
    pub fn saved_status(self) -> Option<Option<bool>> {
        if self.accepted {
            Some(Some(true))
        } else if self.required {
            None
        } else {
            Some(Some(false))
        }
    }
}

/// Whether a push asks the player (vanilla `handleResourcePackPush`'s rule
/// over the server's saved status): prompt always asks, disabled asks only
/// for a required pack, enabled never asks.
pub fn asks(policy: ResourcePackPolicy, forced: bool) -> bool {
    match policy {
        ResourcePackPolicy::Prompt => true,
        ResourcePackPolicy::Decline => forced,
        ResourcePackPolicy::Accept => false,
    }
}

/// What happens to one push right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PushAction {
    /// Answer INVALID_URL (checked before anything else).
    InvalidUrl,
    /// Wait for the confirm screen.
    Ask,
    /// Answer DECLINED.
    Decline,
    /// Accept and download.
    Accept,
}

pub(crate) fn push_action(policy: ResourcePackPolicy, url: &str, forced: bool) -> PushAction {
    if serverpack::parse_pack_url(url).is_err() {
        PushAction::InvalidUrl
    } else if asks(policy, forced) {
        PushAction::Ask
    } else if policy == ResourcePackPolicy::Decline {
        PushAction::Decline
    } else {
        PushAction::Accept
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The batch's (id, url, hash) for [`serverpack::fetch_batch`].
fn batch_of(pushes: &[PendingPackPush]) -> Vec<(Uuid, String, String)> {
    pushes
        .iter()
        .map(|p| (p.id, p.url.clone(), p.hash.clone()))
        .collect()
}

/// The statuses without a downloads directory (tests, tools): canned
/// accepted, downloaded, successfully loaded for the batch.
fn canned_statuses(pushes: &[PendingPackPush]) -> Vec<(Uuid, i32)> {
    [status::DOWNLOADED, status::SUCCESSFULLY_LOADED]
        .iter()
        .flat_map(|&r| pushes.iter().map(move |p| (p.id, r)))
        .collect()
}

// ---------------------------------------------------------------- configuration (the join)

/// The packs of the join's configuration phase, which runs inline before the
/// connection's tasks exist; handed to [`ServerPacks`] when play starts.
pub(super) struct ConfigPacks<'a> {
    /// The live policy (settled by an answer).
    pub policy: ResourcePackPolicy,
    /// The live set (vanilla `ServerPackManager.packs` that loaded).
    pub live: Vec<DownloadedPack>,
    /// Pushes on the open confirm screen.
    pub waiting: Vec<PendingPackPush>,
    prompter: Option<&'a PackPromptHook>,
    downloads_dir: Option<&'a std::path::Path>,
    headers: Vec<(String, String)>,
}

impl<'a> ConfigPacks<'a> {
    pub(super) fn new(opts: &'a super::ConnectOptions) -> Self {
        Self {
            policy: opts.resource_packs,
            live: Vec::new(),
            waiting: Vec::new(),
            prompter: opts.pack_prompt.as_ref(),
            downloads_dir: opts.downloads_dir.as_deref(),
            headers: super::pack_headers(opts),
        }
    }

    /// The prompter while a push waits for it (the configuration loop then
    /// polls it beside reading packets).
    pub(super) fn waiting_on(&self) -> Option<&'a PackPromptHook> {
        self.prompter.filter(|_| !self.waiting.is_empty())
    }

    /// One push in configuration.
    pub(super) async fn on_push(
        &mut self,
        out: &mut StatusWriter<'_>,
        r: &cfg::clientbound::ResourcePackPush,
    ) -> Result<(), ProtocolError> {
        let push = PendingPackPush::from_packet(r);
        let action = push_action(self.policy, &push.url, push.forced);
        tracing::info!(
            "server resource pack {} ({}), required: {}, {:?}: {action:?}",
            push.id,
            push.url,
            push.forced,
            self.policy
        );
        match action {
            PushAction::InvalidUrl => out.write(&[(push.id, status::INVALID_URL)]).await,
            PushAction::Ask if self.prompter.is_some() => {
                self.waiting.push(push);
                if let Some(p) = self.prompter {
                    p.show(&self.waiting);
                }
                Ok(())
            }
            // No screen to ask on: accept (headless runs, tests).
            PushAction::Ask | PushAction::Accept => self.apply(out, vec![push], false).await,
            PushAction::Decline => self.apply(out, vec![push], true).await,
        }
    }

    /// The player answered the confirm screen.
    pub(super) async fn on_answer(
        &mut self,
        out: &mut StatusWriter<'_>,
        accepted: bool,
    ) -> Result<(), ProtocolError> {
        let batch = std::mem::take(&mut self.waiting);
        let answer = PackAnswer::for_pushes(accepted, &batch);
        tracing::info!("server resource pack prompt answered: {answer:?}");
        self.policy = answer.policy();
        if let Some(p) = self.prompter {
            p.answered(answer);
        }
        if answer.disconnects() {
            // Vanilla disconnects first; the DECLINED it queues after that
            // never reaches the server.
            return Err(ProtocolError::RequiredPackDeclined);
        }
        self.apply(out, batch, !accepted).await
    }

    /// What the live connection starts from.
    pub(super) fn finish(self) -> PackHandover {
        PackHandover {
            policy: self.policy,
            live: self.live,
            waiting: self.waiting,
        }
    }

    /// Vanilla `handleResourcePackPop`: one pack (or all) leaves the set.
    pub(super) fn pop(&mut self, id: Option<Uuid>) {
        match id {
            Some(id) => self.live.retain(|p| p.id != id),
            None => self.live.clear(),
        }
    }

    async fn apply(
        &mut self,
        out: &mut StatusWriter<'_>,
        batch: Vec<PendingPackPush>,
        declined: bool,
    ) -> Result<(), ProtocolError> {
        if declined {
            let v: Vec<_> = batch.iter().map(|p| (p.id, status::DECLINED)).collect();
            return out.write(&v).await;
        }
        let accepted: Vec<_> = batch.iter().map(|p| (p.id, status::ACCEPTED)).collect();
        out.write(&accepted).await?;
        let rest = match self.downloads_dir {
            None => canned_statuses(&batch),
            Some(dir) => {
                let fetched = serverpack::fetch_batch(dir, &batch_of(&batch), &self.headers).await;
                serverpack::settle_batch(fetched, &mut self.live)
            }
        };
        out.write(&rest).await
    }
}

/// How often the join polls the confirm screen for the answer.
const PROMPT_POLL: std::time::Duration = std::time::Duration::from_millis(50);

/// The join's next configuration packet. While a push waits on the confirm
/// screen the answer is polled beside the read and applied as it comes, so
/// keep-alives are still answered while the player decides (vanilla's
/// screen does not stop the connection either). The read stays pending
/// across the polls: a packet read is not cancel-safe.
pub(super) async fn recv_answering(
    io: &mut super::Io,
    packs: &mut ConfigPacks<'_>,
) -> Result<crate::transport::RawPacket, ProtocolError> {
    let Some(prompter) = packs.waiting_on() else {
        return io.recv().await;
    };
    let super::Io {
        dec,
        enc,
        tables,
        stats,
        ..
    } = io;
    let read = dec.get_raw_packet();
    tokio::pin!(read);
    let raw = loop {
        if packs.waiting_on().is_none() {
            break (&mut read).await?;
        }
        tokio::select! {
            r = &mut read => break r?,
            _ = tokio::time::sleep(PROMPT_POLL) => {
                if let Some(accepted) = prompter.take_answer() {
                    let mut w = StatusWriter { enc: &mut *enc, stats, tables };
                    packs.on_answer(&mut w, accepted).await?;
                }
            }
        }
    };
    stats.packets_in.fetch_add(1, Ordering::Relaxed);
    stats
        .bytes_in
        .fetch_add(raw.payload.len() as u64, Ordering::Relaxed);
    Ok(raw)
}

/// The configuration phase's packs as play starts.
pub(super) struct PackHandover {
    pub policy: ResourcePackPolicy,
    pub live: Vec<DownloadedPack>,
    pub waiting: Vec<PendingPackPush>,
}

/// Writes configuration-state `resource_pack` statuses on the join's
/// encoder (borrowed apart from the decoder, whose read stays pending).
pub(super) struct StatusWriter<'a> {
    pub enc: &'a mut Encoder,
    pub stats: &'a Stats,
    pub tables: &'a ProtocolTables,
}

impl StatusWriter<'_> {
    async fn write(&mut self, statuses: &[(Uuid, i32)]) -> Result<(), ProtocolError> {
        if statuses.is_empty() {
            return Ok(());
        }
        for &(id, result) in statuses {
            let bytes = super::encode(
                self.tables,
                State::Configuration,
                &cfg::serverbound::ResourcePack { id, result },
            )?;
            self.enc.write_packet(bytes).await?;
            self.stats.packets_out.fetch_add(1, Ordering::Relaxed);
        }
        self.enc.flush().await?;
        Ok(())
    }
}

// ---------------------------------------------------------------- the live connection

/// The packs of a live connection, shared by the reader task (pushes and
/// pops in play and reconfiguration) and [`super::Connection`] (the set the
/// client applies, the answer from the game thread's confirm screen).
pub(crate) struct ServerPacks {
    live: Mutex<Vec<DownloadedPack>>,
    /// Bumped on every change to `live`.
    version: AtomicU64,
    policy: Mutex<ResourcePackPolicy>,
    pending: Mutex<Vec<PendingPackPush>>,
    /// Which state the statuses are encoded for (play, or configuration
    /// during a reconfiguration).
    in_play: AtomicBool,
    downloads_dir: Option<PathBuf>,
    headers: Vec<(String, String)>,
    out: mpsc::UnboundedSender<Outgoing>,
    tables: Arc<ProtocolTables>,
}

impl ServerPacks {
    pub(super) fn new(
        config: PackHandover,
        opts: &super::ConnectOptions,
        out: mpsc::UnboundedSender<Outgoing>,
        tables: Arc<ProtocolTables>,
    ) -> Arc<Self> {
        let version = u64::from(!config.live.is_empty());
        Arc::new(Self {
            live: Mutex::new(config.live),
            // 1 = a set exists from the configuration phase; 0 = none ever.
            version: AtomicU64::new(version),
            policy: Mutex::new(config.policy),
            // A screen still open when configuration finished stays open
            // into play, like vanilla's.
            pending: Mutex::new(config.waiting),
            in_play: AtomicBool::new(true),
            downloads_dir: opts.downloads_dir.clone(),
            headers: super::pack_headers(opts),
            out,
            tables,
        })
    }

    pub(crate) fn live(&self) -> Vec<DownloadedPack> {
        lock(&self.live).clone()
    }

    pub(crate) fn version(&self) -> u64 {
        self.version.load(Ordering::Relaxed)
    }

    pub(crate) fn policy(&self) -> ResourcePackPolicy {
        *lock(&self.policy)
    }

    pub(crate) fn set_policy(&self, policy: ResourcePackPolicy) {
        *lock(&self.policy) = policy;
    }

    pub(crate) fn set_in_play(&self, in_play: bool) {
        self.in_play.store(in_play, Ordering::Relaxed);
    }

    pub(crate) fn pending(&self) -> Vec<PendingPackPush> {
        lock(&self.pending).clone()
    }

    /// Vanilla `handleResourcePackPop`.
    pub(crate) fn pop(&self, id: Option<Uuid>) {
        let mut live = lock(&self.live);
        match id {
            Some(id) => live.retain(|p| p.id != id),
            None => live.clear(),
        }
        self.version.fetch_add(1, Ordering::Relaxed);
    }

    /// A push in play or reconfiguration. A push that asks waits in
    /// [`Self::pending`] for the game thread's confirm screen.
    pub(crate) fn on_push(self: &Arc<Self>, r: &cfg::clientbound::ResourcePackPush) {
        let push = PendingPackPush::from_packet(r);
        let policy = self.policy();
        let action = push_action(policy, &push.url, push.forced);
        tracing::info!(
            "server resource pack {} ({}), required: {}, {policy:?}: {action:?}",
            push.id,
            push.url,
            push.forced
        );
        match action {
            PushAction::InvalidUrl => self.send(push.id, status::INVALID_URL),
            PushAction::Ask => lock(&self.pending).push(push),
            PushAction::Decline => self.apply(vec![push], true),
            PushAction::Accept => self.apply(vec![push], false),
        }
    }

    /// The player answered the confirm screen for every pending push;
    /// `None` if none was pending. When the answer disconnects, nothing is
    /// sent: the caller closes the connection.
    pub(crate) fn answer(self: &Arc<Self>, accepted: bool) -> Option<PackAnswer> {
        let batch = std::mem::take(&mut *lock(&self.pending));
        if batch.is_empty() {
            return None;
        }
        let answer = PackAnswer::for_pushes(accepted, &batch);
        tracing::info!("server resource pack prompt answered: {answer:?}");
        self.set_policy(answer.policy());
        if !answer.disconnects() {
            self.apply(batch, !accepted);
        }
        Some(answer)
    }

    /// Declines, or accepts and downloads off the reader (vanilla's
    /// `DownloadQueue` runs beside the connection), statuses as they happen.
    fn apply(self: &Arc<Self>, batch: Vec<PendingPackPush>, declined: bool) {
        if declined {
            for p in &batch {
                self.send(p.id, status::DECLINED);
            }
            return;
        }
        for p in &batch {
            self.send(p.id, status::ACCEPTED);
        }
        let Some(dir) = self.downloads_dir.clone() else {
            for (id, result) in canned_statuses(&batch) {
                self.send(id, result);
            }
            return;
        };
        let me = self.clone();
        tokio::spawn(async move {
            let fetched = serverpack::fetch_batch(&dir, &batch_of(&batch), &me.headers).await;
            let results = serverpack::settle_batch(fetched, &mut lock(&me.live));
            if results.iter().any(|&(_, r)| r == status::DOWNLOADED) {
                me.version.fetch_add(1, Ordering::Relaxed);
            }
            for (id, result) in results {
                me.send(id, result);
            }
        });
    }

    /// One `minecraft:resource_pack`, encoded for the reader's state now.
    fn send(&self, id: Uuid, result: i32) {
        let bytes: Option<Bytes> = if self.in_play.load(Ordering::Relaxed) {
            crate::packets::encode_with_id(
                &self.tables.packets,
                State::Play,
                Direction::Serverbound,
                &play::serverbound::ResourcePack { id, result },
            )
        } else {
            crate::packets::encode_with_id(
                &self.tables.packets,
                State::Configuration,
                Direction::Serverbound,
                &cfg::serverbound::ResourcePack { id, result },
            )
        };
        if let Some(b) = bytes {
            let _ = self.out.send(Outgoing::Raw(b));
        }
    }
}
