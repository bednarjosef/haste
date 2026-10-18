//! HUD and screens (jvm-host): vanilla's own `Hud` and screens running in the embedded JVM.
//!
//! [`Ui`] connects the client to [`haste_jvm::GuiPipeline`]:
//! * **Joining** ([`Ui::joined`]): the configuration phase's registries and tags (re-encoded as
//!   the exact `registry_data`/`update_tags` packets) become vanilla registries in Java; the
//!   local player's entity id, profile and dimension type are set.
//! * **Packets** ([`Ui::handle`], game thread, every clientbound play packet in order): the
//!   local player's primitive state (health, food, experience, abilities, game mode, selected
//!   slot, operator level, time) goes into [`haste_jvm::GameState`], which Java mirrors once per
//!   frame and tick; packets whose state is tier 3 (chat, titles, boss bars, tab list,
//!   scoreboard, command tree and suggestions, containers, the player's effects, attributes and
//!   entity data, death) are re-encoded and handed to Java, where vanilla's codec decodes them
//!   and vanilla's components apply them.
//! * **Ticks** ([`Ui::tick`], 20 Hz): the HUD's timers, the chat queue, the open screen.
//! * **Input** ([`Ui::key`], [`Ui::char_typed`], [`Ui::mouse_moved`], [`Ui::mouse_button`],
//!   [`Ui::scroll`]): to the open screen; with no screen, the HUD key bindings (T and / open the
//!   chat, E the inventory, Escape the pause menu, F1 hides the HUD, F3 the debug overlay, Tab
//!   shows the player list while held, 1-9 and the wheel select the hotbar slot). A `false`
//!   return means the client should treat the input as game input.
//! * **Frames** ([`Ui::frame`], render thread): HUD and screen as one [`GuiFrame`] from one JNI
//!   call.
//! * **Outgoing** ([`Ui::drain_outgoing`], game thread, once per tick): what vanilla code sent
//!   (container clicks with hashed stacks, container close, command suggestions, respawn, ...)
//!   decoded from vanilla's encoding into [`ServerboundPlay`], plus chat and commands typed in
//!   the chat screen and the selected hotbar slot.
//!
//! `Ui` is cheap to clone: give the game thread and the render thread one each (the JVM side is
//! serialised internally; each clone has its own frame buffer).
// The input methods (key, char_typed, mouse_moved, mouse_button, scroll) are called from app.rs's
// winit event loop, which translates winit's keys to the SDL numbering haste_jvm::keys expects.
#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::Arc;

use haste_jvm::{GameState, GuiPipeline, JvmHost, ScreenKind};
use haste_protocol::haste_core::game::GameMode;
use haste_protocol::haste_core::gui::{GuiCommand, GuiFrame, TextMetrics};
use haste_protocol::haste_core::{BlockPos, BlockStateId, Component};
use haste_protocol::packets::configuration::clientbound::{
    RegistryData, ServerLinkLabel, UpdateTags,
};
use haste_protocol::packets::play::serverbound as sb;
use haste_protocol::packets::{Decoded, Direction, Dispatch, Packet, ProtocolTables, State};
use haste_protocol::wire::Writer;
use haste_protocol::{ClientboundPlay, Connection, Login, ServerboundPlay};
use parking_lot::Mutex;

pub use haste_jvm::keys;
mod hud;
mod input;
mod packets;
mod screens;
#[cfg(test)]
mod tests;

pub use self::input::bits;
pub use self::input::feed_clicks;
pub use self::packets::ui_handles;

/// The HUD and the vanilla screens of one client. See the module documentation.
#[derive(Clone)]
pub struct Ui {
    host: JvmHost,
    shared: Arc<Shared>,
    frame: GuiFrame,
}

struct Shared {
    tables: Mutex<Option<Arc<ProtocolTables>>>,
    serverbound: Mutex<Option<Dispatch<ServerboundPlay>>>,
    local_entity: AtomicI32,
    /// The hotbar slot the server was last told about (`set_carried_item`), -1 none yet.
    sent_slot: AtomicI32,
    tab_down: AtomicBool,
    /// F1: the HUD is hidden (vanilla `Hud.toggle`); the hand is skipped with it
    /// (`GameRenderer.renderItemInHand`). Java keeps its own copy for the HUD.
    hud_hidden: AtomicBool,
    salt: AtomicU64,
    /// Packets the UI itself decided to send (immediate respawn).
    outbox: Mutex<Vec<ServerboundPlay>>,
}

impl Ui {
    /// A UI over a started JVM (`JvmHost::start`; start it early, on its own thread: vanilla's
    /// bootstrap takes seconds).
    pub fn new(host: JvmHost) -> Ui {
        Ui {
            host,
            shared: Arc::new(Shared {
                tables: Mutex::new(None),
                serverbound: Mutex::new(None),
                local_entity: AtomicI32::new(i32::MIN),
                sent_slot: AtomicI32::new(-1),
                tab_down: AtomicBool::new(false),
                hud_hidden: AtomicBool::new(false),
                salt: AtomicU64::new(0x9e37_79b9_7f4a_7c15),
                outbox: Mutex::new(Vec::new()),
            }),
            frame: GuiFrame::default(),
        }
    }

    /// The GUI pipeline (screens, ticks, frames; menu mode drives it before a session exists).
    pub fn gui(&self) -> &GuiPipeline {
        self.host.gui()
    }

    /// JVM memory diagnostics (`Boot.memoryStats`): `key=value` lines (heap, metaspace,
    /// code cache, compressed class space), empty when the JVM failed to start.
    pub fn jvm_memory(&self) -> String {
        self.host.jvm_memory().unwrap_or_default()
    }

    /// The renderer's text metrics (`Renderer::text_metrics`): vanilla's `Font` measures with them.
    pub fn set_text_metrics(&self, m: Arc<dyn TextMetrics>) {
        self.gui().set_text_metrics(m);
    }

    fn state<T>(&self, f: impl FnOnce(&mut GameState) -> T) -> T {
        self.gui().with_state(f)
    }
}
