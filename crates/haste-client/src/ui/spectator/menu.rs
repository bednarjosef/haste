//! Vanilla `SpectatorMenu`: the category the pages walk, the selected slot, `selectSlot`
//! (a second press of the already-selected slot activates the item) and the fixed slots
//! every page has (the previous-page arrow on later pages, the next-page arrow at slot 7,
//! the close item at slot 8).

use std::borrow::Cow;

use haste_core::game::GameMode;
use haste_protocol::ServerboundPlay;

use super::player_item::PlayerItem;
use super::root_category::RootCategory;
use super::teleport_to_player::TeleportToPlayer;
use super::teleport_to_team::{TeamItem, TeleportToTeam};
use super::{TabList, Teams};

/// What activating an item did (vanilla `SpectatorMenuItem.selectItem`'s side effects).
#[derive(Default)]
pub struct MenuEffects {
    /// `PlayerMenuItem.selectItem`'s teleport packets (`ServerboundTeleportToEntityPacket`).
    pub packets: Vec<ServerboundPlay>,
    /// The close item ended the menu (`SpectatorMenu.exit` -> the listener closes it).
    pub closed: bool,
}

/// The menu's category (vanilla `SpectatorMenuCategory`): what the pages walk.
pub enum Category {
    /// `RootSpectatorMenuCategory`: the two teleport categories, as items.
    Root(RootCategory),
    /// `TeleportToPlayerMenuCategory`: one page of players.
    Players(TeleportToPlayer),
    /// `TeleportToTeamMenuCategory`: one item per team with an online, non-spectator member.
    Teams(TeleportToTeam),
}

impl Category {
    /// The category's item at `index` (`getItems`' entry; `None` past the end).
    fn item(&self, index: i32) -> MenuItem<'_> {
        match self {
            Category::Root(root) => match index {
                0 => MenuItem::RootPlayers(&root.players),
                1 => MenuItem::RootTeams(&root.teams),
                _ => MenuItem::Empty,
            },
            Category::Players(c) => match c.items.get(index as usize) {
                Some(p) => MenuItem::Player(p),
                None => MenuItem::Empty,
            },
            Category::Teams(c) => match c.items.get(index as usize) {
                Some(t) => MenuItem::Team(t),
                None => MenuItem::Empty,
            },
        }
    }

    /// `getItems().len()`.
    fn len(&self) -> usize {
        match self {
            Category::Root(_) => 2,
            Category::Players(c) => c.items.len(),
            Category::Teams(c) => c.items.len(),
        }
    }

    /// `getPrompt`: the action line's text while nothing is selected (en_us,
    /// `spectatorMenu.*.prompt`; the Rust HUD has no language loader yet).
    fn prompt(&self) -> Cow<'static, str> {
        match self {
            Category::Root(_) => {
                Cow::Borrowed("Press a key to select a command, and again to use it.")
            }
            Category::Players(_) => Cow::Borrowed("Select a player to teleport to"),
            Category::Teams(_) => Cow::Borrowed("Select a team to teleport to"),
        }
    }
}

/// One slot's content (vanilla `SpectatorMenuItem`; `Empty` is the `EMPTY_SLOT` sentinel).
pub enum MenuItem<'a> {
    Empty,
    /// The close item (`SpectatorMenu.CloseSpectatorItem`).
    Close,
    /// The previous-page arrow (`ScrollMenuItem(-1, true)`; only on later pages).
    ScrollLeft,
    /// The next-page arrow (`ScrollMenuItem(1, enabled)`).
    ScrollRight {
        enabled: bool,
    },
    /// The root page's "Teleport to Player" (`TeleportToPlayerMenuCategory` as an item).
    RootPlayers(&'a TeleportToPlayer),
    /// The root page's "Teleport to Team Member" (`TeleportToTeamMenuCategory` as an item).
    RootTeams(&'a TeleportToTeam),
    Player(&'a PlayerItem),
    Team(&'a TeamItem),
}

/// `SpectatorMenu`: the category, the selected slot (-1 none) and the page.
pub struct SpectatorMenu {
    category: Category,
    selected_slot: i32,
    page: i32,
}

impl SpectatorMenu {
    /// A fresh menu over the root category (`new SpectatorMenu(listener)`; the root reads
    /// the tab list and scoreboard at this moment, vanilla's constructor chain).
    pub fn new(tab: &TabList, teams: &Teams) -> Self {
        SpectatorMenu {
            category: Category::Root(RootCategory::new(tab, teams)),
            selected_slot: -1,
            page: 0,
        }
    }

    /// `getItem(slot)`: the fixed slots first, then the category's items at
    /// `slot + page * 6`.
    pub fn item(&self, slot: i32) -> MenuItem<'_> {
        let index = slot + self.page * 6;
        if self.page > 0 && slot == 0 {
            MenuItem::ScrollLeft
        } else if slot == 7 {
            MenuItem::ScrollRight {
                enabled: index < self.category.len() as i32,
            }
        } else if slot == 8 {
            MenuItem::Close
        } else {
            self.category.item(index)
        }
    }

    /// `selectSlot`: selecting an empty slot does nothing; selecting the already-selected
    /// slot activates the item (when enabled).
    pub fn select_slot(&mut self, slot: i32, tab: &TabList, out: &mut MenuEffects) {
        if matches!(self.item(slot), MenuItem::Empty) {
            return;
        }
        // The item borrow ends here (the branches below mutate).
        let activate = self.selected_slot == slot && self.item(slot).enabled(tab);
        if activate {
            self.activate(slot, tab, out);
        } else {
            self.selected_slot = slot;
        }
    }

    /// The item's `selectItem` (activation).
    fn activate(&mut self, slot: i32, tab: &TabList, out: &mut MenuEffects) {
        // The match holds a borrow of the category, so the category changes happen after it.
        enum Act {
            Close,
            Page(i32),
            Players(TeleportToPlayer),
            Teams(TeleportToTeam),
        }
        let act = match self.item(slot) {
            MenuItem::Empty => None,
            MenuItem::Close => Some(Act::Close),
            MenuItem::ScrollLeft => Some(Act::Page(-1)),
            MenuItem::ScrollRight { .. } => Some(Act::Page(1)),
            MenuItem::RootPlayers(c) => Some(Act::Players(c.clone())),
            MenuItem::RootTeams(c) => Some(Act::Teams(c.clone())),
            MenuItem::Player(p) => {
                p.select(out);
                None
            }
            MenuItem::Team(t) => Some(Act::Players(TeleportToPlayer::from_items(
                tab,
                t.players.clone(),
            ))),
        };
        match act {
            Some(Act::Close) => out.closed = true,
            Some(Act::Page(d)) => self.page += d,
            Some(Act::Players(c)) => self.select_category(Category::Players(c)),
            Some(Act::Teams(c)) => self.select_category(Category::Teams(c)),
            None => {}
        }
    }

    /// `selectCategory`: the new category starts unselected on its first page.
    pub fn select_category(&mut self, category: Category) {
        self.category = category;
        self.selected_slot = -1;
        self.page = 0;
    }

    /// `getSelectedSlot`.
    pub fn selected_slot(&self) -> i32 {
        self.selected_slot
    }

    /// The selected item (`getSelectedItem` = `getItem(selectedSlot)`).
    pub fn selected_item(&self) -> MenuItem<'_> {
        self.item(self.selected_slot)
    }

    /// The action line's text (`SpectatorGui.extractAction`: the selected item's name, or
    /// the category's prompt when the selected slot is empty).
    pub fn action_text(&self) -> Cow<'_, str> {
        let item = self.selected_item();
        match item.name() {
            Some(name) => name,
            None => self.category.prompt(),
        }
    }

    /// `page`, for the tests.
    pub(super) fn page(&self) -> i32 {
        self.page
    }

    /// Whether the root category is showing, for the tests.
    pub(super) fn is_root(&self) -> bool {
        matches!(self.category, Category::Root(_))
    }
}

impl<'m> MenuItem<'m> {
    /// `isEnabled`: the player items re-read the live game mode (a player that turned
    /// spectator dims); the two categories need at least one item.
    pub fn enabled(&self, tab: &TabList) -> bool {
        match self {
            MenuItem::Empty => false,
            MenuItem::Close | MenuItem::ScrollLeft => true,
            MenuItem::ScrollRight { enabled } => *enabled,
            MenuItem::RootPlayers(c) => !c.items.is_empty(),
            MenuItem::RootTeams(c) => !c.items.is_empty(),
            MenuItem::Player(p) => tab.game_mode(p.uuid) != Some(GameMode::Spectator),
            MenuItem::Team(_) => true,
        }
    }

    /// `getName` (en_us, `spectatorMenu.*`; the Rust HUD has no language loader yet).
    /// `None` is the empty slot (vanilla's `CommonComponents.EMPTY` never shows).
    pub fn name(&self) -> Option<Cow<'m, str>> {
        match self {
            MenuItem::Empty => None,
            MenuItem::Close => Some(Cow::Borrowed("Close Menu")),
            MenuItem::ScrollLeft => Some(Cow::Borrowed("Previous Page")),
            MenuItem::ScrollRight { .. } => Some(Cow::Borrowed("Next Page")),
            MenuItem::RootPlayers(_) => Some(Cow::Borrowed("Teleport to Player")),
            MenuItem::RootTeams(_) => Some(Cow::Borrowed("Teleport to Team Member")),
            MenuItem::Player(p) => Some(Cow::Borrowed(p.name.as_str())),
            MenuItem::Team(t) => Some(Cow::Borrowed(t.display_name.as_str())),
        }
    }
}
