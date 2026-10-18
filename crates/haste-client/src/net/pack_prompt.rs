//! The server resource pack prompt on the client's side: vanilla's
//! `ClientCommonPacketListenerImpl.PackConfirmScreen`, the real vanilla
//! `ConfirmScreen` opened through the GUI bridge, and the per-server pack
//! status in servers.dat (`ServerData.ServerPackStatus`, the "Server Resource
//! Packs" button of vanilla's `ManageServerScreen`).
//!
//! The decisions and the statuses sent are the connection's
//! (`haste_protocol`'s `server_packs`); this module shows the screen, hands
//! the answer back and remembers it per server.

use haste_protocol::haste_core::Component;
use haste_protocol::{
    Connection, PackAnswer, PackPrompter, PendingPackPush, ResourcePackPolicy,
    REQUIRED_PACK_DISCONNECT,
};

use crate::ui::Ui;

fn servers_dat() -> std::path::PathBuf {
    crate::app::game_dir().join("servers.dat")
}

/// The server's saved pack status as the connection's policy (vanilla
/// `ConnectScreen.startConnecting` with `convertPackStatus`): enabled
/// accepts, disabled declines, prompt (and a server not in the list) asks.
pub fn policy_for(address: &str) -> ResourcePackPolicy {
    let status = match haste_setup::servers::ServerList::load(&servers_dat()) {
        Ok(list) => list.pack_status_for(address),
        Err(e) => {
            tracing::warn!("reading the server list for the pack status: {e:#}");
            None
        }
    };
    match status {
        Some(true) => ResourcePackPolicy::Accept,
        Some(false) => ResourcePackPolicy::Decline,
        None => ResourcePackPolicy::Prompt,
    }
}

/// Saves the answer's pack status for the server (vanilla's callback sets
/// the `ServerData` status, then `ServerList.saveSingleServer`).
fn remember(address: &str, answer: PackAnswer) {
    let Some(status) = answer.saved_status() else {
        return;
    };
    match haste_setup::servers::ServerList::save_pack_status(&servers_dat(), address, status) {
        Ok(true) => tracing::info!("server {address}: resource packs {status:?} saved"),
        Ok(false) => {}
        Err(e) => tracing::warn!("saving the pack status of {address}: {e:#}"),
    }
}

/// Opens (or rebuilds) the confirm screen for the waiting pushes, worded
/// after the newest one (vanilla `PackConfirmScreen.update`).
fn show(ui: &Ui, pushes: &[PendingPackPush]) {
    let Some(newest) = pushes.last() else {
        return;
    };
    let prompt = newest
        .prompt
        .as_ref()
        .and_then(|c| serde_json::to_string(c).ok());
    ui.gui().show_pack_prompt(newest.forced, prompt.as_deref());
}

/// The confirm screen while joining (configuration phase), for
/// `ConnectOptions::pack_prompt`.
pub struct ScreenPrompter {
    ui: Ui,
    address: String,
}

impl ScreenPrompter {
    pub fn new(ui: Ui, address: &str) -> Self {
        Self {
            ui,
            address: address.to_owned(),
        }
    }
}

impl PackPrompter for ScreenPrompter {
    fn show(&self, pushes: &[PendingPackPush]) {
        show(&self.ui, pushes);
    }

    fn take_answer(&self) -> Option<bool> {
        self.ui.gui().take_pack_prompt_answer()
    }

    fn answered(&self, answer: PackAnswer) {
        remember(&self.address, answer);
    }
}

/// The confirm screen in play (and reconfiguration), run by the game thread
/// once per tick.
#[derive(Debug, Default)]
pub struct PlayPrompt {
    /// The server's address as joined (the servers.dat key).
    address: String,
    /// How many waiting pushes the open screen shows.
    shown: usize,
}

impl PlayPrompt {
    pub fn new(address: &str) -> Self {
        Self {
            address: address.to_owned(),
            shown: 0,
        }
    }

    /// Shows the screen when a push started waiting (again when another
    /// joined it) and applies the answer. Without a UI there is no screen to
    /// ask on: the pushes are accepted, like a join without one.
    pub fn tick(&mut self, conn: &Connection, ui: Option<&Ui>) {
        let waiting = conn.pending_pack_pushes();
        if waiting.is_empty() {
            self.shown = 0;
            return;
        }
        let Some(ui) = ui else {
            conn.answer_server_packs(true);
            return;
        };
        if waiting.len() != self.shown {
            show(ui, &waiting);
            self.shown = waiting.len();
            return;
        }
        let Some(accepted) = ui.gui().take_pack_prompt_answer() else {
            return;
        };
        self.shown = 0;
        let Some(answer) = conn.answer_server_packs(accepted) else {
            return;
        };
        remember(&self.address, answer);
        if answer.disconnects() {
            // The connection is closed already; vanilla's disconnect screen
            // shows the reason.
            ui.disconnected(&Component::translate(REQUIRED_PACK_DISCONNECT, vec![]));
        }
    }
}
