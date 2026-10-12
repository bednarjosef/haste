//! Rust-owned state behind the tier-2 proxies, and the handle scheme.
//!
//! The proxies themselves are Java objects (generated classes, see `proxies.conf`); this module is
//! the Rust side they read through `haste.rt.Native`. Canonical-proxy rule: Java keeps exactly one
//! proxy per handle in `haste.gui.Proxies` for the JVM's lifetime, so state mods attach to a proxy
//! (Phase 2 `@Unique` fields) survives.
//!
//! Handles:
//! * `LocalPlayer`, `Inventory`, `Minecraft`, `Gui`, `Window`, `Options`, `Font`: singletons, handle 0.
//! * `ItemStack`: the stack held in inventory slot `n` has handle `n` (vanilla `Inventory` slot
//!   numbering: 0-8 hotbar, 9-35 main, 36-39 armour feet..head, 40 offhand). The empty stack
//!   (`ItemStack.EMPTY`) has handle -1. A slot's proxy stays the same object when its contents
//!   change (approximation: vanilla moves stack objects between slots; see NOTES.md).
use haste_core::RegistryIndex;
use parking_lot::Mutex;

/// Number of slots of vanilla's player `Inventory` (36 main + 4 armour + offhand).
pub const INVENTORY_SIZE: usize = 41;

/// One non-empty inventory slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemSlot {
    /// Index into the `minecraft:item` registry.
    pub item: RegistryIndex,
    /// Stack size (> 0).
    pub count: i32,
}

/// A chat line or command a screen sent (the client forwards it to the server).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMessage {
    /// The text without the leading `/` for commands.
    pub text: String,
    /// True for commands.
    pub command: bool,
}

/// The game state the proxies read. Owned by Rust; the client updates it (e.g. from inventory
/// packets) and screens see it immediately through the natives.
#[derive(Debug)]
pub struct GameState {
    /// Inventory slots in vanilla `Inventory` numbering (always [`INVENTORY_SIZE`] long).
    pub inventory: Vec<Option<ItemSlot>>,
    /// Selected hotbar slot (0-8).
    pub selected_slot: i32,
    /// Item registry names by registry index (`minecraft:stone`), for tooltips; may be empty.
    pub item_names: Vec<String>,
    /// Chat history shown by the chat screen, oldest first.
    pub chat_lines: Vec<String>,
    /// Messages the chat screen sent since the last [`GameState::take_sent`].
    pub sent: Vec<ChatMessage>,
    /// Local player name.
    pub player_name: String,
}

impl Default for GameState {
    fn default() -> Self {
        GameState {
            inventory: vec![None; INVENTORY_SIZE],
            selected_slot: 0,
            item_names: Vec::new(),
            chat_lines: Vec::new(),
            sent: Vec::new(),
            player_name: "Player".to_owned(),
        }
    }
}

impl GameState {
    /// Slot contents for an `ItemStack` handle (None for empty / out of range / EMPTY).
    pub fn stack(&self, handle: i64) -> Option<ItemSlot> {
        usize::try_from(handle).ok().and_then(|i| self.inventory.get(i).copied().flatten())
    }
    /// Drains the messages sent by screens.
    pub fn take_sent(&mut self) -> Vec<ChatMessage> {
        std::mem::take(&mut self.sent)
    }
}

static STATE: Mutex<Option<GameState>> = Mutex::new(None);

/// Runs `f` on the process-wide game state (created on first use).
pub(crate) fn with_state<T>(f: impl FnOnce(&mut GameState) -> T) -> T {
    let mut g = STATE.lock();
    f(g.get_or_insert_with(GameState::default))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_handles() {
        let mut s = GameState::default();
        s.inventory[3] = Some(ItemSlot { item: RegistryIndex(1), count: 5 });
        assert_eq!(s.stack(3).map(|x| x.count), Some(5));
        assert_eq!(s.stack(-1), None);
        assert_eq!(s.stack(4), None);
        assert_eq!(s.stack(999), None);
        s.sent.push(ChatMessage { text: "hi".into(), command: false });
        assert_eq!(s.take_sent().len(), 1);
        assert!(s.sent.is_empty());
    }
}
