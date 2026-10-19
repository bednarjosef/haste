//! Vanilla `TeleportToTeamMenuCategory` (the root page's second item) and its
//! `TeamSelectionItem`: one item per team with an online, non-spectator member.

use super::player_item::PlayerItem;
use super::{TabList, Team};

/// The team list, built when the menu opens.
#[derive(Clone, Debug)]
pub struct TeleportToTeam {
    pub items: Vec<TeamItem>,
}

impl TeleportToTeam {
    /// `createTeamEntries`: one item per team whose members include a listed-or-unlisted
    /// online non-spectator (`TeamSelectionItem.create`), in team order.
    pub fn new(tab: &TabList, teams: &super::Teams) -> Self {
        TeleportToTeam {
            items: teams
                .iter()
                .filter_map(|t| TeamItem::create(t, tab))
                .collect(),
        }
    }
}

/// `TeamSelectionItem`: a team as a menu item. Selecting it shows the team's members.
#[derive(Clone, Debug)]
pub struct TeamItem {
    /// `getName`: `PlayerTeam.getDisplayName`, as plain text.
    pub display_name: String,
    /// `PlayerTeam.getColor`'s RGB (the icon's backdrop fill).
    pub color: Option<u32>,
    /// The team's online, non-spectator members, snapshotted when the item was built.
    pub players: Vec<PlayerItem>,
    /// The member whose head is the icon (`iconSkin`; vanilla picks one at random).
    pub icon: uuid::Uuid,
}

impl TeamItem {
    /// `TeamSelectionItem.create`: `None` when no member is online and not a spectator.
    fn create(team: &Team, tab: &TabList) -> Option<TeamItem> {
        let mut players = Vec::new();
        for name in &team.players {
            if let Some(e) = tab.by_name(name) {
                if e.game_mode != Some(haste_core::game::GameMode::Spectator) {
                    players.push(PlayerItem {
                        uuid: e.uuid,
                        name: e.name.clone(),
                    });
                }
            }
        }
        if players.is_empty() {
            return None;
        }
        // Vanilla rolls `RandomSource` here; the pick is cosmetic, so it is seeded from the
        // team name (stable, and different per team).
        let icon = players[pick(team, players.len())].uuid;
        Some(TeamItem {
            display_name: team.display_name.clone(),
            color: team.color,
            players,
            icon,
        })
    }
}

/// A stable stand-in for vanilla's random icon pick: the team name's FNV-1a hash.
fn pick(team: &Team, len: usize) -> usize {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for b in team.name.bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    (hash % len as u64) as usize
}
