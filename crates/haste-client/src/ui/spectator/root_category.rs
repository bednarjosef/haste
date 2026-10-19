//! Vanilla `RootSpectatorMenuCategory`: the menu's first page, the two teleport categories
//! as its items.

use super::teleport_to_player::TeleportToPlayer;
use super::teleport_to_team::TeleportToTeam;
use super::{TabList, Teams};

/// The root page: both teleport categories, built when the menu opens (vanilla's
/// constructor chain reads the tab list and scoreboard at that moment).
#[derive(Clone, Debug)]
pub struct RootCategory {
    /// Slot 0: "Teleport to Player" (en_us `spectatorMenu.teleport`).
    pub players: TeleportToPlayer,
    /// Slot 1: "Teleport to Team Member" (en_us `spectatorMenu.team_teleport`).
    pub teams: TeleportToTeam,
}

impl RootCategory {
    pub fn new(tab: &TabList, teams: &Teams) -> Self {
        RootCategory {
            players: TeleportToPlayer::new(tab),
            teams: TeleportToTeam::new(tab, teams),
        }
    }
}
