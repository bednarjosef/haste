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
use haste_protocol::haste_core::Component;
use haste_protocol::packets::configuration::clientbound::{
    RegistryData, ServerLinkLabel, UpdateTags,
};
use haste_protocol::packets::play::serverbound as sb;
use haste_protocol::packets::{Decoded, Direction, Dispatch, Packet, ProtocolTables, State};
use haste_protocol::wire::Writer;
use haste_protocol::{ClientboundPlay, Connection, Login, ServerboundPlay};
use parking_lot::Mutex;

pub use haste_jvm::keys;

/// The bit values of `haste.gui.OptionBridge`'s game-input and click masks (the Java constants
/// are the source; keep these in step with them).
pub mod bits {
    /// The movement and action `KeyMapping`s' down state (`Bridge.gameInput`).
    pub const B_FORWARD: i32 = 1;
    pub const B_BACK: i32 = 1 << 1;
    pub const B_LEFT: i32 = 1 << 2;
    pub const B_RIGHT: i32 = 1 << 3;
    pub const B_JUMP: i32 = 1 << 4;
    pub const B_SNEAK: i32 = 1 << 5;
    pub const B_SPRINT: i32 = 1 << 6;
    pub const B_ATTACK: i32 = 1 << 7;
    pub const B_USE: i32 = 1 << 8;

    /// Clicks of the action `KeyMapping`s not yet consumed (`Bridge.consumeClicks`).
    pub const C_HOTBAR_0: i32 = 1;
    pub const C_DROP: i32 = 1 << 9;
    pub const C_SWAP_OFFHAND: i32 = 1 << 10;
    pub const C_JUMP: i32 = 1 << 11;
}

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
    salt: AtomicU64,
    /// Packets the UI itself decided to send (immediate respawn).
    outbox: Mutex<Vec<ServerboundPlay>>,
}

/// Clientbound play packets whose state lives in tier 3 (forwarded to Java whole).
const FORWARDED: &[&str] = &[
    "minecraft:system_chat",
    "minecraft:player_chat",
    "minecraft:disguised_chat",
    "minecraft:delete_chat",
    "minecraft:custom_chat_completions",
    "minecraft:set_title_text",
    "minecraft:set_subtitle_text",
    "minecraft:set_titles_animation",
    "minecraft:set_action_bar_text",
    "minecraft:clear_titles",
    "minecraft:boss_event",
    "minecraft:tab_list",
    "minecraft:player_info_update",
    "minecraft:player_info_remove",
    "minecraft:set_objective",
    "minecraft:set_score",
    "minecraft:reset_score",
    "minecraft:set_display_objective",
    "minecraft:set_player_team",
    "minecraft:commands",
    "minecraft:command_suggestions",
    "minecraft:update_advancements",
    "minecraft:select_advancements_tab",
    "minecraft:award_stats",
    "minecraft:game_rule_values",
    "minecraft:open_screen",
    "minecraft:update_recipes",
    "minecraft:container_set_content",
    "minecraft:container_set_slot",
    "minecraft:container_set_data",
    "minecraft:container_close",
    "minecraft:set_cursor_item",
    "minecraft:set_player_inventory",
    "minecraft:merchant_offers",
    "minecraft:update_tags",
    // the screens that edit block entities (signs, command blocks): the entity store and the
    // editors live in Java (see haste.gui.BlockEntities)
    "minecraft:block_entity_data",
    "minecraft:open_sign_editor",
    "minecraft:open_book",
    // the recipe book (real vanilla Java over the forwarded packets): the book's displays and
    // settings, the recipe container, and the ghost recipe the overlay shows on a right-click
    "minecraft:recipe_book_add",
    "minecraft:recipe_book_remove",
    "minecraft:recipe_book_settings",
    "minecraft:update_recipes",
    "minecraft:place_ghost_recipe",
];

/// Packets about one entity: forwarded only when they are about the local player.
const FORWARDED_LOCAL: &[&str] = &[
    "minecraft:set_entity_data",
    "minecraft:update_mob_effect",
    "minecraft:remove_mob_effect",
    "minecraft:update_attributes",
    "minecraft:player_combat_kill",
];

/// Whether [`Ui::handle`] uses this packet (tier-2 state or a forwarded tier-3 packet). The
/// HUD JVM boots for seconds while play packets already arrive; `NetDriver` keeps these packets
/// until the `Ui` exists so a fresh join's tab list, boss bars and scoreboard are not lost.
pub fn ui_handles(p: &ClientboundPlay) -> bool {
    use ClientboundPlay as P;
    matches!(
        p,
        P::SetHealth(_)
            | P::SetExperience(_)
            | P::PlayerAbilities(_)
            | P::GameEvent(_)
            | P::PlayerCombatKill(_)
            | P::Respawn(_)
            | P::SetHeldSlot(_)
            | P::EntityEvent(_)
            | P::SetTime(_)
    ) || p
        .name()
        .is_some_and(|n| FORWARDED.contains(&n) || FORWARDED_LOCAL.contains(&n))
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

    /// Opens a real vanilla screen (menu mode's title screen, tests).
    pub fn open_screen(&self, kind: ScreenKind) -> anyhow::Result<()> {
        self.gui().open_screen(kind)
    }

    /// The multiplayer screen's Join: the address to connect to, if any was clicked since the
    /// last call (menu mode; the connection itself is Rust's).
    pub fn take_join_request(&self) -> Option<String> {
        self.gui().take_join_request()
    }

    /// The connecting screen's Cancel since the last call: drop a join still in progress.
    pub fn take_join_cancelled(&self) -> bool {
        self.gui().take_join_cancelled()
    }

    /// The title screen's Quit Game since the last call: close the client.
    pub fn take_quit_request(&self) -> bool {
        self.gui().take_quit_request()
    }

    /// The world list's "Play Selected World" since the last call: the world folder to start
    /// the local server for and join.
    pub fn take_open_world_request(&self) -> Option<String> {
        self.gui().take_open_world_request()
    }

    /// The create screen's "Create New World" since the last call: the world to make.
    pub fn take_create_world_request(&self) -> Option<haste_jvm::CreateWorldRequest> {
        self.gui().take_create_world_request()
    }

    /// The renderer's text metrics (`Renderer::text_metrics`): vanilla's `Font` measures with them.
    pub fn set_text_metrics(&self, m: Arc<dyn TextMetrics>) {
        self.gui().set_text_metrics(m);
    }

    /// Joined a server (after `Connection::connect`): registries, tags and feature flags from the
    /// configuration phase, the local player from `login`.
    ///
    /// `game_mode` is the mode the client has actually seen (the session's, which follows every
    /// packet since the `login`). The HUD's JVM takes seconds to start, and packets that arrive
    /// before it (a server that switches a joining player's mode, a respawn with another mode)
    /// never reach [`Ui::handle`]; without the session's mode the `login` packet's would win and
    /// the HUD would show the mode of the world-loading screen (no hearts in survival, or hearts
    /// in creative). Where vanilla stores the local mode is `MultiPlayerGameMode.localMode`
    /// (`ClientPacketListener.handleGameEvent` -> `MultiPlayerGameMode.setLocalMode`); here the
    /// equivalent store is [`haste_jvm::GameState::game_mode`], which every game mode change
    /// writes: this call and [`Ui::handle`]'s `game_event` and respawn arms alike.
    pub fn joined(
        &self,
        conn: &Connection,
        login: &Login,
        game_mode: GameMode,
    ) -> anyhow::Result<()> {
        let tables = conn.tables().clone();
        *self.shared.serverbound.lock() = Some(Dispatch::new(
            &tables.packets,
            State::Play,
            Direction::Serverbound,
            ServerboundPlay::DECODERS,
        ));
        *self.shared.tables.lock() = Some(tables);
        let regs = conn.registries();
        for name in regs.names() {
            let entries = regs.entries(name).unwrap_or_default().to_vec();
            let p = RegistryData {
                registry: name.clone(),
                entries,
            };
            self.gui().registry_data(&body(&p))?;
        }
        if !regs.tags.is_empty() {
            let p = UpdateTags {
                registries: regs.tags.clone(),
            };
            self.gui().tags(&body(&p), false)?;
        }
        self.gui().finish_configuration()?;
        let features: Vec<String> = regs.features.iter().map(|f| f.to_string()).collect();
        self.gui().set_features(&features)?;
        // the pause menu's server links (b = built-in label id, c = custom label flattened to text)
        let links: Vec<String> = regs
            .server_links
            .iter()
            .map(|l| match &l.label {
                ServerLinkLabel::BuiltIn(id) => {
                    format!("b\t{id}\t{}", l.url)
                }
                ServerLinkLabel::Custom(c) => {
                    format!("c\t{}\t{}", c.to_plain(&|_| None), l.url)
                }
            })
            .collect();
        self.gui().set_server_links(&links)?;
        let entity = login.entity_id.0;
        self.shared.local_entity.store(entity, Ordering::Relaxed);
        self.gui()
            .set_player(entity, &login.profile.name, login.profile.uuid.as_u128())?;
        self.gui()
            .set_dimension_type(login.packet.spawn.dimension_type);
        self.gui().with_state(|s| {
            s.in_world = true;
            s.game_mode = game_mode;
            s.vitals.hardcore = login.hardcore;
            s.immediate_respawn = !login.packet.enable_respawn_screen;
            s.disconnect_requested = false;
        });
        Ok(())
    }

    /// One clientbound play packet (call for every packet, in order, on the game thread).
    pub fn handle(&self, p: &ClientboundPlay) {
        use ClientboundPlay as P;
        let local = self.shared.local_entity.load(Ordering::Relaxed);
        match p {
            P::SetHealth(h) => self.state(|s| {
                s.vitals.health = h.health;
                s.vitals.food = h.food;
                s.vitals.saturation = h.saturation;
            }),
            P::SetExperience(x) => self.state(|s| {
                s.vitals.xp_progress = x.bar;
                s.vitals.xp_level = x.level;
                s.vitals.xp_total = x.total;
            }),
            P::PlayerAbilities(a) => self.state(|s| {
                let f = a.flags;
                s.vitals.invulnerable = f & 1 != 0;
                s.vitals.flying = f & 2 != 0;
                s.vitals.may_fly = f & 4 != 0;
                s.vitals.instabuild = f & 8 != 0;
                s.vitals.fly_speed = a.flying_speed;
                s.vitals.walk_speed = a.fov_modifier;
            }),
            P::GameEvent(g) if g.event == 3 => {
                if let Some(m) = GameMode::from_id(g.value as i32) {
                    self.state(|s| s.game_mode = m);
                }
            }
            // 11: respawn screen on (0) or immediate respawn (1)
            P::GameEvent(g) if g.event == 11 => {
                self.state(|s| s.immediate_respawn = g.value == 1.0)
            }
            P::PlayerCombatKill(k)
                if k.player_id == local && self.state(|s| s.immediate_respawn) =>
            {
                // no death screen: respawn at once, as the vanilla client does
                self.shared
                    .outbox
                    .lock()
                    .push(ServerboundPlay::ClientCommand(sb::ClientCommand {
                        action: 0,
                    }));
            }
            P::Respawn(r) => {
                // a respawned player is a new entity at full health until the server says otherwise
                self.state(|s| s.vitals.health = 20.0);
                if let Some(m) = GameMode::from_id(r.spawn.game_mode) {
                    self.state(|s| s.game_mode = m);
                }
                self.gui().set_dimension_type(r.spawn.dimension_type);
                // the death screen (or any screen) closes when the player is back in the world
                if self.gui().is_open() {
                    self.gui().close_screen();
                }
            }
            P::SetHeldSlot(h) => {
                self.state(|s| s.selected_slot = h.slot);
                self.shared.sent_slot.store(h.slot, Ordering::Relaxed);
            }
            P::EntityEvent(e) if e.entity_id == local => {
                // 24..=28: operator permission level 0..=4
                if (24..=28).contains(&e.status) {
                    self.state(|s| s.permission_level = (e.status - 24) as u8);
                }
            }
            P::SetTime(t) => self.state(|s| {
                s.game_time = t.world_age;
                if let Some(c) = t.clocks.first() {
                    s.day_time = c.time;
                }
            }),
            _ => {}
        }
        let Some(name) = p.name() else { return };
        let local_only = FORWARDED_LOCAL.contains(&name);
        if !FORWARDED.contains(&name) && !local_only {
            return;
        }
        if local_only && entity_of(p) != Some(local) {
            return;
        }
        if name == "minecraft:update_tags" {
            if let Err(e) = self.gui().tags(&encode_body(p), true) {
                tracing::warn!("update_tags into Java: {e:#}");
            }
            return;
        }
        let Some(id) = self.packet_id(name) else {
            return;
        };
        let mut w = Writer::new();
        w.var_int(id);
        p.encode_body(&mut w);
        if let Err(e) = self.gui().packet(&w.into_inner()) {
            tracing::warn!("{name} into Java: {e:#}");
        }
    }

    fn packet_id(&self, name: &str) -> Option<i32> {
        self.shared
            .tables
            .lock()
            .as_ref()?
            .packets
            .id(State::Play, Direction::Clientbound, name)
    }

    fn state<T>(&self, f: impl FnOnce(&mut GameState) -> T) -> T {
        self.gui().with_state(f)
    }

    /// Whether any hotbar slot (vanilla `Inventory` 0-8) holds an item. The check scripts wait
    /// on this after the runner's RCON `give`: on a busy shared server the inventory sync can
    /// lag seconds behind the command's answer.
    pub fn hotbar_has_items(&self) -> bool {
        self.state(|s| s.inventory.iter().take(9).any(|s| !s.is_empty()))
    }

    /// One 20 Hz client tick (game thread).
    pub fn tick(&self) {
        self.gui().tick();
        if let Some(e) = self.gui().take_last_error() {
            tracing::warn!("ui tick: {e}");
        }
    }

    /// Everything the UI sends this tick, in order: packets vanilla code sent, chat and commands
    /// typed in the chat screen, the selected hotbar slot when it changed, and the respawn a
    /// dead player's immediate respawn queued (`PlayerCombatKill`).
    pub fn drain_outgoing(&self, out: &mut Vec<ServerboundPlay>) {
        out.append(&mut self.shared.outbox.lock());
        let (packets, chat, slot) =
            self.state(|s| (s.take_outgoing(), s.take_sent(), s.selected_slot));
        if !packets.is_empty() {
            let tables = self.shared.tables.lock().clone();
            let dispatch = self.shared.serverbound.lock();
            if let (Some(tables), Some(dispatch)) = (tables, dispatch.as_ref()) {
                for p in packets {
                    match decode_serverbound(dispatch, &tables, &p) {
                        Ok(sp) => {
                            if let ServerboundPlay::ContainerClick(c) = &sp {
                                tracing::info!(
                                    "container_click: window {} slot {} button {} mode {} state {} changed {} carried {}",
                                    c.window_id,
                                    c.slot,
                                    c.button,
                                    c.mode,
                                    c.state_id,
                                    c.changed.len(),
                                    c.carried.item.is_some()
                                );
                            }
                            out.push(sp)
                        }
                        Err(e) => tracing::warn!("packet from Java not sent: {e}"),
                    }
                }
            }
        }
        for m in chat {
            out.push(if m.command {
                ServerboundPlay::ChatCommand(sb::ChatCommand { command: m.text })
            } else {
                ServerboundPlay::Chat(sb::Chat {
                    message: m.text,
                    timestamp: now_millis(),
                    salt: self.next_salt(),
                    signature: None,
                    message_count: 0,
                    acknowledged: [0; 3],
                    checksum: 0,
                })
            });
        }
        if self.shared.sent_slot.swap(slot, Ordering::Relaxed) != slot {
            out.push(ServerboundPlay::SetCarriedItem(sb::SetCarriedItem {
                slot: slot as i16,
            }));
        }
    }

    /// The option changes vanilla screens made since the last call, as options.txt key/value
    /// pairs, oldest first. The client applies each to its live options (FOV, render distance,
    /// volumes, key binds, ...); `haste.gui.OptionBridge` reports them from vanilla's own
    /// `OptionInstance.onValueUpdate` callbacks.
    pub fn drain_option_changes(&self) -> Vec<(String, String)> {
        self.state(|s| s.take_option_changes())
    }

    /// The inventory slots vanilla Java code changed since the last call (vanilla
    /// `Inventory` numbering, oldest first): creative slot sets, menu clicks, pick block —
    /// edits vanilla predicts client-side, which the server does not echo back (vanilla's
    /// `handleSetCreativeModeSlot` marks the slot remote before `broadcastChanges`, so no
    /// `container_set_slot`/`set_player_inventory` arrives for it).
    pub fn drain_inventory_changes(&self) -> Vec<(i32, haste_core::entity::ItemStackData)> {
        self.state(|s| s.take_inventory_changes())
    }

    /// The resource pack selection the pack screen committed since the last call (vanilla
    /// `Minecraft.reloadResourcePacks` after Done): repository ids, bottom first.
    pub fn take_resource_pack_selection(&self) -> Option<Vec<String>> {
        self.gui().take_resource_pack_ids()
    }

    /// A game key event (no screen open, SDL3 scancode) mirrored into the vanilla `KeyMapping`s,
    /// vanilla's `KeyboardHandler.keyPress` (press sets the down state and counts a click).
    pub fn game_key(&self, scancode: i32, pressed: bool) {
        self.gui().game_key(scancode, pressed);
    }

    /// A game mouse button (no screen open, vanilla numbering) mirrored into the vanilla
    /// `KeyMapping`s, vanilla's `MouseHandler.onPress`.
    pub fn game_mouse(&self, button: i32, pressed: bool) {
        self.gui().game_mouse(button, pressed);
    }

    /// The down state of the movement and action `KeyMapping`s as a [`bits`] mask: the controls
    /// the client reads are the player's own bindings.
    pub fn game_input(&self) -> i32 {
        self.gui().game_input()
    }

    /// Consumes the action `KeyMapping` clicks (hotbar, drop, swap offhand, jump) since the last
    /// call as a [`bits`] mask, one bit per click.
    pub fn consume_clicks(&self) -> i32 {
        self.gui().consume_clicks()
    }

    fn next_salt(&self) -> i64 {
        // SplitMix64 over a counter: unpredictable enough for an unsigned message's salt
        let mut z = self
            .shared
            .salt
            .fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed)
            .wrapping_add(now_millis() as u64);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        (z ^ (z >> 31)) as i64
    }

    /// A key event (26.3 numbering, see [`keys`]): `key` is the SDL scancode, `keycode` the SDL
    /// keycode, `mods` SDL keymod bits. Returns true when the UI used it (the client must not also
    /// treat it as game input).
    pub fn key(&self, key: i32, keycode: i32, mods: i32, pressed: bool) -> bool {
        let gui = self.gui();
        if gui.is_open() {
            gui.key(key, keycode, mods, pressed);
            return true;
        }
        // No screen: vanilla's `KeyboardHandler.keyPress` sets the `KeyMapping`s (`handlesGameInput`),
        // including the HUD's own keys, which are KeyMappings in vanilla too. The client reads the
        // movement and action mappings back through `game_input`/`consume_clicks`.
        gui.game_key(key, pressed);
        if key == keys::TAB {
            self.shared.tab_down.store(pressed, Ordering::Relaxed);
            gui.set_tab_list_visible(pressed);
            return true;
        }
        if !pressed {
            return false;
        }
        let open = |kind| {
            if let Err(e) = gui.open_screen(kind) {
                tracing::warn!("opening {kind:?}: {e:#}");
            }
            true
        };
        match key {
            k if k == keys::T => open(ScreenKind::Chat),
            k if k == keys::SLASH => open(ScreenKind::Command),
            k if k == keys::E => open(ScreenKind::Inventory),
            k if k == keys::ESCAPE => open(ScreenKind::Pause),
            k if k == keys::F1 => {
                gui.toggle_hud();
                true
            }
            k if k == keys::F3 => {
                gui.toggle_debug();
                true
            }
            k if (keys::NUM_1..keys::NUM_1 + 9).contains(&k) => {
                self.state(|s| s.selected_slot = k - keys::NUM_1);
                true
            }
            _ => false,
        }
    }

    /// A typed character (to the open screen's focused text field).
    pub fn char_typed(&self, c: char, mods: i32) -> bool {
        self.gui().is_open() && self.gui().char_typed(c, mods)
    }

    /// The mouse moved (GUI-scaled coordinates). True when a screen is open.
    pub fn mouse_moved(&self, x: f64, y: f64) -> bool {
        if !self.gui().is_open() {
            return false;
        }
        self.gui().mouse(x, y, None);
        true
    }

    /// A mouse button (vanilla numbering: 1 left, 2 middle, 3 right) at (x, y) in GUI pixels.
    pub fn mouse_button(&self, x: f64, y: f64, button: i32, pressed: bool) -> bool {
        let gui = self.gui();
        if !gui.is_open() {
            // No screen: vanilla's `MouseHandler.onPress` sets the mouse `KeyMapping`s.
            gui.game_mouse(button, pressed);
            return false;
        }
        gui.mouse(x, y, Some((button, pressed)));
        true
    }

    /// The scroll wheel: to the open screen, else the hotbar selection (one slot per notch, like
    /// vanilla; wheel down moves right).
    pub fn scroll(&self, x: f64, y: f64, dx: f64, dy: f64) -> bool {
        if self.gui().is_open() {
            self.gui().scroll(x, y, dx, dy);
            return true;
        }
        if dy == 0.0 {
            return false;
        }
        let step = if dy > 0.0 { -1 } else { 1 };
        self.state(|s| s.selected_slot = (s.selected_slot + step).rem_euclid(9));
        true
    }

    /// The GUI frame: the HUD (once joined) and the open screen, GUI-scaled size `width` x
    /// `height` at `scale`, the mouse at the last position given to [`Ui::mouse_moved`].
    pub fn frame(
        &mut self,
        width: u32,
        height: u32,
        scale: u32,
        mouse: (f64, f64),
        partial: f32,
    ) -> &GuiFrame {
        match self.gui().render(width, height, scale, mouse, partial) {
            Ok(f) => {
                self.trace_vitals(&f);
                self.frame = f;
            }
            Err(e) => {
                tracing::warn!("ui frame: {e:#}");
                self.frame.commands.clear();
            }
        }
        &self.frame
    }

    /// What the HUD drew of the player's vitals, at debug level (the frame path stays clean
    /// when debug logging is off): the game mode the HUD renders for and the heart and food
    /// sprite counts, so a "no health, no hunger" report can be traced to the side that has
    /// them wrong — the state, or the Java HUD that reads it (`Hud.extractHotbarAndDecorations`
    /// draws them only while `gameMode.canHurtPlayer()`).
    fn trace_vitals(&self, f: &GuiFrame) {
        if !tracing::enabled!(tracing::Level::DEBUG) {
            return;
        }
        let (hearts, food) = f.commands.iter().fold((0usize, 0usize), |(h, fd), c| {
            if let GuiCommand::Sprite { sprite, .. } | GuiCommand::SpriteRegion { sprite, .. } = c {
                let path = sprite.path();
                (
                    h + usize::from(path.starts_with("hud/heart")),
                    fd + usize::from(path.starts_with("hud/food")),
                )
            } else {
                (h, fd)
            }
        });
        let mode = self.state(|s| s.game_mode);
        tracing::debug!("hud frame: {mode:?} hearts {hearts} food {food}");
    }

    /// Whether a screen is open (the client releases the mouse and stops game input).
    pub fn screen_open(&self) -> bool {
        self.gui().is_open()
    }

    /// Whether the F3 debug overlay is on (its strings are only built then).
    pub fn debug_shown(&self) -> bool {
        self.gui().debug_shown()
    }

    /// Whether vanilla code wants the mouse grabbed (e.g. after "Back to Game").
    pub fn wants_mouse_grabbed(&self) -> bool {
        self.gui().mouse_grabbed()
    }

    /// The client grabbed or released the mouse itself.
    pub fn set_mouse_grabbed(&self, grabbed: bool) {
        self.gui().set_mouse_grabbed(grabbed);
    }

    /// The selected hotbar slot (0-8): the HUD's and the server's, kept here.
    pub fn selected_slot(&self) -> i32 {
        self.state(|s| s.selected_slot)
    }

    /// The client selected a hotbar slot itself (1-9, the wheel): the HUD shows it from here.
    /// The client's own `set_carried_item` tells the server, so `sent_slot` is set with it (the
    /// UI must not send it a second time).
    pub fn set_selected_slot(&self, slot: i32) {
        self.state(|s| s.selected_slot = slot);
        self.shared.sent_slot.store(slot, Ordering::Relaxed);
    }

    /// Whether vanilla code asked to leave the server (pause screen "Disconnect"); clears it.
    pub fn take_disconnect_request(&self) -> bool {
        self.state(|s| std::mem::take(&mut s.disconnect_requested))
    }

    /// Whether the player may use game-master blocks (vanilla
    /// `Player.canUseGameMasterBlocks`: creative and operator level >= 2).
    pub fn game_master_state(&self) -> (bool, u8) {
        self.state(|s| (s.game_mode == GameMode::Creative, s.permission_level))
    }

    /// A use opened the command block editor (vanilla `LocalPlayer.openCommandBlock`).
    pub fn open_used_command_block(&self, pos: haste_protocol::haste_core::BlockPos) {
        if let Err(e) = self.gui().open_command_block(pos) {
            tracing::warn!("opening the command block editor: {e:#}");
        }
    }

    /// A use opened the writable book's editor (vanilla `LocalPlayer.openItemGui`); `hand` is
    /// `InteractionHand.values()` numbering (0 main, 1 off).
    pub fn open_item_gui(&self, hand: i32) {
        if let Err(e) = self.gui().open_item_gui(hand) {
            tracing::warn!("opening the book editor: {e:#}");
        }
    }

    /// The connection ended: vanilla's disconnect screen with the reason; the HUD stops.
    pub fn disconnected(&self, reason: &Component) {
        self.state(|s| s.in_world = false);
        if let Err(e) = self.gui().disconnected(reason) {
            tracing::warn!("disconnect screen: {e:#}");
        }
    }

    /// The client left the world (pause screen "Disconnect", death screen "Title Screen"):
    /// the HUD stops drawing (`Hud.extractRenderState` runs only while
    /// [`GameState::in_world`] is set, like vanilla's `Gui` once its level is null).
    pub fn left_world(&self) {
        self.state(|s| s.in_world = false);
    }

    /// The F3 overlay's numbers (FPS, position, rotation, chunk and entity counts), every frame.
    pub fn set_debug(&self, d: haste_jvm::DebugInfo) {
        self.state(|s| s.debug = d);
    }

    /// The player's eyes are in water (air bubbles and the water overlay), from physics.
    pub fn set_eye_in_water(&self, v: bool) {
        self.state(|s| s.vitals.eye_in_water = v);
    }
}

/// The entity a single-entity packet is about.
fn entity_of(p: &ClientboundPlay) -> Option<i32> {
    use ClientboundPlay as P;
    Some(match p {
        P::SetEntityData(x) => x.entity_id,
        P::UpdateMobEffect(x) => x.entity_id,
        P::RemoveMobEffect(x) => x.entity_id,
        P::UpdateAttributes(x) => x.entity_id,
        P::PlayerCombatKill(x) => x.player_id,
        _ => return None,
    })
}

fn body<P: Packet>(p: &P) -> Vec<u8> {
    let mut w = Writer::new();
    p.encode(&mut w);
    w.into_inner()
}

fn encode_body(p: &ClientboundPlay) -> Vec<u8> {
    let mut w = Writer::new();
    p.encode_body(&mut w);
    w.into_inner()
}

/// Decodes a serverbound packet vanilla code encoded (VarInt id + body).
fn decode_serverbound(
    dispatch: &Dispatch<ServerboundPlay>,
    tables: &ProtocolTables,
    p: &[u8],
) -> Result<ServerboundPlay, String> {
    let mut i = 0;
    let id = read_var_int(p, &mut i).ok_or("truncated packet id")?;
    let body = Vec::from(&p[i..]).into();
    match dispatch.decode(id, &body, Some(tables)) {
        Decoded::Ok(sp) => Ok(sp),
        Decoded::NoDecoder => Ok(ServerboundPlay::Unknown { id, body }),
        Decoded::Failed(name, e) => Err(format!("{name}: {e}")),
    }
}

fn read_var_int(b: &[u8], i: &mut usize) -> Option<i32> {
    let mut v: u32 = 0;
    for shift in (0..35).step_by(7) {
        let x = *b.get(*i)?;
        *i += 1;
        v |= u32::from(x & 0x7f) << shift;
        if x & 0x80 == 0 {
            return Some(v as i32);
        }
    }
    None
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn var_ints() {
        let mut i = 0;
        assert_eq!(read_var_int(&[0xAC, 0x02, 7], &mut i), Some(300));
        assert_eq!(i, 2);
        let mut i = 0;
        assert_eq!(read_var_int(&[0x80], &mut i), None);
    }

    #[test]
    fn forwarding_lists_are_disjoint_and_named_like_packets_json() {
        for n in FORWARDED.iter().chain(FORWARDED_LOCAL) {
            assert!(n.starts_with("minecraft:"), "{n}");
        }
        for n in FORWARDED_LOCAL {
            assert!(!FORWARDED.contains(n), "{n}");
        }
    }

    #[test]
    fn ui_handles_tier2_state_and_forwarded_packets_only() {
        use haste_protocol::packets::play::clientbound as cb;
        // Tier-2 state (health) and a forwarded tier-3 packet (the sidebar display) are kept for
        // the HUD; a plain world packet is not (the boot backlog must stay small).
        let health = ClientboundPlay::SetHealth(cb::SetHealth {
            health: 1.0,
            food: 2,
            saturation: 0.0,
        });
        assert!(ui_handles(&health));
        let display = ClientboundPlay::SetDisplayObjective(cb::SetDisplayObjective {
            position: 1,
            name: "s".to_owned(),
        });
        assert!(ui_handles(&display));
        let block = ClientboundPlay::BlockUpdate(cb::BlockUpdate {
            pos: haste_protocol::haste_core::BlockPos::new(0, 0, 0),
            state: 1,
        });
        assert!(!ui_handles(&block));
    }

    // ---- tests against a server (ignored: need cache/ and the test server) ----

    use haste_protocol::haste_core::gui::GuiCommand;
    use haste_protocol::haste_core::DVec3;

    const NEEDS_SERVER: &str = "needs cache/ and a test server (HASTE_TEST_SERVER, default 127.0.0.1:25614) where 'haste' is an operator; run with --test-threads=1";

    /// The process's JVM (one per process), started once.
    fn host() -> JvmHost {
        static HOST: std::sync::OnceLock<JvmHost> = std::sync::OnceLock::new();
        HOST.get_or_init(|| {
            let cache = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cache");
            let t = std::time::Instant::now();
            let host = JvmHost::start(haste_jvm::JvmConfig::from_cache(&cache).unwrap()).unwrap();
            eprintln!("JVM ready in {:?}: {}", t.elapsed(), host.proxy_summary());
            host
        })
        .clone()
    }

    /// A minimal client for the tests: joins, answers teleports, sends the per-tick movement and
    /// tick-end packets (standing still), drives `Ui` like the game loop does.
    struct TestClient {
        rt: tokio::runtime::Runtime,
        conn: Option<Connection>,
        ui: Ui,
        pos: Option<(DVec3, f32, f32)>,
        loaded: bool,
        tick: u64,
        received: Vec<&'static str>,
        sent: Vec<ServerboundPlay>,
        kicked: Option<Component>,
    }

    impl TestClient {
        fn join() -> TestClient {
            use haste_protocol::{Account, ConnectOptions};
            let addr =
                std::env::var("HASTE_TEST_SERVER").unwrap_or_else(|_| "127.0.0.1:25614".into());
            let ui = Ui::new(host());
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .build()
                .unwrap();
            let opts = ConnectOptions::new(
                addr.clone(),
                Account::Offline {
                    username: "haste".into(),
                },
            );
            let (conn, login) = rt.block_on(Connection::connect(opts)).expect("connect");
            let t = std::time::Instant::now();
            ui.joined(&conn, &login, login.game_mode)
                .expect("registries into Java");
            eprintln!(
                "joined {addr}; {} synchronised registries into Java in {:?}",
                conn.registries().names().count(),
                t.elapsed()
            );
            TestClient {
                rt,
                conn: Some(conn),
                ui,
                pos: None,
                loaded: false,
                tick: 0,
                received: Vec::new(),
                sent: Vec::new(),
                kicked: None,
            }
        }

        fn conn(&mut self) -> &mut Connection {
            self.conn.as_mut().expect("connected")
        }

        fn send(&mut self, p: ServerboundPlay) {
            self.conn().send(p).unwrap();
        }

        /// `n` game ticks (50 ms each).
        fn run(&mut self, n: u64) {
            let _rt = self.rt.enter();
            for _ in 0..n {
                while let Some(p) = self.conn.as_mut().and_then(|c| c.try_recv()) {
                    self.ui.handle(&p);
                    if let Some(n) = p.name() {
                        self.received.push(n);
                    }
                    match &p {
                        ClientboundPlay::PlayerPosition(t) => {
                            self.pos = Some((t.pos, t.yaw, t.pitch));
                            self.send(ServerboundPlay::AcceptTeleportation(
                                sb::AcceptTeleportation {
                                    teleport_id: t.teleport_id,
                                    pos: t.pos,
                                    yaw: t.yaw,
                                    pitch: t.pitch,
                                },
                            ));
                            if !self.loaded {
                                self.send(ServerboundPlay::PlayerLoaded(sb::PlayerLoaded {}));
                                self.loaded = true;
                            }
                        }
                        ClientboundPlay::Disconnect(d) => {
                            self.ui.disconnected(&d.reason);
                            self.kicked = Some(d.reason.clone());
                            self.conn.take();
                            return;
                        }
                        _ => {}
                    }
                }
                if let (true, Some((p, yaw, pitch))) = (self.loaded, self.pos) {
                    self.send(ServerboundPlay::MovePlayerPosRot(sb::MovePlayerPosRot {
                        pos: p,
                        yaw,
                        pitch,
                        flags: sb::move_flags::ON_GROUND,
                    }));
                    self.send(ServerboundPlay::ClientTickEnd(sb::ClientTickEnd {}));
                }
                self.ui.tick();
                let mut out = Vec::new();
                self.ui.drain_outgoing(&mut out);
                for p in out {
                    self.sent.push(p.clone());
                    self.send(p);
                }
                self.tick += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }

        /// Runs until the player is loaded in the world.
        fn spawn(&mut self) {
            for _ in 0..200 {
                if self.loaded {
                    self.run(10);
                    return;
                }
                self.run(1);
            }
            panic!("no spawn teleport");
        }

        /// A command as the chat screen would send it (setup steps of the tests).
        fn command(&mut self, c: &str) {
            self.ui.gui().with_state(|s| {
                s.sent.push(haste_jvm::ChatMessage {
                    text: c.into(),
                    command: true,
                })
            });
        }

        fn frame(&mut self, mouse: (f64, f64)) -> haste_protocol::haste_core::gui::GuiFrame {
            self.ui.frame(320, 240, 2, mouse, 0.0).clone()
        }

        fn press(&mut self, key: i32) {
            let kc = keys::keycode(key);
            self.ui.key(key, kc, 0, true);
            self.ui.key(key, kc, 0, false);
        }

        fn type_text(&mut self, t: &str) {
            for c in t.chars() {
                self.ui.char_typed(c, 0);
            }
        }

        /// Clicks the button whose label is `label` (the widget sprite under the text).
        fn click(&mut self, label: &str) {
            let f = self.frame((0.0, 0.0));
            let (x, y) =
                text_pos(&f, label).unwrap_or_else(|| panic!("no {label:?} in {:?}", texts(&f)));
            self.ui.mouse_moved(f64::from(x) + 1.0, f64::from(y) + 1.0);
            self.ui.mouse_button(
                f64::from(x) + 1.0,
                f64::from(y) + 1.0,
                keys::MOUSE_LEFT,
                true,
            );
            self.ui.mouse_button(
                f64::from(x) + 1.0,
                f64::from(y) + 1.0,
                keys::MOUSE_LEFT,
                false,
            );
        }

        fn disconnect(mut self) {
            if let Some(c) = self.conn.take() {
                c.disconnect();
            }
            self.ui.gui().with_state(|s| s.in_world = false);
            self.ui.gui().close_screen();
        }
    }

    /// Vanilla code that hits a refused class fails, and inside a `CompletableFuture` it fails
    /// silently: every test checks the class log. These refusals are known and harmless.
    fn assert_no_refusals(host: &JvmHost) {
        const KNOWN: &[&str] = &[
            "com.mojang.renderpearl.api.pipeline.RenderPipeline", // a parameter type of draw calls
            "jakarta.servlet.Servlet",                            // log4j probes
            "javax.servlet.Servlet",
            "net.minecraft.client.sounds.MusicManager", // option callbacks (no music)
        ];
        let log = host.class_log().unwrap();
        let bad: Vec<_> = log
            .refused()
            .into_iter()
            .filter(|(n, _)| !KNOWN.contains(n))
            .collect();
        assert!(bad.is_empty(), "classes refused: {bad:#?}");
    }

    fn texts(f: &haste_protocol::haste_core::gui::GuiFrame) -> Vec<String> {
        f.commands
            .iter()
            .filter_map(|c| match c {
                GuiCommand::Text { text, .. } | GuiCommand::StyledText { text, .. } => {
                    Some(text.clone())
                }
                _ => None,
            })
            .collect()
    }

    /// Where a text is drawn, in absolute GUI pixels (translations applied).
    fn text_pos(f: &haste_protocol::haste_core::gui::GuiFrame, want: &str) -> Option<(i32, i32)> {
        let mut stack: Vec<(f32, f32)> = vec![(0.0, 0.0)];
        for c in &f.commands {
            match c {
                GuiCommand::TranslatePush { dx, dy, .. } => {
                    let (x, y) = *stack.last().unwrap();
                    stack.push((x + dx, y + dy));
                }
                GuiCommand::TranslatePop => {
                    stack.pop();
                }
                GuiCommand::Text { text, x, y, .. } | GuiCommand::StyledText { text, x, y, .. }
                    if text == want =>
                {
                    let (dx, dy) = *stack.last().unwrap();
                    return Some((x + dx as i32, y + dy as i32));
                }
                _ => {}
            }
        }
        None
    }

    /// Joins the test server as "haste", gives itself stone and says hello through commands (the
    /// player must be an operator: `ops.json` of the test server), plays a few seconds and renders
    /// the HUD: the hotbar with the stone and the chat line must be in the frame.
    #[test]
    #[ignore = "needs cache/ and a test server (HASTE_TEST_SERVER, default 127.0.0.1:25614) where 'haste' is an operator; run with --test-threads=1"]
    fn joins_and_renders_the_hud() {
        let mut c = TestClient::join();
        c.spawn();
        c.command("clear @s");
        c.command("give @s minecraft:stone 64");
        c.command("say hello from haste");
        c.run(40);
        assert!(c.kicked.is_none(), "kicked: {:?}", c.kicked);
        eprintln!(
            "tier-3 packets seen: {:?}",
            c.received
                .iter()
                .filter(|n| FORWARDED.contains(n) || FORWARDED_LOCAL.contains(n))
                .collect::<std::collections::BTreeSet<_>>()
        );
        let stone =
            c.ui.gui()
                .with_state(|s| s.slot(0).cloned())
                .expect("stone in hotbar slot 0 (from the server's container_set_slot)");
        assert_eq!(stone.count, 64);
        let t = std::time::Instant::now();
        let f = c.frame((0.0, 0.0));
        eprintln!(
            "HUD frame in {:?}: {} commands",
            t.elapsed(),
            f.commands.len()
        );
        for cmd in &f.commands {
            eprintln!("  {cmd:?}");
        }
        assert!(f.commands.iter().any(
            |c| matches!(c, GuiCommand::Sprite { sprite, .. } if sprite.path() == "hud/hotbar")
        ));
        assert!(f.commands.iter().any(|c| matches!(c,
            GuiCommand::Item { item, count: 64, x: 72, y: 221 } if Some(*item) == stone.item)));
        let text = texts(&f);
        assert!(
            text.iter().any(|t| t.contains("hello from haste")),
            "chat line in the HUD: {text:?}"
        );
        let chat = c.ui.gui().chat_history().unwrap();
        eprintln!("chat log: {chat:?}");
        assert!(
            chat.iter().any(|l| l == "[haste] hello from haste"),
            "{chat:?}"
        );
        let unimpl = c.ui.host.unimplemented().unwrap();
        eprintln!("Unimplemented: {unimpl:#?}");
        assert_no_refusals(&c.ui.host);
        c.disconnect();
    }

    /// The chat screen parses commands with the server's command tree (from its `commands`
    /// packet) and asks the server for suggestions where the tree says so (`command_suggestion`
    /// round trip, here the recipe names of `/recipe give <player> `).
    #[test]
    #[ignore = "needs cache/ and a test server (HASTE_TEST_SERVER, default 127.0.0.1:25614) where 'haste' is an operator; run with --test-threads=1"]
    fn command_suggestions_from_the_server() {
        let mut c = TestClient::join();
        c.spawn();
        assert!(c.received.contains(&"minecraft:commands"));
        let names = c.ui.gui().command_names().unwrap();
        eprintln!(
            "permission level {}, {} commands: {names:?}",
            c.ui.gui().with_state(|s| s.permission_level),
            names.len()
        );
        assert!(names.iter().any(|n| n == "gamemode"), "{names:?}");
        let comp = c.ui.gui().completions("gamem").unwrap();
        eprintln!(
            "completions of 'gamem': {comp:?}; autoSuggestions {:?}",
            c.ui.gui().option("autoSuggestions")
        );
        // "/" opens the chat screen with "/" typed
        c.press(keys::SLASH);
        assert_eq!(
            c.ui.gui().screen_class().as_deref(),
            Some("net.minecraft.client.gui.screens.ChatScreen")
        );
        c.type_text("gamem");
        c.run(2);
        let f = c.frame((0.0, 0.0));
        eprintln!(
            "unimplemented: {:#?}; last error {:?}",
            c.ui.host.unimplemented().unwrap(),
            c.ui.gui().take_last_error()
        );
        let t = texts(&f);
        assert!(
            t.iter().any(|x| x == "gamemode"),
            "client-side suggestions: {t:?}"
        );
        // an argument the server completes: the screen sends command_suggestion, the answer shows
        for _ in 0.."gamem".len() {
            c.press(keys::BACKSPACE);
        }
        c.type_text("recipe give haste ");
        c.run(20);
        assert!(
            c.sent.iter().any(|p| matches!(p, ServerboundPlay::CommandSuggestion(s) if s.text.starts_with("/recipe give haste"))),
            "{:?}",
            c.sent
        );
        assert!(c.received.contains(&"minecraft:command_suggestions"));
        let t = texts(&c.frame((0.0, 0.0)));
        assert!(
            t.iter().any(|x| x.starts_with("minecraft:")),
            "server suggestions: {t:?}"
        );
        // like vanilla: the first Escape hides the suggestion list, the second closes the screen
        c.press(keys::ESCAPE);
        assert!(c.ui.screen_open());
        c.press(keys::ESCAPE);
        assert!(!c.ui.screen_open());
        assert!(c.kicked.is_none(), "kicked: {:?}", c.kicked);
        assert_no_refusals(&c.ui.host);
        c.disconnect();
    }

    /// The server's `game_event` CHANGE_GAME_MODE (an operator's `/gamemode`) reaches the Java
    /// side where vanilla stores the local mode (`MultiPlayerGameMode.setLocalMode`): the HUD
    /// shows the hearts and the hunger bar in survival and drops them again in creative.
    #[test]
    #[ignore = "needs cache/ (JDK, client jar, libraries, ASM, data generator reports)"]
    fn game_mode_change_shows_and_hides_the_vitals() {
        use haste_protocol::packets::play::clientbound as cb;
        let mut ui = Ui::new(host());
        ui.gui().with_state(|s| {
            s.in_world = true;
            s.game_mode = GameMode::Creative;
            s.vitals.health = 20.0;
            s.vitals.food = 20;
        });
        let vitals = |ui: &mut Ui| {
            let f = ui.frame(320, 240, 2, (0.0, 0.0), 0.0);
            let sprites: Vec<String> =
                f.commands
                    .iter()
                    .filter_map(|c| match c {
                        GuiCommand::Sprite { sprite, .. }
                        | GuiCommand::SpriteRegion { sprite, .. } => Some(sprite.to_string()),
                        _ => None,
                    })
                    .collect();
            let count = |prefix: &str| sprites.iter().filter(|s| s.starts_with(prefix)).count();
            (count("minecraft:hud/heart"), count("minecraft:hud/food"))
        };
        // creative: no health, no hunger
        assert_eq!(vitals(&mut ui), (0, 0));
        // CHANGE_GAME_MODE (event 3), the param is the GameType id: 0 = survival
        ui.handle(&ClientboundPlay::GameEvent(cb::GameEvent {
            event: 3,
            value: 0.0,
        }));
        assert_eq!(ui.gui().with_state(|s| s.game_mode), GameMode::Survival);
        let (hearts, food) = vitals(&mut ui);
        assert!(hearts >= 10, "hearts: {hearts}");
        assert!(food >= 10, "food: {food}");
        // and back to creative (1): the hearts and the hunger bar go away again
        ui.handle(&ClientboundPlay::GameEvent(cb::GameEvent {
            event: 3,
            value: 1.0,
        }));
        assert_eq!(vitals(&mut ui), (0, 0));
        // the respawn packet carries the game mode too (a respawn in another mode)
        ui.handle(&ClientboundPlay::Respawn(cb::Respawn {
            spawn: cb::SpawnInfo {
                dimension_type: 0,
                dimension: "minecraft:overworld".parse().unwrap(),
                hashed_seed: 0,
                game_mode: 0,
                previous_game_mode: 0,
                is_debug: false,
                is_flat: false,
                death_location: None,
                portal_cooldown: 0,
                sea_level: 63,
            },
            data_kept: 0,
        }));
        let (hearts, food) = vitals(&mut ui);
        assert!(hearts >= 10, "hearts after the respawn: {hearts}");
        assert!(food >= 10, "food after the respawn: {food}");
    }

    /// Dying opens vanilla's death screen; its Respawn button sends `client_command` and the
    /// server's respawn closes it.
    #[test]
    #[ignore = "needs cache/ and a test server (HASTE_TEST_SERVER, default 127.0.0.1:25614) where 'haste' is an operator; run with --test-threads=1"]
    fn death_screen_and_respawn() {
        let mut c = TestClient::join();
        c.spawn();
        if c.ui.gui().with_state(|s| s.vitals.health <= 0.0) {
            // left dead by an earlier run: respawn first
            c.send(ServerboundPlay::ClientCommand(sb::ClientCommand {
                action: 0,
            }));
            c.run(20);
        }
        c.command("kill @s");
        c.run(10);
        assert_eq!(
            c.ui.gui().screen_class().as_deref(),
            Some("net.minecraft.client.gui.screens.DeathScreen"),
            "received {:?}",
            c.received
        );
        let t = texts(&c.frame((0.0, 0.0)));
        assert!(t.iter().any(|x| x == "You Died!"), "{t:?}");
        // the buttons are enabled after a second
        c.run(25);
        c.click("Respawn");
        c.run(10);
        assert!(
            c.sent
                .iter()
                .any(|p| matches!(p, ServerboundPlay::ClientCommand(x) if x.action == 0)),
            "{:?}",
            c.sent
        );
        assert!(c.received.contains(&"minecraft:respawn"));
        assert!(!c.ui.screen_open(), "{:?}", c.ui.gui().screen_class());
        assert!(c.kicked.is_none(), "kicked: {:?}", c.kicked);
        let unimpl = c.ui.host.unimplemented().unwrap();
        eprintln!("Unimplemented: {unimpl:#?}");
        assert_no_refusals(&c.ui.host);
        c.disconnect();
    }

    /// A chest the player opens is vanilla's container screen over the server's container
    /// packets; clicks are predicted and sent with hashed stacks, and the server agrees (no
    /// correction of the clicked slots), Escape closes it with `container_close`.
    #[test]
    #[ignore = "needs cache/ and a test server (HASTE_TEST_SERVER, default 127.0.0.1:25614) where 'haste' is an operator; run with --test-threads=1"]
    fn chest_screen() {
        let mut c = TestClient::join();
        c.spawn();
        let (p, _, _) = c.pos.unwrap();
        let chest = haste_protocol::haste_core::BlockPos::new(
            p.x.floor() as i32 + 2,
            p.y.floor() as i32,
            p.z.floor() as i32,
        );
        c.command(&format!(
            "setblock {} {} {} minecraft:air",
            chest.x, chest.y, chest.z
        ));
        c.command(&format!(
            "setblock {} {} {} minecraft:chest{{Items:[{{Slot:0b,id:\"minecraft:diamond\",count:5}}]}}",
            chest.x, chest.y, chest.z
        ));
        c.run(10);
        c.send(ServerboundPlay::UseItemOn(sb::UseItemOn {
            hand: 0,
            pos: chest,
            face: 4, // west face (the player is to the west)
            cursor_x: 0.0,
            cursor_y: 0.5,
            cursor_z: 0.5,
            inside_block: false,
            world_border_hit: false,
            sequence: 1,
        }));
        c.run(10);
        assert!(
            c.received.contains(&"minecraft:open_screen"),
            "{:?}",
            c.received
        );
        assert_eq!(
            c.ui.gui().screen_class().as_deref(),
            Some("net.minecraft.client.gui.screens.inventory.ContainerScreen")
        );
        let f = c.frame((0.0, 0.0));
        let t = texts(&f);
        assert!(t.iter().any(|x| x == "Chest"), "{t:?}");
        assert!(t.iter().any(|x| x == "5"), "the diamonds' count: {t:?}");
        // pick up the diamonds: slot 0 of the chest, at (8, 18) in the 176x166 container
        let origin = ((320 - 176) / 2, (240 - 166) / 2);
        let at = (f64::from(origin.0 + 8 + 8), f64::from(origin.1 + 18 + 8));
        let corrections_before = c
            .received
            .iter()
            .filter(|n| **n == "minecraft:container_set_slot")
            .count();
        c.ui.mouse_moved(at.0, at.1);
        c.ui.mouse_button(at.0, at.1, keys::MOUSE_LEFT, true);
        c.ui.mouse_button(at.0, at.1, keys::MOUSE_LEFT, false);
        c.run(10);
        let click = c
            .sent
            .iter()
            .find_map(|p| match p {
                ServerboundPlay::ContainerClick(x) => Some(x.clone()),
                _ => None,
            })
            .expect("container_click sent");
        assert_eq!((click.slot, click.button, click.mode), (0, 0, 0));
        assert!(click.window_id > 0);
        assert_eq!(click.changed.len(), 1, "{click:?}");
        assert!(click.carried.item.is_some(), "carried diamonds: {click:?}");
        let corrections = c
            .received
            .iter()
            .filter(|n| **n == "minecraft:container_set_slot")
            .count()
            - corrections_before;
        assert_eq!(corrections, 0, "the server corrected our prediction");
        c.press(keys::ESCAPE);
        c.run(5);
        assert!(!c.ui.screen_open());
        assert!(
            c.sent.iter().any(|p| matches!(p, ServerboundPlay::ContainerClose(x) if x.window_id == click.window_id)),
            "{:?}",
            c.sent
        );
        assert!(c.kicked.is_none(), "kicked: {:?}", c.kicked);
        let unimpl = c.ui.host.unimplemented().unwrap();
        eprintln!("Unimplemented: {unimpl:#?}");
        assert_no_refusals(&c.ui.host);
        c.disconnect();
    }

    /// The pause menu's Advancements screen: the server's update_advancements packets feed
    /// vanilla's ClientAdvancements (its tree and progress), the screen opens with the server's
    /// tabs (a vanilla server sends at least the "Minecraft" story root) and asks the server for
    /// the selected tab (seen_advancements).
    #[test]
    #[ignore = "needs cache/ and a test server (HASTE_TEST_SERVER, default 127.0.0.1:25614) where 'haste' is an operator; run with --test-threads=1"]
    fn advancements_screen() {
        let mut c = TestClient::join();
        c.spawn();
        c.run(40);
        assert!(
            c.received.contains(&"minecraft:update_advancements"),
            "no update_advancements from the server: {:?}",
            c.received
        );
        c.press(keys::ESCAPE);
        c.click("Advancements");
        assert_eq!(
            c.ui.gui().screen_class().as_deref(),
            Some("net.minecraft.client.gui.screens.advancements.AdvancementsScreen"),
            "{:?}",
            c.ui.gui().take_last_error()
        );
        let f = c.frame((0.0, 0.0));
        let t = texts(&f);
        // the vanilla server's story root: the "Minecraft" tab title and the window's title
        assert!(
            t.iter().any(|x| x == "Advancements"),
            "missing title: {t:?}"
        );
        assert!(
            t.iter().any(|x| x == "Minecraft"),
            "missing the story tab: {t:?}"
        );
        c.press(keys::ESCAPE);
        c.run(5);
        assert!(!c.ui.screen_open());
        // the screen told the server it closed the tab (vanilla sends seen_advancements)
        assert!(
            c.sent
                .iter()
                .any(|p| matches!(p, ServerboundPlay::SeenAdvancements(_))),
            "no seen_advancements: {:?}",
            c.sent
        );
        let unimpl = c.ui.host.unimplemented().unwrap();
        eprintln!("Unimplemented: {unimpl:#?}");
        assert_no_refusals(&c.ui.host);
        assert!(c.kicked.is_none(), "kicked: {:?}", c.kicked);
        c.disconnect();
    }

    /// A kick shows vanilla's disconnect screen with the reason.
    #[test]
    #[ignore = "needs cache/ and a test server (HASTE_TEST_SERVER, default 127.0.0.1:25614) where 'haste' is an operator; run with --test-threads=1"]
    fn kick_shows_the_disconnect_screen() {
        let mut c = TestClient::join();
        c.spawn();
        c.command("kick haste Bye from the test");
        c.run(20);
        assert!(c.kicked.is_some(), "not kicked");
        assert_eq!(
            c.ui.gui().screen_class().as_deref(),
            Some("net.minecraft.client.gui.screens.DisconnectedScreen")
        );
        let t = texts(&c.frame((0.0, 0.0)));
        assert!(t.iter().any(|x| x.contains("Bye from the test")), "{t:?}");
        c.ui.gui().close_screen();
    }

    /// The HUD's server-driven parts, all vanilla: effect icons, a boss bar, the sidebar
    /// scoreboard, title and subtitle, the action bar, the player list while Tab is held.
    #[test]
    #[ignore = "needs cache/ and a test server (HASTE_TEST_SERVER, default 127.0.0.1:25614) where 'haste' is an operator; run with --test-threads=1"]
    fn hud_elements_from_the_server() {
        let mut c = TestClient::join();
        c.spawn();
        for cmd in [
            "effect give @s minecraft:speed 60 1",
            "bossbar add haste:test \"Test Boss\"",
            "bossbar set haste:test players haste",
            "scoreboard objectives add hastetest dummy \"Haste Sidebar\"",
            "scoreboard objectives setdisplay sidebar hastetest",
            "scoreboard players set haste hastetest 42",
            "title @s times 10 100 10",
            "title @s subtitle \"Small subtitle\"",
            "title @s title \"Big Title\"",
            "title @s actionbar \"Action!\"",
        ] {
            c.command(cmd);
        }
        c.run(20);
        c.ui.key(keys::TAB, keys::keycode(keys::TAB), 0, true);
        let f = c.frame((0.0, 0.0));
        c.ui.key(keys::TAB, keys::keycode(keys::TAB), 0, false);
        for cmd in [
            "effect clear @s",
            "bossbar remove haste:test",
            "scoreboard objectives remove hastetest",
            "title @s clear",
        ] {
            c.command(cmd);
        }
        c.run(5);
        for cmd in &f.commands {
            eprintln!("  {cmd:?}");
        }
        let t = texts(&f);
        for want in [
            "Test Boss",
            "Haste Sidebar",
            "42",
            "Big Title",
            "Small subtitle",
            "Action!",
        ] {
            assert!(
                t.iter().any(|x| x.contains(want)),
                "missing {want:?} in {t:?}"
            );
        }
        // the player list shows our name (and the sidebar its score holder)
        assert!(
            t.iter().filter(|x| x.as_str() == "haste").count() >= 2,
            "{t:?}"
        );
        let sprites: Vec<String> = f
            .commands
            .iter()
            .filter_map(|c| match c {
                GuiCommand::Sprite { sprite, .. } | GuiCommand::SpriteRegion { sprite, .. } => {
                    Some(sprite.to_string())
                }
                _ => None,
            })
            .collect();
        assert!(
            sprites.iter().any(|s| s.contains("effect_background")),
            "effect icon background: {sprites:?}"
        );
        assert!(
            sprites.iter().any(|s| s.contains("speed")),
            "speed effect icon: {sprites:?}"
        );
        assert!(
            sprites.iter().any(|s| s.contains("boss_bar")),
            "boss bar: {sprites:?}"
        );
        let unimpl = c.ui.host.unimplemented().unwrap();
        eprintln!("Unimplemented: {unimpl:#?}");
        assert_no_refusals(&c.ui.host);
        assert!(c.kicked.is_none(), "kicked: {:?}", c.kicked);
        c.disconnect();
    }

    /// Leaving the server stops the HUD like vanilla `Minecraft.disconnectClient` (the pause
    /// screen's "Disconnect"): after `left_world` the title screen draws without the hotbar,
    /// hearts or crosshair, over the plain menu background instead of the last world frame.
    #[test]
    fn leaving_the_world_hides_the_hud() {
        let mut ui = Ui::new(host());
        ui.gui().with_state(|s| {
            s.in_world = true;
            s.game_mode = GameMode::Survival;
            s.vitals.health = 20.0;
            s.vitals.food = 20;
        });
        let in_world = ui.frame(320, 240, 2, (0.0, 0.0), 0.0).clone();
        let sprites: Vec<String> = in_world
            .commands
            .iter()
            .filter_map(|c| match c {
                GuiCommand::Sprite { sprite, .. } | GuiCommand::SpriteRegion { sprite, .. } => {
                    Some(sprite.to_string())
                }
                _ => None,
            })
            .collect();
        assert!(
            sprites
                .iter()
                .any(|s| s.starts_with("minecraft:hud/hotbar")),
            "hotbar while in the world: {sprites:?}"
        );
        // the window's `leave_server` path
        ui.left_world();
        let left = ui.frame(320, 240, 2, (0.0, 0.0), 0.0).clone();
        assert!(
            !left.commands.iter().any(|c| match c {
                GuiCommand::Sprite { sprite, .. } | GuiCommand::SpriteRegion { sprite, .. } => {
                    sprite.path().starts_with("hud/")
                }
                _ => false,
            }),
            "no HUD sprites after leaving: {:?}",
            texts(&left)
        );
    }
}
