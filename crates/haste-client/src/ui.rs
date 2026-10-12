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
// Wired into the game loop by the client (client-app); until then only the tests use it.
#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::Arc;

use haste_jvm::{GameState, GuiPipeline, JvmHost, ScreenKind};
use haste_protocol::haste_core::game::GameMode;
use haste_protocol::haste_core::gui::{GuiFrame, TextMetrics};
use haste_protocol::haste_core::Component;
use haste_protocol::packets::configuration::clientbound::{RegistryData, UpdateTags};
use haste_protocol::packets::play::serverbound as sb;
use haste_protocol::packets::{Decoded, Direction, Dispatch, Packet, ProtocolTables, State};
use haste_protocol::wire::Writer;
use haste_protocol::{ClientboundPlay, Connection, Login, ServerboundPlay};
use parking_lot::Mutex;

pub use haste_jvm::keys;

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
    "minecraft:open_screen",
    "minecraft:container_set_content",
    "minecraft:container_set_slot",
    "minecraft:container_set_data",
    "minecraft:container_close",
    "minecraft:set_cursor_item",
    "minecraft:set_player_inventory",
    "minecraft:merchant_offers",
    "minecraft:update_tags",
];

/// Packets about one entity: forwarded only when they are about the local player.
const FORWARDED_LOCAL: &[&str] = &[
    "minecraft:set_entity_data",
    "minecraft:update_mob_effect",
    "minecraft:remove_mob_effect",
    "minecraft:update_attributes",
    "minecraft:player_combat_kill",
];

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
            }),
            frame: GuiFrame::default(),
        }
    }

    fn gui(&self) -> &GuiPipeline {
        self.host.gui()
    }

    /// The renderer's text metrics (`Renderer::text_metrics`): vanilla's `Font` measures with them.
    pub fn set_text_metrics(&self, m: Arc<dyn TextMetrics>) {
        self.gui().set_text_metrics(m);
    }

    /// Joined a server (after `Connection::connect`): registries, tags and feature flags from the
    /// configuration phase, the local player from `login`.
    pub fn joined(&self, conn: &Connection, login: &Login) -> anyhow::Result<()> {
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
        let entity = login.entity_id.0;
        self.shared.local_entity.store(entity, Ordering::Relaxed);
        self.gui()
            .set_player(entity, &login.profile.name, login.profile.uuid.as_u128())?;
        self.gui()
            .set_dimension_type(login.packet.spawn.dimension_type);
        self.gui().with_state(|s| {
            s.in_world = true;
            s.game_mode = login.game_mode;
            s.vitals.hardcore = login.hardcore;
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
            P::Respawn(r) => {
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

    /// One 20 Hz client tick (game thread).
    pub fn tick(&self) {
        self.gui().tick();
        if let Some(e) = self.gui().take_last_error() {
            tracing::warn!("ui tick: {e}");
        }
    }

    /// Everything the UI sends this tick, in order: packets vanilla code sent, chat and commands
    /// typed in the chat screen, the selected hotbar slot when it changed.
    pub fn drain_outgoing(&self, out: &mut Vec<ServerboundPlay>) {
        let (packets, chat, slot) =
            self.state(|s| (s.take_outgoing(), s.take_sent(), s.selected_slot));
        if !packets.is_empty() {
            let tables = self.shared.tables.lock().clone();
            let dispatch = self.shared.serverbound.lock();
            if let (Some(tables), Some(dispatch)) = (tables, dispatch.as_ref()) {
                for p in packets {
                    match decode_serverbound(dispatch, &tables, &p) {
                        Ok(sp) => out.push(sp),
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
        if !self.gui().is_open() {
            return false;
        }
        self.gui().mouse(x, y, Some((button, pressed)));
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
            Ok(f) => self.frame = f,
            Err(e) => {
                tracing::warn!("ui frame: {e:#}");
                self.frame.commands.clear();
            }
        }
        &self.frame
    }

    /// Whether a screen is open (the client releases the mouse and stops game input).
    pub fn screen_open(&self) -> bool {
        self.gui().is_open()
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

    /// Whether vanilla code asked to leave the server (pause screen "Disconnect"); clears it.
    pub fn take_disconnect_request(&self) -> bool {
        self.state(|s| std::mem::take(&mut s.disconnect_requested))
    }

    /// The connection ended: vanilla's disconnect screen with the reason; the HUD stops.
    pub fn disconnected(&self, reason: &Component) {
        self.state(|s| s.in_world = false);
        if let Err(e) = self.gui().disconnected(reason) {
            tracing::warn!("disconnect screen: {e:#}");
        }
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

    /// Joins the test server as "haste", gives itself stone and says hello through commands (the
    /// player must be an operator: `ops.json` of the test server), plays a few seconds and renders
    /// the HUD: the hotbar with the stone and the chat line must be in the frame.
    #[test]
    #[ignore = "needs cache/ and a test server (HASTE_TEST_SERVER, default 127.0.0.1:25614) where 'haste' is an operator"]
    fn joins_and_renders_the_hud() {
        use haste_protocol::haste_core::gui::GuiCommand;
        use haste_protocol::packets::play::clientbound as cb;
        use haste_protocol::{Account, ConnectOptions};

        let addr = std::env::var("HASTE_TEST_SERVER").unwrap_or_else(|_| "127.0.0.1:25614".into());
        let cache = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cache");
        let t = std::time::Instant::now();
        let host = JvmHost::start(haste_jvm::JvmConfig::from_cache(&cache).unwrap()).unwrap();
        eprintln!("JVM ready in {:?}: {}", t.elapsed(), host.proxy_summary());
        let mut ui = Ui::new(host);
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .build()
            .unwrap();
        rt.block_on(async {
            let opts = ConnectOptions::new(
                addr.clone(),
                Account::Offline {
                    username: "haste".into(),
                },
            );
            let (mut conn, login) = Connection::connect(opts).await.expect("connect");
            let t = std::time::Instant::now();
            ui.joined(&conn, &login).expect("registries into Java");
            eprintln!(
                "joined {addr}; {} synchronised registries into Java in {:?}",
                conn.registries().names().count(),
                t.elapsed()
            );
            let mut pos = None;
            let mut loaded = false;
            let mut out = Vec::new();
            let mut received = Vec::new();
            for tick in 0..140 {
                while let Some(p) = conn.try_recv() {
                    ui.handle(&p);
                    if let Some(n) = p.name() {
                        received.push(n);
                    }
                    if let ClientboundPlay::PlayerPosition(t) = &p {
                        pos = Some((t.pos, t.yaw, t.pitch));
                        conn.send(ServerboundPlay::AcceptTeleportation(
                            sb::AcceptTeleportation {
                                teleport_id: t.teleport_id,
                                pos: t.pos,
                                yaw: t.yaw,
                                pitch: t.pitch,
                            },
                        ))
                        .unwrap();
                        if !loaded {
                            conn.send(ServerboundPlay::PlayerLoaded(sb::PlayerLoaded {}))
                                .unwrap();
                            loaded = true;
                            continue;
                        }
                    }
                    if let ClientboundPlay::Disconnect(d) = &p {
                        panic!("kicked: {}", d.reason.to_plain(&|_| None));
                    }
                }
                if loaded && tick == 30 {
                    // what the chat screen produces for typed commands
                    ui.gui().with_state(|s| {
                        s.sent.push(haste_jvm::ChatMessage {
                            text: "give @s minecraft:stone 64".into(),
                            command: true,
                        });
                        s.sent.push(haste_jvm::ChatMessage {
                            text: "say hello from haste".into(),
                            command: true,
                        });
                    });
                }
                if let (true, Some((p, yaw, pitch))) = (loaded, pos) {
                    conn.send(ServerboundPlay::MovePlayerPosRot(sb::MovePlayerPosRot {
                        pos: p,
                        yaw,
                        pitch,
                        flags: sb::move_flags::ON_GROUND,
                    }))
                    .unwrap();
                    conn.send(ServerboundPlay::ClientTickEnd(sb::ClientTickEnd {}))
                        .unwrap();
                }
                ui.tick();
                ui.drain_outgoing(&mut out);
                for p in out.drain(..) {
                    conn.send(p).unwrap();
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            assert!(!conn.is_closed(), "closed: {:?}", conn.close_reason());
            let _ = cb::SetHealth::NAME;
            eprintln!(
                "tier-3 packets seen: {:?}",
                received
                    .iter()
                    .filter(|n| FORWARDED.contains(n) || FORWARDED_LOCAL.contains(n))
                    .collect::<std::collections::BTreeSet<_>>()
            );
            conn.disconnect();
        });
        let stone = ui
            .gui()
            .with_state(|s| s.slot(0).cloned())
            .expect("stone in hotbar slot 0 (from the server's container_set_slot)");
        assert_eq!(stone.count, 64);
        let t = std::time::Instant::now();
        let f = ui.frame(320, 240, 2, (0.0, 0.0), 0.0).clone();
        eprintln!(
            "HUD frame in {:?}: {} commands",
            t.elapsed(),
            f.commands.len()
        );
        for c in &f.commands {
            eprintln!("  {c:?}");
        }
        assert!(f.commands.iter().any(
            |c| matches!(c, GuiCommand::Sprite { sprite, .. } if sprite.path() == "hud/hotbar")
        ));
        assert!(f.commands.iter().any(|c| matches!(c,
            GuiCommand::Item { item, count: 64, x: 72, y: 221 } if Some(*item) == stone.item)));
        let text: Vec<&str> = f
            .commands
            .iter()
            .filter_map(|c| match c {
                GuiCommand::Text { text, .. } | GuiCommand::StyledText { text, .. } => {
                    Some(text.as_str())
                }
                _ => None,
            })
            .collect();
        assert!(
            text.iter().any(|t| t.contains("hello from haste")),
            "chat line in the HUD: {text:?}"
        );
        let chat = ui.gui().chat_history().unwrap();
        eprintln!("chat log: {chat:?}");
        assert!(
            chat.iter().any(|l| l == "[haste] hello from haste"),
            "{chat:?}"
        );
        let unimpl = ui.host.unimplemented().unwrap();
        eprintln!("Unimplemented: {unimpl:#?}");
    }
}
