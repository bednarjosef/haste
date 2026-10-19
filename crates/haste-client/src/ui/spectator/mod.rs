//! The spectator menu (vanilla `SpectatorGui`, `SpectatorMenu` and its categories), Haste's
//! port of the tier-3 side the HUD needs input-routed to it: the menu logic (one file per
//! vanilla class, see the submodules), the tab list and teams it reads (vanilla
//! `ClientPacketListener.getListedOnlinePlayers` and `Scoreboard.getPlayerTeams`), and the
//! player skin each item's head icon uses. Drawing goes into the same batched [`GuiFrame`]
//! the vanilla `Hud` filled (it draws the normal hotbar only outside spectator mode, exactly
//! vanilla's `Hud.extractHotbarAndDecorations` split); input arrives through the UI's key,
//! wheel and mouse-button paths (vanilla `Minecraft.handleKeybinds`' spectator branches).
//!
//! Phase 2 note: vanilla's own `SpectatorGui` object lives inside the JVM's real `Hud` and
//! still draws nothing (its menu is never opened from Rust); a mod injecting into
//! `SpectatorMenu.selectSlot` will need this port bridged onto that object.

mod menu;
mod player_item;
mod root_category;
mod spectator_gui;
mod teleport_to_player;
mod teleport_to_team;
#[cfg(test)]
mod tests;

pub(crate) use menu::MenuEffects;
use spectator_gui::SpectatorGui;

use std::sync::Arc;

use ahash::AHashMap;
use haste_core::game::GameMode;
use haste_core::gui::TextMetrics;
use haste_core::Identifier;
use haste_protocol::packets::play::clientbound as cb;
use uuid::Uuid;

use crate::game::Shared;

/// One tab-list entry (vanilla `PlayerInfo`): the fields the menu reads.
#[derive(Clone, Debug)]
pub struct TabEntry {
    pub uuid: Uuid,
    /// The profile name (`ADD_PLAYER`).
    pub name: String,
    /// `UPDATE_LISTED`: whether the tab list shows the player (`getListedOnlinePlayers`).
    pub listed: bool,
    /// `UPDATE_GAME_MODE`, `None` until the server sent one (vanilla's null: not a spectator).
    pub game_mode: Option<GameMode>,
}

/// The client's tab list (vanilla `ClientPacketListener`'s player info map), fed by
/// `player_info_update`/`player_info_remove` on the game thread and read by the render
/// thread's menu drawing.
#[derive(Default)]
pub struct TabList {
    entries: Vec<TabEntry>,
}

impl TabList {
    /// `player_info_update`: adds or updates entries.
    pub fn update(&mut self, p: &cb::PlayerInfoUpdate) {
        for e in &p.entries {
            let entry = match self.entries.iter_mut().find(|t| t.uuid == e.uuid) {
                Some(t) => t,
                None => {
                    self.entries.push(TabEntry {
                        uuid: e.uuid,
                        name: String::new(),
                        listed: false,
                        game_mode: None,
                    });
                    self.entries.last_mut().unwrap()
                }
            };
            if let Some((name, _)) = &e.add {
                entry.name = name.clone();
            }
            if let Some(listed) = e.listed {
                entry.listed = listed;
            }
            if let Some(mode) = e.game_mode {
                entry.game_mode = GameMode::from_id(mode);
            }
        }
    }

    /// `player_info_remove`.
    pub fn remove(&mut self, p: &cb::PlayerInfoRemove) {
        self.entries.retain(|t| !p.uuids.contains(&t.uuid));
    }

    /// `getListedOnlinePlayers`: the listed entries, in arrival order (vanilla's
    /// `LinkedHashMap` values).
    fn listed_online(&self) -> impl Iterator<Item = &TabEntry> {
        self.entries.iter().filter(|t| t.listed)
    }

    /// `getPlayerInfo(String)`: the entry whose profile name is `name`.
    fn by_name(&self, name: &str) -> Option<&TabEntry> {
        self.entries.iter().find(|t| t.name == name)
    }

    /// `PlayerInfo.getGameMode`: the live game mode (`None` until the server sent one).
    fn game_mode(&self, uuid: Uuid) -> Option<GameMode> {
        self.entries
            .iter()
            .find(|t| t.uuid == uuid)
            .and_then(|t| t.game_mode)
    }
}

/// One scoreboard team (vanilla `PlayerTeam`): the fields the menu reads.
#[derive(Clone, Debug)]
pub struct Team {
    pub name: String,
    /// The display name (`PlayerTeam.getDisplayName`), as plain text.
    pub display_name: String,
    /// The team colour's RGB (`TeamColor.rgb`), `None` without one.
    pub color: Option<u32>,
    /// The member names (`PlayerTeam.getPlayers`), in arrival order.
    pub players: Vec<String>,
}

/// The scoreboard's teams (vanilla `Scoreboard.getPlayerTeams`), fed by `set_player_team`.
/// Insertion order (vanilla iterates a hash map, whose order is its own).
#[derive(Default)]
pub struct Teams {
    teams: Vec<Team>,
}

impl Teams {
    /// `set_player_team` (method 0 create, 1 remove, 2 update, 3 add entities, 4 remove).
    pub fn apply(&mut self, p: &cb::SetPlayerTeam) {
        let params = p.parameters.as_ref();
        match p.method {
            1 => self.teams.retain(|t| t.name != p.name),
            0 | 2 => {
                let team = match find_team(&mut self.teams, &p.name) {
                    Some(t) => t,
                    None if p.method == 0 => {
                        self.teams.push(Team {
                            name: p.name.clone(),
                            display_name: String::new(),
                            color: None,
                            players: Vec::new(),
                        });
                        self.teams.last_mut().unwrap()
                    }
                    None => return,
                };
                if let Some(params) = params {
                    team.display_name = params.display_name.to_plain(&|_| None);
                    team.color = params.color.map(team_color);
                }
                // A create also carries the team's first members (vanilla's handler adds
                // them after `addPlayerTeam`).
                if p.method == 0 {
                    team.players.extend(p.entities.iter().cloned());
                }
            }
            3 => {
                if let Some(team) = find_team(&mut self.teams, &p.name) {
                    team.players.extend(p.entities.iter().cloned());
                }
            }
            4 => {
                if let Some(team) = find_team(&mut self.teams, &p.name) {
                    team.players.retain(|n| !p.entities.contains(n));
                }
            }
            _ => {}
        }
    }

    /// `getPlayerTeams`.
    fn iter(&self) -> impl Iterator<Item = &Team> {
        self.teams.iter()
    }
}

/// The team named `name`.
fn find_team<'t>(teams: &'t mut [Team], name: &str) -> Option<&'t mut Team> {
    teams.iter_mut().find(|t| t.name == name)
}

/// `set_player_team`'s colour is a `TeamColor` id; its RGB is the matching legacy chat
/// colour (`TeamColor.BLACK..WHITE` are `TextColor`'s sixteen named colours).
fn team_color(id: i32) -> u32 {
    const COLORS: [u32; 16] = [
        0x000000, 0x0000AA, 0x00AA00, 0x00AAAA, 0xAA0000, 0xAA00AA, 0xFFAA00, 0xAAAAAA, 0x555555,
        0x5555FF, 0x55FF55, 0x55FFFF, 0xFF5555, 0xFF55FF, 0xFFFF55, 0xFFFFFF,
    ];
    // Vanilla's ByIdMap maps an out-of-range id onto index 0 (black).
    COLORS.get(id as usize).copied().unwrap_or(COLORS[0])
}

/// One client's spectator menu state: the menu itself (vanilla `SpectatorGui`), the tab list
/// and teams it reads, the players' head textures, and the text metrics for centring the
/// action line. Everything behind one mutex ([`crate::ui::Ui`]'s shared state); packets the
/// menu sends leave through the UI's outbox, like every packet vanilla code sends.
pub struct SpectatorHud {
    gui: SpectatorGui,
    tab_list: TabList,
    teams: Teams,
    /// Each tab-list player's head texture (`AbstractClientPlayer.getSkin`): the resolved
    /// skin's body, or the default skin's. Refreshed by the game thread's tick while the menu
    /// is open; entries older than that fall back to the default skin at draw time.
    skins: AHashMap<Uuid, Identifier>,
    /// The action line's text, rebuilt when the menu changed (no per-frame allocation;
    /// vanilla allocates its `Component` per draw instead), with the menu revision it was
    /// built at.
    action: String,
    action_rev: u64,
    metrics: Option<Arc<dyn TextMetrics>>,
}

impl SpectatorHud {
    pub fn new() -> Self {
        SpectatorHud {
            gui: SpectatorGui::new(),
            tab_list: TabList::default(),
            teams: Teams::default(),
            skins: AHashMap::new(),
            action: String::new(),
            action_rev: 0,
            metrics: None,
        }
    }

    /// `player_info_update`.
    pub fn player_info_update(&mut self, p: &cb::PlayerInfoUpdate) {
        self.tab_list.update(p);
    }

    /// `player_info_remove`.
    pub fn player_info_remove(&mut self, p: &cb::PlayerInfoRemove) {
        self.tab_list.remove(p);
    }

    /// `set_player_team`.
    pub fn set_player_team(&mut self, p: &cb::SetPlayerTeam) {
        self.teams.apply(p);
    }

    /// The renderer's text metrics (installed once the renderer exists).
    pub fn set_metrics(&mut self, m: Arc<dyn TextMetrics>) {
        self.metrics = Some(m);
    }

    /// `AbstractClientPlayer.getSkin` for every tab-list player (the entity store's resolved
    /// skin, else the profile's default), refreshed by the game thread's tick while the menu
    /// is open — vanilla's `PlayerSkinRenderCache` resolves asynchronously too.
    pub fn refresh_skins(&mut self, store: &haste_entity::EntityStore) {
        if !self.gui.menu_active() {
            return;
        }
        for entry in &self.tab_list.entries {
            let body = store.player_skin(entry.uuid).body.clone();
            self.skins.insert(entry.uuid, body);
        }
    }

    /// Whether the menu is open (`SpectatorGui.isMenuActive`).
    pub fn menu_active(&self) -> bool {
        self.gui.menu_active()
    }

    /// `SpectatorGui.onHotbarSelected` (a digit key, vanilla `Minecraft.handleKeybinds`).
    pub fn on_hotbar_selected(&mut self, slot: i32, out: &mut MenuEffects) {
        let Self {
            gui,
            tab_list,
            teams,
            ..
        } = self;
        gui.on_hotbar_selected(slot, tab_list, teams, out);
        self.remember_action();
    }

    /// `SpectatorGui.onMouseScrolled` (the wheel with the menu open).
    pub fn on_mouse_scrolled(&mut self, wheel: i32, out: &mut MenuEffects) {
        let Self { gui, tab_list, .. } = self;
        gui.on_mouse_scrolled(wheel, tab_list, out);
        self.remember_action();
    }

    /// `SpectatorGui.onHotbarActionKeyPressed` (the key bound to `keySpectatorHotbar`:
    /// middle click).
    pub fn on_hotbar_action_key(&mut self, out: &mut MenuEffects) {
        let Self {
            gui,
            tab_list,
            teams,
            ..
        } = self;
        gui.on_hotbar_action_key(tab_list, teams, out);
        self.remember_action();
    }

    /// The menu's HUD drawing (`SpectatorGui.extractHotbar` + `extractAction`), into the
    /// frame the vanilla `Hud` filled. The fade-out's end closes the menu, exactly vanilla's
    /// `extractHotbar` (`menu.exit()` once the alpha reaches zero).
    pub fn extract(&mut self, frame: &mut haste_core::gui::GuiFrame) {
        if !self.gui.menu_active() {
            return;
        }
        let ctx = DrawContext {
            tab: &self.tab_list,
            skins: &self.skins,
            metrics: self.metrics.as_deref(),
            action: &self.action,
        };
        self.gui.extract(
            &mut frame.commands,
            frame.width as i32,
            frame.height as i32,
            &ctx,
        );
        self.remember_action();
    }

    /// Rebuilds the action line's text after a menu change (`getSelectedItem`'s name or the
    /// category's prompt).
    fn remember_action(&mut self) {
        let rev = self.gui.revision();
        if rev == self.action_rev {
            return;
        }
        self.action_rev = rev;
        self.action = self.gui.action_text().unwrap_or_default().into_owned();
    }
}

/// What the drawing reads: the live tab list (a player that turned spectator dims), the head
/// textures, the text metrics and the cached action text.
struct DrawContext<'a> {
    tab: &'a TabList,
    skins: &'a AHashMap<Uuid, Identifier>,
    metrics: Option<&'a dyn TextMetrics>,
    action: &'a str,
}

/// Vanilla `MouseHandler.onScroll`'s spectator branch: with the menu open the wheel pages
/// through its slots (`SpectatorGui.onMouseScrolled(-wheel)`), with it closed the wheel
/// steps the flying speed (the game thread's tick applies it). `acc` is the caller's
/// sub-notch scroll accumulator; `grabbed` whether the window has the mouse.
pub fn mouse_scroll(
    ui: Option<&crate::ui::Ui>,
    shared: &Shared,
    dy: f64,
    acc: &mut f64,
    grabbed: bool,
) -> bool {
    if !grabbed {
        return false;
    }
    *acc += dy;
    let steps = acc.trunc() as i32;
    if steps == 0 {
        return true;
    }
    *acc -= steps as f64;
    if let Some(ui) = ui.filter(|ui| ui.spectator_menu_active()) {
        ui.spectator_mouse_scrolled(-steps);
    } else {
        shared.input.lock().spectator_scroll += steps as f32;
    }
    true
}
