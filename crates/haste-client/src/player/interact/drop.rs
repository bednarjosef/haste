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
    /// Ctrl held). The removal happens on the Java inventory — the one the HUD and screens
    /// read — through `remove_from_selected`: vanilla `LocalPlayer.drop` runs
    /// `Inventory.removeFromSelected` on the client inventory itself and the server does not
    /// resend the slot, so the Java count is the one the player sees (predicting on Rust's own
    /// copy left the hotbar stale). `remove_from_selected(all)` is the bridge: it removes on the
    /// Java inventory — the existing `InventorySync` mirrors the change back to Rust, so the two
    /// never disagree — and returns whether the removed stack was empty. The action goes out
    /// with a zeroed pos and face, and the main hand swings only when the removed stack was not
    /// empty.
    pub(super) fn drop_item(
        &mut self,
        remove_from_selected: &dyn Fn(bool) -> bool,
        all: bool,
        out: &mut Vec<ServerboundPlay>,
    ) {
        let removed_empty = remove_from_selected(all);
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
        if !removed_empty {
            self.hand.swing();
        }
    }

    /// The no-JVM prediction (vanilla `Inventory.removeFromSelected` on Rust's own hotbar copy),
    /// used when there is no live UI to mirror the Java change back. Returns whether the removed
    /// stack was empty.
    fn predict_drop(&mut self, all: bool) -> bool {
        let slot = self.held_slot.clamp(0, 8) as usize;
        let removed = self.int.hotbar[slot].clone();
        if all {
            self.int.hotbar[slot] = Default::default();
        } else if !removed.is_empty() {
            self.int.hotbar[slot].count -= 1;
            if self.int.hotbar[slot].count == 0 {
                self.int.hotbar[slot] = Default::default();
            }
        }
        removed.is_empty()
    }
}
