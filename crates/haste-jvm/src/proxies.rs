//! Rust-owned state behind the tier-2 proxies.
//!
//! The proxies themselves are Java objects (generated classes, see `proxies.conf`); this module is
//! the Rust side they read through `haste.rt.Native`. Canonical-proxy rule: Java keeps exactly one
//! proxy per Rust object in `haste.gui.Proxies` for the JVM's lifetime, so state mods attach to a
//! proxy (Phase 2 `@Unique` fields) survives.
//!
//! The player's inventory is the exception (see `NOTES.md`, "Inventory"): it is a real vanilla
//! `Inventory` of real `ItemStack`s in Java, because vanilla's menus apply the server's container
//! packets and predict clicks by mutating stacks in place. Rust keeps a mirror
//! ([`GameState::inventory`], [`GameState::carried`]) that Java updates after every change.
use haste_core::entity::ItemStackData;
use haste_core::game::GameMode;
use haste_core::RegistryIndex;
use parking_lot::Mutex;

/// Number of slots of vanilla's player `Inventory` that Rust mirrors (36 main + armour + offhand
/// + body; Java reports the real size, the mirror grows to it).
pub const INVENTORY_SIZE: usize = 41;

/// A chat line or command a screen sent (the client forwards it to the server).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMessage {
    /// The text without the leading `/` for commands.
    pub text: String,
    /// True for commands.
    pub command: bool,
}

/// Vanilla's `ContainerInput` (the click type of the container-click packet; ids are its ordinals).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerInput {
    /// Normal click (pick up / put down / swap with carried).
    Pickup,
    /// Shift-click.
    QuickMove,
    /// Number key or offhand swap (button = hotbar slot, 40 = offhand).
    Swap,
    /// Middle click in creative.
    Clone,
    /// Drop key over a slot.
    Throw,
    /// Drag distribution.
    QuickCraft,
    /// Double click: collect all of the kind.
    PickupAll,
    /// The container was closed (`LocalPlayer.closeContainer`; slot and button are -1).
    Close,
    /// An id this crate does not know.
    Unknown(i32),
}

impl ContainerInput {
    /// From vanilla's ordinal.
    pub fn from_id(id: i32) -> Self {
        match id {
            0 => Self::Pickup,
            1 => Self::QuickMove,
            2 => Self::Swap,
            3 => Self::Clone,
            4 => Self::Throw,
            5 => Self::QuickCraft,
            6 => Self::PickupAll,
            -1 => Self::Close,
            n => Self::Unknown(n),
        }
    }
}

/// A click in a container screen, to be sent to the server by the client (Rust owns the inventory).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContainerClick {
    /// Menu id (0 = the player's own inventory menu).
    pub container_id: i32,
    /// Menu slot index (inventory menu: 0 result, 1-4 crafting, 5-8 armour, 9-35 main, 36-44 hotbar, 45 offhand), -999 outside.
    pub slot: i32,
    /// Mouse button or hotbar key, as vanilla passes it.
    pub button: i32,
    /// Click type.
    pub input: ContainerInput,
}

/// The game state the proxies read. Owned by Rust; the client updates it (e.g. from packets) and
/// screens see it immediately through the natives. Java writes back what vanilla code produced
/// (inventory changes, chat, serverbound packets).
#[derive(Debug)]
pub struct GameState {
    /// Mirror of the player's inventory in vanilla `Inventory` numbering (0-8 hotbar, 9-35 main,
    /// 36-39 armour feet..head, 40 offhand, then any further equipment slots). Written by Java.
    pub inventory: Vec<ItemStackData>,
    /// Mirror of the stack carried by the cursor in an open container. Written by Java.
    pub carried: ItemStackData,
    /// Selected hotbar slot (0-8).
    pub selected_slot: i32,
    /// Chat history shown by the chat screen, oldest first.
    pub chat_lines: Vec<String>,
    /// Messages the chat screen sent since the last [`GameState::take_sent`].
    pub sent: Vec<ChatMessage>,
    /// Local player name.
    pub player_name: String,
    /// Container clicks from screens since the last [`GameState::take_container_clicks`].
    pub container_clicks: Vec<ContainerClick>,
    /// Local player's game mode (creative has infinite materials).
    pub game_mode: GameMode,
    /// Operator permission level 0-4 (entity events 24-28 for the local player).
    pub permission_level: u8,
    /// Serverbound play packets vanilla code sent (VarInt id + body, vanilla's encoding), oldest first.
    pub outgoing: Vec<Vec<u8>>,
}

impl Default for GameState {
    fn default() -> Self {
        GameState {
            inventory: vec![empty_stack(); INVENTORY_SIZE],
            carried: empty_stack(),
            selected_slot: 0,
            chat_lines: Vec::new(),
            sent: Vec::new(),
            player_name: "Player".to_owned(),
            game_mode: GameMode::Survival,
            permission_level: 0,
            container_clicks: Vec::new(),
            outgoing: Vec::new(),
        }
    }
}

/// The empty stack.
pub fn empty_stack() -> ItemStackData {
    ItemStackData {
        count: 0,
        item: None,
        components_raw: bytes::Bytes::new(),
    }
}

impl GameState {
    /// Mirror of inventory slot `slot` (empty when out of range).
    pub fn slot(&self, slot: usize) -> Option<&ItemStackData> {
        self.inventory.get(slot).filter(|s| !s.is_empty())
    }
    /// The selected hotbar stack.
    pub fn selected(&self) -> Option<&ItemStackData> {
        usize::try_from(self.selected_slot)
            .ok()
            .and_then(|s| self.slot(s))
    }
    /// Stores a stack Java reported (`slot` -1 = carried).
    pub(crate) fn set_mirror(&mut self, slot: i32, stack: ItemStackData) {
        if slot < 0 {
            self.carried = stack;
            return;
        }
        let i = slot as usize;
        if self.inventory.len() <= i {
            self.inventory.resize(i + 1, empty_stack());
        }
        self.inventory[i] = stack;
    }
    /// Drains the container clicks made in screens.
    pub fn take_container_clicks(&mut self) -> Vec<ContainerClick> {
        std::mem::take(&mut self.container_clicks)
    }
    /// Drains the messages sent by screens.
    pub fn take_sent(&mut self) -> Vec<ChatMessage> {
        std::mem::take(&mut self.sent)
    }
    /// Drains the serverbound packets vanilla code sent (VarInt id + body each).
    pub fn take_outgoing(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.outgoing)
    }
}

/// Decodes a stack in vanilla's network encoding (`ItemStack.OPTIONAL_STREAM_CODEC`: VarInt
/// count, then for a non-empty stack VarInt item id and the component patch) into
/// [`ItemStackData`]. The patch is kept raw, like `haste-protocol` does.
pub fn decode_stack(b: &[u8]) -> Option<ItemStackData> {
    let mut i = 0;
    let count = read_var_int(b, &mut i)?;
    if count <= 0 {
        return Some(empty_stack());
    }
    let item = read_var_int(b, &mut i)?;
    Some(ItemStackData {
        count,
        item: Some(RegistryIndex(u32::try_from(item).ok()?)),
        components_raw: bytes::Bytes::copy_from_slice(&b[i..]),
    })
}

/// Encodes [`ItemStackData`] in vanilla's network encoding (the inverse of [`decode_stack`]).
pub fn encode_stack(s: &ItemStackData) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + s.components_raw.len());
    match (s.count, s.item) {
        (c, Some(item)) if c > 0 => {
            write_var_int(&mut out, c);
            write_var_int(&mut out, item.0 as i32);
            if s.components_raw.is_empty() {
                out.extend_from_slice(&[0, 0]);
            } else {
                out.extend_from_slice(&s.components_raw);
            }
        }
        _ => out.push(0),
    }
    out
}

pub(crate) fn read_var_int(b: &[u8], i: &mut usize) -> Option<i32> {
    let mut v: u32 = 0;
    for shift in (0..35).step_by(7) {
        let byte = *b.get(*i)?;
        *i += 1;
        v |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(v as i32);
        }
    }
    None
}

pub(crate) fn write_var_int(out: &mut Vec<u8>, v: i32) {
    let mut v = v as u32;
    loop {
        if v & !0x7f == 0 {
            out.push(v as u8);
            return;
        }
        out.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
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
    fn stack_encoding_round_trips() {
        let s = ItemStackData {
            count: 64,
            item: Some(RegistryIndex(300)),
            components_raw: bytes::Bytes::from_static(&[1, 0, 5, 9]),
        };
        let b = encode_stack(&s);
        assert_eq!(b[..3], [64, 0xAC, 0x02]);
        assert_eq!(decode_stack(&b), Some(s));
        assert_eq!(decode_stack(&[0]), Some(empty_stack()));
        assert_eq!(encode_stack(&empty_stack()), vec![0]);
        assert_eq!(decode_stack(&[0x80]), None, "truncated VarInt");
        let plain = ItemStackData {
            count: 1,
            item: Some(RegistryIndex(1)),
            components_raw: bytes::Bytes::new(),
        };
        assert_eq!(encode_stack(&plain), vec![1, 1, 0, 0]);
    }

    #[test]
    fn mirror_slots() {
        let mut s = GameState::default();
        s.set_mirror(
            3,
            ItemStackData {
                count: 5,
                item: Some(RegistryIndex(1)),
                components_raw: bytes::Bytes::new(),
            },
        );
        assert_eq!(s.slot(3).map(|x| x.count), Some(5));
        assert!(s.slot(4).is_none());
        assert!(s.slot(999).is_none());
        s.set_mirror(50, empty_stack());
        assert_eq!(s.inventory.len(), 51);
        s.selected_slot = 3;
        assert_eq!(s.selected().map(|x| x.count), Some(5));
        s.sent.push(ChatMessage {
            text: "hi".into(),
            command: false,
        });
        assert_eq!(s.take_sent().len(), 1);
        assert!(s.sent.is_empty());
    }
}
