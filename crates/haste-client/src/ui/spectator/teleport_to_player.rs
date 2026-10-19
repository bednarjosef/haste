//! Vanilla `TeleportToPlayerMenuCategory`: the page of players a teleport goes to. It is
//! also the root page's first item (the class is both category and item in vanilla).

use haste_core::game::GameMode;

use super::player_item::PlayerItem;
use super::TabList;

/// The player list, snapshotted when it was built (vanilla builds it in the constructor:
/// the tab list's listed players, filtered and sorted, become fixed `PlayerMenuItem`s).
#[derive(Clone, Debug)]
pub struct TeleportToPlayer {
    pub items: Vec<PlayerItem>,
}

impl TeleportToPlayer {
    /// Over the tab list's listed players (`getListedOnlinePlayers`): spectators filtered
    /// out by their mode at construction, sorted by profile id (`PROFILE_ORDER`).
    pub fn new(tab: &TabList) -> Self {
        Self::build(
            tab,
            tab.listed_online().map(|e| PlayerItem {
                uuid: e.uuid,
                name: e.name.clone(),
            }),
        )
    }

    /// Over a team's players (the team item's list): the same filter and order, applied
    /// when the team is picked (vanilla constructs the category on that click).
    pub fn from_items(tab: &TabList, items: Vec<PlayerItem>) -> Self {
        Self::build(tab, items.into_iter())
    }

    fn build(tab: &TabList, players: impl Iterator<Item = PlayerItem>) -> Self {
        let mut items: Vec<PlayerItem> = players
            .filter(|p| tab.game_mode(p.uuid) != Some(GameMode::Spectator))
            .collect();
        items.sort_by_key(|i| i.uuid);
        TeleportToPlayer { items }
    }
}
