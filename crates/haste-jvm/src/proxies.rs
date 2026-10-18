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

/// The mount HUD while riding (vanilla `Hud`'s vehicle hearts and jump bar). Set while the
/// player rides a living vehicle, cleared on foot: the hearts replace the food bar
/// (`Hud.extractVehicleHealth` over `player.getVehicle()`), and the jump bar replaces the
/// experience bar (`Hud.nextContextualInfoState` over `LocalPlayer.jumpableVehicle`) while
/// we steer a saddled horse-family vehicle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MountHud {
    /// The vehicle's health (`LivingEntity.DATA_HEALTH_ID`, metadata index 9).
    pub health: f32,
    /// The vehicle's `Attributes.MAX_HEALTH` (the synced attribute, the family's base
    /// before a sync lands).
    pub max_health: f32,
    /// Vanilla `LocalPlayer.jumpableVehicle`: the vehicle we control can jump (the
    /// horse-family vehicle Rust steers; its jump bar shows [`Self::jump_scale`]).
    pub jumpable: bool,
    /// Vanilla `LocalPlayer.getJumpRidingScale`: the charged jump the jump bar shows.
    pub jump_scale: f32,
}

/// The attack indicator's charge (vanilla `Hud.extractCrosshair` and its hotbar variant): the
/// local player's attack strength ticker and the held item's attack delay, ticked by the client
/// once per tick (`Player.tick` / `Player.getCurrentItemAttackStrengthDelay`) and read by the
/// player proxy's `getAttackStrengthScale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AttackStrengthHud {
    /// `Player.attackStrengthTicker`: ticks since the last attack or held-item change.
    pub ticker: f32,
    /// `Player.getCurrentItemAttackStrengthDelay`: 20 / attack speed, in ticks.
    pub delay: f32,
}

impl Default for AttackStrengthHud {
    fn default() -> Self {
        // A fist fully charged (attack speed 4.0): the HUD draws no bar before the client
        // publishes its own value.
        AttackStrengthHud {
            ticker: 5.0,
            delay: 5.0,
        }
    }
}

/// What the F3 overlay shows (vanilla's default debug entries, see `NOTES.md`): filled by the
/// client every frame. The numbers go through the mirror arrays; the strings
/// ([`DebugInfo::biome`], [`DebugInfo::target_block`], ...) through `Native.debugText`, pulled by
/// Java only while the overlay is on.
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
    /// Entity instances drawn this frame (`E:`'s first number).
    pub entities_rendered: i32,
    /// Server simulation distance (`E:`'s `SD:`).
    pub simulation_distance: i32,
    /// Server view distance (`C:`'s `D:`).
    pub view_distance: i32,
    /// Framerate limit as vanilla prints it in the fps line (260 prints as "inf").
    pub framerate_limit: i32,
    /// Refresh rate in Hz of the mode in use (0 when there is no window: offscreen runs).
    pub refresh_rate: i32,
    /// Display size in pixels (the window's framebuffer size; 0x0 offscreen).
    pub display_size: [i32; 2],
    /// GUI scale the HUD runs at.
    pub gui_scale: i32,
    /// GPU name and backend ("Radeon ... (Vulkan)").
    pub gpu: String,
    /// CPU as vanilla prints it ("16x AMD Ryzen ...").
    pub cpu: String,
    /// Biome at the feet position, e.g. "minecraft:plains".
    pub biome: String,
    /// Dimension id, e.g. "minecraft:overworld".
    pub dimension: String,
    /// Sky and block light at the feet position.
    pub light: [i32; 2],
    /// Server brand ("vanilla", "fabric", ...).
    pub brand: String,
    /// Packets sent/received per second.
    pub tx_per_s: f64,
    pub rx_per_s: f64,
    /// The block under the crosshair (20 blocks reach, vanilla `Entity.pick` without liquids).
    pub target_block: Option<TargetedState>,
    /// The fluid under the crosshair (vanilla `Entity.pick` with liquids).
    pub target_fluid: Option<TargetedState>,
}

/// A block or fluid the crosshair points at: what vanilla's looking-at entries print.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TargetedState {
    /// Registry name, e.g. "minecraft:water".
    pub name: String,
    /// Block position.
    pub pos: [i32; 3],
    /// State properties as `name: value` pairs, sorted by name (vanilla's order).
    pub properties: Vec<(String, String)>,
    /// Tags the block/fluid is in, as "#ns:name" strings.
    pub tags: Vec<String>,
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
    pub const I_ENTITIES_RENDERED: usize = 12;
    pub const I_SIMULATION_DISTANCE: usize = 13;
    pub const I_VIEW_DISTANCE: usize = 14;
    pub const I_FRAMERATE_LIMIT: usize = 15;
    pub const I_REFRESH_RATE: usize = 16;
    pub const I_DISPLAY_W: usize = 17;
    pub const I_DISPLAY_H: usize = 18;
    pub const I_GUI_SCALE: usize = 19;
    pub const I_LIGHT_SKY: usize = 20;
    pub const I_LIGHT_BLOCK: usize = 21;
    pub const INTS: usize = 22;
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
    pub const D_TX_PER_S: usize = 12;
    pub const D_RX_PER_S: usize = 13;
    /// The mount's health and max health, and the charged jump scale (see `MountHud`).
    pub const D_MOUNT_HEALTH: usize = 14;
    pub const D_MOUNT_MAX_HEALTH: usize = 15;
    pub const D_MOUNT_JUMP_SCALE: usize = 16;
    /// The attack indicator's charge (see `AttackStrengthHud`).
    pub const D_ATTACK_STRENGTH_TICKER: usize = 17;
    pub const D_ATTACK_STRENGTH_DELAY: usize = 18;
    pub const DOUBLES: usize = 19;
    pub const F_HARDCORE: i32 = 1;
    pub const F_EYE_IN_WATER: i32 = 2;
    pub const F_INVULNERABLE: i32 = 4;
    pub const F_FLYING: i32 = 8;
    pub const F_MAY_FLY: i32 = 16;
    pub const F_INSTABUILD: i32 = 32;
    pub const F_IN_WORLD: i32 = 64;
    pub const F_IMMEDIATE_RESPAWN: i32 = 128;
    pub const F_DEBUG_SHOWN: i32 = 256;
    /// Riding a living vehicle (the mount HUD's hearts show; see `MountHud`).
    pub const F_MOUNT_LIVING: i32 = 512;
    /// Steering a vehicle that can jump (the jump bar replaces the experience bar).
    pub const F_MOUNT_JUMPABLE: i32 = 1024;
    /// The crosshair picks a living, alive entity (the attack indicator's full icon).
    pub const F_CROSSHAIR_PICK: i32 = 2048;
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
            | flag(self.in_world, F_IN_WORLD)
            | flag(self.immediate_respawn, F_IMMEDIATE_RESPAWN)
            | flag(self.debug_shown, F_DEBUG_SHOWN)
            | flag(self.mount.is_some(), F_MOUNT_LIVING)
            | flag(self.mount.is_some_and(|m| m.jumpable), F_MOUNT_JUMPABLE)
            | flag(self.crosshair_pick_living, F_CROSSHAIR_PICK);
        let dbg = &self.debug;
        i[I_FPS] = dbg.fps;
        i[I_CHUNKS] = dbg.chunks;
        i[I_SECTIONS_DRAWN] = dbg.sections_drawn;
        i[I_SECTIONS_TOTAL] = dbg.sections_total;
        i[I_ENTITIES] = dbg.entities;
        i[I_ENTITIES_RENDERED] = dbg.entities_rendered;
        i[I_SIMULATION_DISTANCE] = dbg.simulation_distance;
        i[I_VIEW_DISTANCE] = dbg.view_distance;
        i[I_FRAMERATE_LIMIT] = dbg.framerate_limit;
        i[I_REFRESH_RATE] = dbg.refresh_rate;
        i[I_DISPLAY_W] = dbg.display_size[0];
        i[I_DISPLAY_H] = dbg.display_size[1];
        i[I_GUI_SCALE] = dbg.gui_scale;
        i[I_LIGHT_SKY] = dbg.light[0];
        i[I_LIGHT_BLOCK] = dbg.light[1];
        d[D_HEALTH] = f64::from(v.health);
        d[D_SATURATION] = f64::from(v.saturation);
        d[D_XP_PROGRESS] = f64::from(v.xp_progress);
        d[D_FLY_SPEED] = f64::from(v.fly_speed);
        d[D_WALK_SPEED] = f64::from(v.walk_speed);
        d[D_X] = dbg.pos[0];
        d[D_Y] = dbg.pos[1];
        d[D_Z] = dbg.pos[2];
        d[D_YAW] = f64::from(dbg.rot[0]);
        d[D_PITCH] = f64::from(dbg.rot[1]);
        d[D_GAME_TIME] = self.game_time as f64;
        d[D_DAY_TIME] = self.day_time as f64;
        d[D_TX_PER_S] = dbg.tx_per_s;
        d[D_RX_PER_S] = dbg.rx_per_s;
        if let Some(m) = &self.mount {
            d[D_MOUNT_HEALTH] = f64::from(m.health);
            d[D_MOUNT_MAX_HEALTH] = f64::from(m.max_health);
            d[D_MOUNT_JUMP_SCALE] = f64::from(m.jump_scale);
        }
        d[D_ATTACK_STRENGTH_TICKER] = f64::from(self.attack_strength.ticker);
        d[D_ATTACK_STRENGTH_DELAY] = f64::from(self.attack_strength.delay);
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

/// The world the create screen asked to make (vanilla's CreateWorldScreen choices). The client
/// turns it into `server.properties` for the local server, which creates the world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateWorldRequest {
    /// The world folder (under the singleplayer dir; the create screen picked an unused name).
    pub folder: String,
    /// The world's name as typed (displayed in the world list).
    pub name: String,
    /// Vanilla `GameType` id: 0 survival, 1 creative, 2 adventure, 3 spectator.
    pub game_mode: i32,
    /// Vanilla `Difficulty` id: 0 peaceful .. 3 hard.
    pub difficulty: i32,
    /// Hardcore world.
    pub hardcore: bool,
    /// Cheats (vanilla `allowCommands`): the player joins as an operator.
    pub allow_commands: bool,
    /// The seed, decimal (empty = random).
    pub seed: String,
    /// Generate structures.
    pub generate_structures: bool,
    /// World type, a preset id like `minecraft:normal`.
    pub level_type: String,
}

/// The game state the proxies read. Owned by Rust; the client updates it (e.g. from packets) and
/// screens see it immediately through the natives. Java writes back what vanilla code produced
/// (inventory changes, chat, serverbound packets).
pub struct GameState {
    /// Mirror of the player's inventory in vanilla `Inventory` numbering (0-8 hotbar, 9-35 main,
    /// 36-39 armour feet..head, 40 offhand, then any further equipment slots). Written by Java.
    pub inventory: Vec<ItemStackData>,
    /// Mirror of the stack carried by the cursor in an open container. Written by Java.
    pub carried: ItemStackData,
    /// Inventory slots Java changed since the last [`GameState::take_inventory_changes`]
    /// (vanilla numbering), oldest first. Java predicts inventory edits itself (creative
    /// slot sets, menu clicks); the server never echoes those back, so the client must take
    /// them from here.
    pub inventory_changes: Vec<(i32, ItemStackData)>,
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
    /// The mount HUD while riding a living vehicle (None on foot), see [`MountHud`].
    pub mount: Option<MountHud>,
    /// The attack indicator's charge (crosshair bar and hotbar icon), see [`AttackStrengthHud`].
    pub attack_strength: AttackStrengthHud,
    /// The crosshair picks a living, alive entity in reach (vanilla
    /// `Minecraft.crosshairPickEntity`): the attack indicator's full icon.
    pub crosshair_pick_living: bool,
    /// What the F3 overlay shows.
    pub debug: DebugInfo,
    /// Whether the F3 overlay is on (vanilla's `DebugScreenEntryList.isOverlayVisible`; the
    /// client toggles it, Java reads it from the mirror so both sides agree).
    pub debug_shown: bool,
    /// Joined a world (the HUD is drawn; before that only screens).
    pub in_world: bool,
    /// The server respawns dead players at once (no death screen: `login`, `game_event` 11).
    pub immediate_respawn: bool,
    /// World age in ticks (`set_time`).
    pub game_time: i64,
    /// Time of day in ticks (`set_time`).
    pub day_time: i64,
    /// Vanilla code asked to leave the server (pause screen "Disconnect"); the client disconnects
    /// and clears this.
    pub disconnect_requested: bool,
    /// The multiplayer screen's Join: the address to connect to, until the client takes it.
    pub join_requested: Option<String>,
    /// The connecting screen's Cancel: a join still in progress must be dropped.
    pub join_cancelled: bool,
    /// Vanilla code asked to quit the game (the title screen's Quit Game).
    pub quit_requested: bool,
    /// The world list's "Play Selected World": the world folder to start the local server for
    /// and join, until the client takes it.
    pub open_world_requested: Option<String>,
    /// The create screen's "Create New World": the world to create (folder and settings), until
    /// the client takes it.
    pub create_world_requested: Option<CreateWorldRequest>,
    /// A local singleplayer server is running (the pause screen's "Save and Quit" label). Set by
    /// the client when it starts/stops the local server.
    pub local_world_running: bool,
    /// Serverbound play packets vanilla code sent (VarInt id + body, vanilla's encoding), oldest first.
    pub outgoing: Vec<Vec<u8>>,
    /// Option changes vanilla screens made, as options.txt key/value pairs, oldest first. The
    /// client drains them once per tick and applies them live (render distance, sound volumes, ...).
    pub option_changes: Vec<(String, String)>,
    /// Block states of the positions whose block entities the screens edit, `[x, y, z] -> global
    /// block state id` (`Block.BLOCK_STATE_REGISTRY`, which the data generator's blocks.json
    /// numbers the same). The client records them from its world before forwarding the packets
    /// that make Java build an entity (`block_entity_data`, `open_sign_editor`, a use of a
    /// game-master block); Java builds the entity's real state from it (`Block.stateById`), like
    /// vanilla's chunks build entities from the block state (`LevelChunk.setBlockState`).
    pub block_states: std::collections::HashMap<[i32; 3], i32>,
    /// The resource pack selection the pack screen committed (vanilla
    /// `Minecraft.reloadResourcePacks`), repository ids bottom first; the client rebuilds its
    /// asset stack from them. `None` = no change pending.
    pub resource_pack_ids: Option<Vec<String>>,
    /// The player's answer to the server resource pack prompt (vanilla's
    /// `PackConfirmScreen` callback), until the client takes it. `None` while
    /// the screen is open (or none was shown).
    pub pack_prompt_answer: Option<bool>,
}

impl Default for GameState {
    fn default() -> Self {
        GameState {
            inventory: vec![empty_stack(); INVENTORY_SIZE],
            carried: empty_stack(),
            inventory_changes: Vec::new(),
            selected_slot: 0,
            sent: Vec::new(),
            player_name: "Player".to_owned(),
            game_mode: GameMode::Survival,
            permission_level: 0,
            vitals: PlayerVitals::default(),
            mount: None,
            attack_strength: AttackStrengthHud::default(),
            crosshair_pick_living: false,
            debug: DebugInfo::default(),
            debug_shown: false,
            in_world: false,
            immediate_respawn: false,
            game_time: 0,
            day_time: 0,
            outgoing: Vec::new(),
            disconnect_requested: false,
            join_requested: None,
            join_cancelled: false,
            quit_requested: false,
            open_world_requested: None,
            create_world_requested: None,
            local_world_running: false,
            option_changes: Vec::new(),
            block_states: std::collections::HashMap::new(),
            resource_pack_ids: None,
            pack_prompt_answer: None,
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
        self.inventory[i] = stack.clone();
        self.inventory_changes.push((slot, stack));
    }

    /// Drains the inventory slots Java changed since the last call, oldest first.
    pub fn take_inventory_changes(&mut self) -> Vec<(i32, ItemStackData)> {
        std::mem::take(&mut self.inventory_changes)
    }
    /// Drains the messages sent by screens.
    pub fn take_sent(&mut self) -> Vec<ChatMessage> {
        std::mem::take(&mut self.sent)
    }
    /// Drains the serverbound packets vanilla code sent (VarInt id + body each).
    pub fn take_outgoing(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.outgoing)
    }
    /// Drains the option changes vanilla screens made (options.txt key/value pairs each), the
    /// way `haste.gui.OptionBridge` reported them.
    pub fn take_option_changes(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.option_changes)
    }
    /// Takes the resource pack selection the pack screen committed, if any.
    pub fn take_resource_pack_ids(&mut self) -> Option<Vec<String>> {
        self.resource_pack_ids.take()
    }

    /// The F3 overlay's string values as `\n`-joined `key value` lines for `Native.debugText`
    /// (`biome`, `dimension`, `brand`, `cpu`, `gpu`, then `block`/`fluid` with `pos`, `prop` and
    /// `tag` lines). Only pulled while the overlay is on, so it allocates on demand.
    pub fn debug_text(&self) -> String {
        let d = &self.debug;
        let mut s = String::with_capacity(160);
        let mut line = |k: &str, v: &str| {
            s.push_str(k);
            s.push(' ');
            s.push_str(v);
            s.push('\n');
        };
        line("biome", &d.biome);
        line("dimension", &d.dimension);
        line("brand", &d.brand);
        line("cpu", &d.cpu);
        line("gpu", &d.gpu);
        for (k, t) in [("block", &d.target_block), ("fluid", &d.target_fluid)] {
            let Some(t) = t else { continue };
            line(
                k,
                &format!("{} {} {} {}", t.name, t.pos[0], t.pos[1], t.pos[2]),
            );
            for (pk, pv) in &t.properties {
                line(&format!("{k}prop"), &format!("{pk}: {pv}"));
            }
            for tag in &t.tags {
                line(&format!("{k}tag"), tag);
            }
        }
        s
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
        // Every mirrored change is queued for the client to drain, oldest first.
        assert_eq!(
            s.take_inventory_changes(),
            vec![(3, s.slot(3).cloned().unwrap()), (50, empty_stack())]
        );
        s.set_mirror(3, empty_stack());
        assert_eq!(
            s.take_inventory_changes(),
            vec![(3, empty_stack())],
            "clearing a slot is a change too (a creative pickup empties the hotbar slot)"
        );
        assert!(s.take_inventory_changes().is_empty(), "drained");
    }
}
