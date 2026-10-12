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

/// The local player's primitive state the HUD reads (vanilla `Player` getters and fields on the
/// player proxy). The client fills it from the packets (`set_health`, `set_experience`,
/// `player_abilities`, `login`/`game_event` for the game mode) and its own physics (eyes in
/// water); it is copied into Java once per frame and per tick (`haste.gui.Mirror`). The local
/// player's synched entity data (air, absorption), effects and attributes are vanilla objects in
/// Java, updated from the forwarded packets.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerVitals {
    /// Health (0 = dead, 20 = full).
    pub health: f32,
    /// Food level 0-20.
    pub food: i32,
    /// Food saturation.
    pub saturation: f32,
    /// Progress to the next experience level, 0-1.
    pub xp_progress: f32,
    /// Experience level.
    pub xp_level: i32,
    /// Total experience points.
    pub xp_total: i32,
    /// The player's eyes are in water (air bubbles shown).
    pub eye_in_water: bool,
    /// Hardcore world (hardcore hearts).
    pub hardcore: bool,
    /// Abilities: invulnerable.
    pub invulnerable: bool,
    /// Abilities: flying.
    pub flying: bool,
    /// Abilities: may fly.
    pub may_fly: bool,
    /// Abilities: instant build (creative).
    pub instabuild: bool,
    /// Abilities: flying speed.
    pub fly_speed: f32,
    /// Abilities: walking speed (field of view modifier).
    pub walk_speed: f32,
}

impl Default for PlayerVitals {
    fn default() -> Self {
        PlayerVitals {
            health: 20.0,
            food: 20,
            saturation: 5.0,
            xp_progress: 0.0,
            xp_level: 0,
            xp_total: 0,
            eye_in_water: false,
            hardcore: false,
            invulnerable: false,
            flying: false,
            may_fly: false,
            instabuild: false,
            fly_speed: 0.05,
            walk_speed: 0.1,
        }
    }
}

/// What the F3 overlay shows (a Haste stub, see `NOTES.md`): filled by the client every frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DebugInfo {
    /// Frames per second.
    pub fps: i32,
    /// Feet position.
    pub pos: [f64; 3],
    /// Yaw and pitch in degrees.
    pub rot: [f32; 2],
    /// Loaded chunks.
    pub chunks: i32,
    /// Chunk sections drawn / total.
    pub sections_drawn: i32,
    /// Chunk sections total.
    pub sections_total: i32,
    /// Entities tracked.
    pub entities: i32,
}

/// Layout of the per-frame mirror arrays (`haste.gui.Mirror` in Java has the same numbers).
pub(crate) mod mirror {
    pub const I_SELECTED: usize = 0;
    pub const I_GAME_MODE: usize = 1;
    pub const I_PERMISSION: usize = 2;
    pub const I_FOOD: usize = 3;
    pub const I_XP_LEVEL: usize = 4;
    pub const I_XP_TOTAL: usize = 5;
    pub const I_FLAGS: usize = 6;
    pub const I_FPS: usize = 7;
    pub const I_CHUNKS: usize = 8;
    pub const I_SECTIONS_DRAWN: usize = 9;
    pub const I_SECTIONS_TOTAL: usize = 10;
    pub const I_ENTITIES: usize = 11;
    pub const INTS: usize = 12;
    pub const D_HEALTH: usize = 0;
    pub const D_SATURATION: usize = 1;
    pub const D_XP_PROGRESS: usize = 2;
    pub const D_FLY_SPEED: usize = 3;
    pub const D_WALK_SPEED: usize = 4;
    pub const D_X: usize = 5;
    pub const D_Y: usize = 6;
    pub const D_Z: usize = 7;
    pub const D_YAW: usize = 8;
    pub const D_PITCH: usize = 9;
    pub const D_GAME_TIME: usize = 10;
    pub const D_DAY_TIME: usize = 11;
    pub const DOUBLES: usize = 12;
    pub const F_HARDCORE: i32 = 1;
    pub const F_EYE_IN_WATER: i32 = 2;
    pub const F_INVULNERABLE: i32 = 4;
    pub const F_FLYING: i32 = 8;
    pub const F_MAY_FLY: i32 = 16;
    pub const F_INSTABUILD: i32 = 32;
    pub const F_IN_WORLD: i32 = 64;
}

impl GameState {
    /// The mirror arrays Java copies into the proxies (layout in [`mirror`]).
    pub(crate) fn mirror(&self) -> ([i32; mirror::INTS], [f64; mirror::DOUBLES]) {
        use mirror::*;
        let v = &self.vitals;
        let mut i = [0i32; INTS];
        let mut d = [0f64; DOUBLES];
        i[I_SELECTED] = self.selected_slot;
        i[I_GAME_MODE] = self.game_mode as i32;
        i[I_PERMISSION] = i32::from(self.permission_level);
        i[I_FOOD] = v.food;
        i[I_XP_LEVEL] = v.xp_level;
        i[I_XP_TOTAL] = v.xp_total;
        let flag = |b: bool, f: i32| if b { f } else { 0 };
        i[I_FLAGS] = flag(v.hardcore, F_HARDCORE)
            | flag(v.eye_in_water, F_EYE_IN_WATER)
            | flag(v.invulnerable, F_INVULNERABLE)
            | flag(v.flying, F_FLYING)
            | flag(v.may_fly, F_MAY_FLY)
            | flag(v.instabuild, F_INSTABUILD)
            | flag(self.in_world, F_IN_WORLD);
        i[I_FPS] = self.debug.fps;
        i[I_CHUNKS] = self.debug.chunks;
        i[I_SECTIONS_DRAWN] = self.debug.sections_drawn;
        i[I_SECTIONS_TOTAL] = self.debug.sections_total;
        i[I_ENTITIES] = self.debug.entities;
        d[D_HEALTH] = f64::from(v.health);
        d[D_SATURATION] = f64::from(v.saturation);
        d[D_XP_PROGRESS] = f64::from(v.xp_progress);
        d[D_FLY_SPEED] = f64::from(v.fly_speed);
        d[D_WALK_SPEED] = f64::from(v.walk_speed);
        d[D_X] = self.debug.pos[0];
        d[D_Y] = self.debug.pos[1];
        d[D_Z] = self.debug.pos[2];
        d[D_YAW] = f64::from(self.debug.rot[0]);
        d[D_PITCH] = f64::from(self.debug.rot[1]);
        d[D_GAME_TIME] = self.game_time as f64;
        d[D_DAY_TIME] = self.day_time as f64;
        (i, d)
    }
}

/// A chat line or command a screen sent (the client forwards it to the server).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMessage {
    /// The text without the leading `/` for commands.
    pub text: String,
    /// True for commands.
    pub command: bool,
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
    /// Messages the chat screen sent since the last [`GameState::take_sent`].
    pub sent: Vec<ChatMessage>,
    /// Local player name.
    pub player_name: String,
    /// Local player's game mode (creative has infinite materials).
    pub game_mode: GameMode,
    /// Operator permission level 0-4 (entity events 24-28 for the local player).
    pub permission_level: u8,
    /// The local player's health, food, experience, abilities, ...
    pub vitals: PlayerVitals,
    /// What the F3 overlay shows.
    pub debug: DebugInfo,
    /// Joined a world (the HUD is drawn; before that only screens).
    pub in_world: bool,
    /// World age in ticks (`set_time`).
    pub game_time: i64,
    /// Time of day in ticks (`set_time`).
    pub day_time: i64,
    /// Serverbound play packets vanilla code sent (VarInt id + body, vanilla's encoding), oldest first.
    pub outgoing: Vec<Vec<u8>>,
}

impl Default for GameState {
    fn default() -> Self {
        GameState {
            inventory: vec![empty_stack(); INVENTORY_SIZE],
            carried: empty_stack(),
            selected_slot: 0,
            sent: Vec::new(),
            player_name: "Player".to_owned(),
            game_mode: GameMode::Survival,
            permission_level: 0,
            vitals: PlayerVitals::default(),
            debug: DebugInfo::default(),
            in_world: false,
            game_time: 0,
            day_time: 0,
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
