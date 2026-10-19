//! Tests for the spectator menu: the menu logic (root page, paging, selection, exit, the
//! spectator filter, the teleport packet, the wheel's slot skipping, the fade-out, the
//! drawing) and, against the JVM, the input routing (the digit keys open the menu in
//! spectator mode and change the hotbar slot outside it).

use std::sync::Arc;

use haste_core::gui::{GuiCommand, GuiFrame, TextMetrics};
use haste_protocol::haste_core::BlockStateId;
use haste_protocol::packets::play::clientbound as cb;
use haste_protocol::packets::play::clientbound::player_info_actions as actions;
use haste_protocol::{ClientboundPlay, ServerboundPlay};
use uuid::Uuid;

use super::menu::{MenuItem, SpectatorMenu};
use super::{MenuEffects, SpectatorHud};

/// A `player_info_update` for one player: name, listed flag and game mode together.
fn info(uuid: Uuid, name: &str, listed: bool, game_mode: i32) -> cb::PlayerInfoUpdate {
    cb::PlayerInfoUpdate {
        actions: actions::ADD_PLAYER | actions::UPDATE_LISTED | actions::UPDATE_GAME_MODE,
        entries: vec![cb::PlayerInfoEntry {
            uuid,
            add: Some((name.to_owned(), Vec::new())),
            listed: Some(listed),
            game_mode: Some(game_mode),
            ..Default::default()
        }],
    }
}

/// `uuid(i)`: distinct ids in a known order (`uuid(0) < uuid(1) < ...`).
fn uuid(i: u16) -> Uuid {
    Uuid::from_u64_pair(0, u64::from(i))
}

/// A spectator HUD with `n` listed survival players named `p0`.. and one unlisted.
fn hud_with_players(n: u16) -> SpectatorHud {
    let mut hud = SpectatorHud::new();
    for i in 0..n {
        hud.player_info_update(&info(uuid(i), &format!("p{i}"), true, 0));
    }
    hud.player_info_update(&info(uuid(n), "hidden", false, 0));
    hud
}

/// `set_player_team` creating a team.
fn team(name: &str, color: i32, players: &[&str]) -> cb::SetPlayerTeam {
    cb::SetPlayerTeam {
        name: name.to_owned(),
        method: 0,
        parameters: Some(cb::TeamParameters {
            display_name: haste_protocol::haste_core::Component::text(name),
            prefix: Default::default(),
            suffix: Default::default(),
            name_tag_visibility: 0,
            collision_rule: 0,
            color: Some(color),
            flags: 0,
        }),
        entities: players.iter().map(|p| (*p).to_owned()).collect(),
    }
}

/// Selects `slot` twice (vanilla: the second press of the selected slot activates it).
fn select(menu: &mut SpectatorMenu, slot: i32, hud: &SpectatorHud, out: &mut MenuEffects) {
    menu.select_slot(slot, &hud.tab_list, out);
    menu.select_slot(slot, &hud.tab_list, out);
}

/// The slot names of a page, `None` for empty slots.
fn names(menu: &SpectatorMenu, slots: i32) -> Vec<Option<String>> {
    (0..slots)
        .map(|s| menu.item(s).name().map(|n| n.into_owned()))
        .collect()
}

fn effects() -> MenuEffects {
    MenuEffects::default()
}

fn frame() -> GuiFrame {
    GuiFrame {
        width: 480,
        height: 240,
        ..Default::default()
    }
}

fn sprites(frame: &GuiFrame) -> Vec<&str> {
    frame
        .commands
        .iter()
        .filter_map(|c| match c {
            GuiCommand::Sprite { sprite, .. } => Some(sprite.path()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_root_page_lists_both_teleport_categories() {
    let hud = hud_with_players(2);
    let menu = SpectatorMenu::new(&hud.tab_list, &hud.teams);
    assert_eq!(menu.item(0).name().as_deref(), Some("Teleport to Player"));
    assert_eq!(
        menu.item(1).name().as_deref(),
        Some("Teleport to Team Member")
    );
    assert_eq!(menu.item(8).name().as_deref(), Some("Close Menu"));
    // No team and two players: the player category is enabled, the team one is not, and the
    // root fits one page (the next-page arrow at slot 7 is disabled).
    assert!(menu.item(0).enabled(&hud.tab_list));
    assert!(!menu.item(1).enabled(&hud.tab_list));
    assert!(!menu.item(7).enabled(&hud.tab_list));
    assert!(menu.item(8).enabled(&hud.tab_list));
    for slot in 2..7 {
        assert!(matches!(menu.item(slot), MenuItem::Empty));
    }
    // An empty root (no players, no teams): both teleport items are disabled.
    let empty = SpectatorHud::new();
    let menu = SpectatorMenu::new(&empty.tab_list, &empty.teams);
    assert!(!menu.item(0).enabled(&empty.tab_list));
    assert!(!menu.item(1).enabled(&empty.tab_list));
}

#[test]
fn paging_walks_more_than_eight_players() {
    // 10 players: 7 on the first page (slots 0..6), 3 on the second (slots 1..3).
    let hud = hud_with_players(10);
    let mut menu = SpectatorMenu::new(&hud.tab_list, &hud.teams);
    let mut out = effects();
    select(&mut menu, 0, &hud, &mut out);
    assert!(!menu.is_root(), "the player category is showing");
    assert_eq!(menu.page(), 0);
    for slot in 0..7 {
        // The players are sorted by profile id: uuid(0)..uuid(6).
        assert!(
            matches!(menu.item(slot), MenuItem::Player(p) if p.uuid == uuid(slot as u16)),
            "slot {slot}"
        );
    }
    assert!(menu.item(7).enabled(&hud.tab_list), "more players remain");
    // The next page: the previous-page arrow takes slot 0, players 7..9 take slots 1..3.
    select(&mut menu, 7, &hud, &mut out);
    assert_eq!(menu.page(), 1);
    assert!(matches!(menu.item(0), MenuItem::ScrollLeft));
    for slot in 1..=3 {
        assert!(
            matches!(menu.item(slot), MenuItem::Player(p) if p.uuid == uuid(slot as u16 + 6)),
            "slot {slot}"
        );
    }
    for slot in 4..7 {
        assert!(matches!(menu.item(slot), MenuItem::Empty));
    }
    assert!(
        !menu.item(7).enabled(&hud.tab_list),
        "no third page of players"
    );
    // And back: the previous-page arrow returns to page 0.
    select(&mut menu, 0, &hud, &mut out);
    assert_eq!(menu.page(), 0);
    assert!(matches!(menu.item(0), MenuItem::Player(_)));
}

#[test]
fn selecting_and_activating_the_close_item_exits_the_menu() {
    let mut hud = hud_with_players(1);
    let mut out = effects();
    // The first digit opens the menu (nothing selected yet: the root's prompt shows); the
    // second selects the slot; the third activates it.
    hud.on_hotbar_selected(0, &mut out);
    assert!(hud.menu_active());
    assert_eq!(hud.gui.menu_selected_slot(), -1);
    assert_eq!(
        hud.gui.action_text().unwrap(),
        "Press a key to select a command, and again to use it."
    );
    hud.on_hotbar_selected(0, &mut out);
    assert_eq!(hud.gui.menu_selected_slot(), 0);
    assert_eq!(hud.gui.action_text().unwrap(), "Teleport to Player");
    hud.on_hotbar_selected(0, &mut out);
    assert!(!hud.gui.menu_is_root(), "the teleport activated");
    assert_eq!(
        hud.gui.action_text().unwrap(),
        "Select a player to teleport to"
    );
    // Selecting an empty slot does nothing (the selection stays empty).
    hud.on_hotbar_selected(5, &mut out);
    assert_eq!(hud.gui.menu_selected_slot(), -1);
    // The close item (slot 8): selected once, activated the second time.
    hud.on_hotbar_selected(8, &mut out);
    hud.on_hotbar_selected(8, &mut out);
    assert!(!hud.menu_active(), "the close item ended the menu");
}

#[test]
fn spectators_are_filtered_out_of_the_player_list() {
    let mut hud = SpectatorHud::new();
    hud.player_info_update(&info(uuid(0), "survivor", true, 0));
    hud.player_info_update(&info(uuid(1), "spectator", true, 3));
    let mut menu = SpectatorMenu::new(&hud.tab_list, &hud.teams);
    let mut out = effects();
    select(&mut menu, 0, &hud, &mut out);
    assert_eq!(
        names(&menu, 7),
        vec![Some("survivor".into()), None, None, None, None, None, None]
    );
    // A player that turns spectator while the menu is open stays listed but is disabled.
    hud.player_info_update(&info(uuid(0), "survivor", true, 3));
    assert!(!menu.item(0).enabled(&hud.tab_list));
}

#[test]
fn selecting_a_player_sends_the_teleport_packet_with_their_uuid() {
    let hud = hud_with_players(3);
    let mut menu = SpectatorMenu::new(&hud.tab_list, &hud.teams);
    let mut out = effects();
    select(&mut menu, 0, &hud, &mut out);
    select(&mut menu, 1, &hud, &mut out);
    assert_eq!(
        out.packets,
        vec![ServerboundPlay::TeleportToEntity(
            haste_protocol::packets::play::serverbound::TeleportToEntity { target: uuid(1) }
        )],
        "the second player, by profile-id order"
    );
}

#[test]
fn the_team_page_lists_teams_with_online_members() {
    let mut hud = SpectatorHud::new();
    hud.player_info_update(&info(uuid(0), "online", true, 0));
    hud.player_info_update(&info(uuid(1), "ghost", true, 3));
    hud.set_player_team(&team("red", 12, &["online", "ghost", "offline"]));
    hud.set_player_team(&team("empty", 7, &["offline"]));
    let mut menu = SpectatorMenu::new(&hud.tab_list, &hud.teams);
    let mut out = effects();
    select(&mut menu, 1, &hud, &mut out);
    // Only the team with an online, non-spectator member shows, with its colour.
    let MenuItem::Team(t) = menu.item(0) else {
        panic!("the team item is at slot 0");
    };
    let (name, color) = (t.display_name.clone(), t.color);
    assert_eq!((name.as_str(), color), ("red", Some(0xFF5555)));
    assert!(matches!(menu.item(1), MenuItem::Empty));
    // Selecting it shows the team's online players (the spectator is filtered out).
    select(&mut menu, 0, &hud, &mut out);
    assert_eq!(
        names(&menu, 7),
        vec![Some("online".into()), None, None, None, None, None, None]
    );
}

#[test]
fn the_wheel_skips_empty_and_disabled_slots() {
    let mut hud = hud_with_players(3);
    let mut out = effects();
    // Three presses open the menu, select slot 0 and activate it (the player category).
    hud.on_hotbar_selected(0, &mut out);
    hud.on_hotbar_selected(0, &mut out);
    hud.on_hotbar_selected(0, &mut out);
    // Wheel down from nothing selected: slot 0 (the first player) is selected.
    hud.on_mouse_scrolled(1, &mut out);
    assert_eq!(hud.gui.menu_selected_slot(), 0);
    hud.on_mouse_scrolled(1, &mut out);
    assert_eq!(hud.gui.menu_selected_slot(), 1);
    // The third player sits at slot 2; the next notch then skips the empty slots 3..6 and
    // the disabled arrow at 7 in one go: the close item (8) is the next selectable one.
    hud.on_mouse_scrolled(1, &mut out);
    assert_eq!(hud.gui.menu_selected_slot(), 2);
    hud.on_mouse_scrolled(1, &mut out);
    assert_eq!(hud.gui.menu_selected_slot(), 8);
    // Wheel up skips back over everything empty to slot 2.
    hud.on_mouse_scrolled(-1, &mut out);
    assert_eq!(hud.gui.menu_selected_slot(), 2);
}

#[test]
fn the_fade_out_closes_the_menu() {
    let mut hud = hud_with_players(1);
    let mut out = effects();
    hud.on_hotbar_selected(0, &mut out);
    assert!(hud.menu_active());
    // Not yet: the bar stays up for 5 s and fades for 2.
    hud.gui
        .age_selection(std::time::Duration::from_millis(4000));
    hud.extract(&mut frame());
    assert!(hud.menu_active());
    // Past the fade the menu closes (`SpectatorGui.extractHotbar`'s `menu.exit()`).
    hud.gui
        .age_selection(std::time::Duration::from_millis(4000));
    hud.extract(&mut frame());
    assert!(!hud.menu_active());
}

/// 6 px per character, like the JVM's fallback metrics.
struct FixedMetrics;
impl TextMetrics for FixedMetrics {
    fn width(&self, text: &str) -> i32 {
        text.chars().count() as i32 * 6
    }
}

#[test]
fn a_fresh_menu_draws_the_hotbar_the_root_icons_and_the_prompt() {
    let mut hud = hud_with_players(1);
    hud.set_metrics(Arc::new(FixedMetrics));
    let mut out = effects();
    // Open the menu and select slot 0 (the root's teleport item).
    hud.on_hotbar_selected(0, &mut out);
    hud.on_hotbar_selected(0, &mut out);
    let mut f = frame();
    hud.extract(&mut f);
    // The bar and the selection highlight (over slot 0), the two root icons, the disabled
    // next-page arrow (drawn dimmed) and the close item.
    assert_eq!(
        sprites(&f),
        vec![
            "hud/hotbar",
            "hud/hotbar_selection",
            "spectator/teleport_to_player",
            "spectator/teleport_to_team",
            "spectator/scroll_right",
            "spectator/close",
        ]
    );
    // The bar sits at the bottom, the highlight one pixel up-left of slot 0.
    let GuiCommand::Sprite {
        x: bar_x, y: bar_y, ..
    } = &f.commands[0]
    else {
        panic!("the hotbar sprite");
    };
    assert_eq!((*bar_x, *bar_y), (480 / 2 - 91, 240 - 22));
    let GuiCommand::Sprite {
        x: sel_x, y: sel_y, ..
    } = &f.commands[1]
    else {
        panic!("the selection sprite");
    };
    assert_eq!((*sel_x, *sel_y), (480 / 2 - 92, 240 - 23));
    // The key labels of the enabled slots (1 and the close item's 9; the team item and the
    // next-page arrow are disabled and get no label), then the selected item's name centred
    // above the bar.
    let action = "Teleport to Player";
    let texts: Vec<(&str, i32, i32)> = f
        .commands
        .iter()
        .filter_map(|c| match c {
            GuiCommand::Text { x, y, text, .. } => Some((text.as_str(), *x, *y)),
            _ => None,
        })
        .collect();
    assert_eq!(
        texts,
        vec![
            // Slot x is `width/2 - 90 + slot*20 + 2`; the label sits at `x + 17 - width`.
            ("1", 480 / 2 - 90 + 2 + 17 - 6, 240 - 22 + 3 + 9),
            ("9", 480 / 2 - 90 + 8 * 20 + 2 + 17 - 6, 240 - 22 + 3 + 9),
            (action, (480 - FixedMetrics.width(action)) / 2, 240 - 35),
        ]
    );
}

// ---- against the JVM: the input routing (needs cache/) ----

#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM, data generator reports)"]
fn hotbar_keys_select_menu_slots_in_spectator_and_the_hotbar_outside() {
    let _serial = super::super::tests::frame_tests().lock().unwrap();
    let mut ui = crate::ui::Ui::new(super::super::tests::host());
    ui.gui().reset_for_test();
    ui.set_text_metrics(Arc::new(FixedMetrics));
    ui.gui().with_state(|s| {
        s.in_world = true;
        s.game_mode = haste_core::game::GameMode::Spectator;
        s.selected_slot = 4;
    });
    ui.handle(
        &ClientboundPlay::PlayerInfoUpdate(info(uuid(0), "other", true, 0)),
        &|_| BlockStateId::AIR,
    );
    // The digit opens the menu (the first press selects slot 0, the root's teleport item).
    ui.key(
        haste_jvm::keys::NUM_1,
        haste_jvm::keys::keycode(haste_jvm::keys::NUM_1),
        0,
        true,
    );
    assert!(ui.spectator_menu_active());
    let f = ui.frame(480, 240, 2, (0.0, 0.0), 0.0).clone();
    let drawn = sprites(&f);
    assert!(
        drawn.contains(&"hud/hotbar"),
        "the spectator bar: {drawn:?}"
    );
    assert!(
        drawn.contains(&"spectator/teleport_to_player"),
        "the root page: {drawn:?}"
    );
    assert_eq!(
        ui.selected_slot(),
        4,
        "in spectator the digit never changes the inventory slot"
    );
    // Outside spectator mode the same digit changes the hotbar slot and nothing draws.
    ui.gui()
        .with_state(|s| s.game_mode = haste_core::game::GameMode::Survival);
    ui.key(
        haste_jvm::keys::NUM_1 + 1,
        haste_jvm::keys::keycode(haste_jvm::keys::NUM_1 + 1),
        0,
        true,
    );
    assert_eq!(ui.selected_slot(), 1);
    let f = ui.frame(480, 240, 2, (0.0, 0.0), 0.0).clone();
    let drawn = sprites(&f);
    assert!(
        !drawn.contains(&"spectator/teleport_to_player"),
        "the menu does not draw outside spectator mode: {drawn:?}"
    );
}
