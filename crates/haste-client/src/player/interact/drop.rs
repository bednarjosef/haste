//! Dropping the held item (player workstream): `MultiPlayerGameMode.dropItem` — the client
//! predicts the stack leaving the selected slot, sends the `player_action` and swings the
//! main hand when something was actually removed.

use haste_core::BlockPos;
use haste_protocol::packets::play::serverbound as sb;
use haste_protocol::ServerboundPlay;

use super::super::LocalPlayer;
use super::action;

impl LocalPlayer {
    /// `MultiPlayerGameMode.dropItem` (`Minecraft.handleKeybinds`' keyDrop loop, `all` =
    /// Ctrl held): `Inventory.removeFromSelected` predicts the removal client-side (one item,
    /// or the whole stack), the action goes out with a zeroed pos and face, and the main hand
    /// swings only when the removed stack was not empty. The server answers with the slot
    /// update, which the prediction matches in normal play.
    pub(super) fn drop_item(&mut self, all: bool, out: &mut Vec<ServerboundPlay>) {
        let slot = self.held_slot.clamp(0, 8) as usize;
        let removed = {
            let held = &mut self.int.hotbar[slot];
            let removed = held.clone();
            if all {
                *held = Default::default();
            } else if !held.is_empty() {
                held.count -= 1;
                if held.count == 0 {
                    *held = Default::default();
                }
            }
            removed
        };
        out.push(ServerboundPlay::PlayerAction(sb::PlayerAction {
            status: if all {
                action::DROP_ALL_ITEMS
            } else {
                action::DROP_ITEM
            },
            pos: BlockPos::new(0, 0, 0),
            face: 0,
            sequence: 0,
        }));
        if !removed.is_empty() {
            self.hand.swing();
        }
    }
}
