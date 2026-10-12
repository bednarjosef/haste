//! Serverbound play packets.
//!
//! Layouts from the minecraft.wiki "Java Edition protocol/Packets" page for
//! 26.3. Note: 26.3 has no serverbound `swing` packet; arm swings are
//! `punch` (no fields) and attacks are `attack`.

use bytes::Bytes;
use haste_core::{BlockPos, DVec3, Identifier};
use uuid::Uuid;

use super::clientbound::{ChatSession, GameRuleValue};
pub use crate::packets::configuration::serverbound::{
    ClientInformation, CookieResponse, CustomClickAction, CustomPayload, KeepAlive, Pong,
    ResourcePack,
};
use crate::packets::{packet, packet_enum, Packet};
use crate::wire::{DecodeResult, List, LpVec3, Opt, Reader, Rest, VarInt, VarLong, Wire, Writer};

packet_enum! {
    /// Serverbound play packets, in `packets.json` id order.
    ServerboundPlay {
        AcceptTeleportation, Attack, BlockEntityTagQuery, BundleItemSelected, ChangeDifficulty, ChangeGameMode,
        ChatAck, ChatCommand, ChatCommandSigned, Chat, ChatSessionUpdate, ChunkBatchReceived, ClientCommand,
        ClientTickEnd, ClientInformation, CommandSuggestion, ConfigurationAcknowledged, ContainerButtonClick,
        ContainerClick, ContainerClose, ContainerSlotStateChanged, CookieResponse, CustomPayload,
        DebugSubscriptionRequest, EditBook, EntityTagQuery, Interact, JigsawGenerate, KeepAlive, LockDifficulty,
        MovePlayerPos, MovePlayerPosRot, MovePlayerRot, MovePlayerStatusOnly, MoveVehicle, PaddleBoat,
        PickItemFromBlock, PickItemFromEntity, PingRequest, PlaceRecipe, PlayerAbilities, PlayerAction, PlayerCommand,
        PlayerInput, PlayerLoaded, Pong, Punch, RecipeBookChangeSettings, RecipeBookSeenRecipe, RenameItem,
        ResourcePack, SeenAdvancements, SelectTrade, SetBeacon, SetCarriedItem, SetCommandBlock, SetCommandMinecart,
        SetCreativeModeSlot, SetGameRule, SetJigsawBlock, SetStructureBlock, SetTestBlock, SignUpdate,
        SpectatorAction, TeleportToEntity, TestInstanceBlockAction, UseItemOn, UseItem, CustomClickAction,
    }
}

/// Movement flag bits for the `move_player_*` packets.
pub mod move_flags {
    /// On ground.
    pub const ON_GROUND: u8 = 0x01;
    /// Pushing against a wall (horizontal collision).
    pub const HORIZONTAL_COLLISION: u8 = 0x02;
}

packet! {
    /// Confirms a `player_position` teleport; in 26.3 it also reports the
    /// resulting position and rotation.
    AcceptTeleportation = "minecraft:accept_teleportation" {
        /// The teleport id.
        teleport_id: i32 as VarInt,
        /// Resulting position.
        pos: DVec3,
        /// Resulting yaw.
        yaw: f32,
        /// Resulting pitch.
        pitch: f32,
    }
}

packet! {
    /// Attacks an entity.
    Attack = "minecraft:attack" {
        /// Target.
        entity_id: i32 as VarInt,
    }
}

packet! {
    /// Acknowledges received signed chat messages.
    ChatAck = "minecraft:chat_ack" {
        /// Messages seen since the last acknowledgement.
        message_count: i32 as VarInt,
    }
}

packet! {
    /// An unsigned command (without the `/`).
    ChatCommand = "minecraft:chat_command" {
        /// Command text.
        command: String,
    }
}

/// A chat message. Haste sends unsigned messages (`signature: None`).
#[derive(Clone, Debug, PartialEq)]
pub struct Chat {
    /// Text (at most 256 chars).
    pub message: String,
    /// Milliseconds since the epoch.
    pub timestamp: i64,
    /// Salt.
    pub salt: i64,
    /// 256-byte signature.
    pub signature: Option<Bytes>,
    /// Messages seen since the last chat packet.
    pub message_count: i32,
    /// Fixed bit set (20 bits, 3 bytes) of acknowledged messages.
    pub acknowledged: [u8; 3],
    /// Checksum of the last-seen signatures (0 when none).
    pub checksum: i8,
}
impl Packet for Chat {
    const NAME: &'static str = "minecraft:chat";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let message = r.string_max(256)?;
        let timestamp = r.i64()?;
        let salt = r.i64()?;
        let signature = r.option(|r| Ok(Bytes::copy_from_slice(r.take(256)?)))?;
        let message_count = r.var_int()?;
        let a = r.take(3)?;
        Ok(Self {
            message,
            timestamp,
            salt,
            signature,
            message_count,
            acknowledged: [a[0], a[1], a[2]],
            checksum: r.i8()?,
        })
    }
    fn encode(&self, w: &mut Writer) {
        w.string(&self.message);
        w.i64(self.timestamp);
        w.i64(self.salt);
        w.option(&self.signature, |w, s| w.bytes(s));
        w.var_int(self.message_count);
        w.bytes(&self.acknowledged);
        w.i8(self.checksum);
    }
}

packet! {
    /// Acknowledges a chunk batch and asks for a send rate.
    ChunkBatchReceived = "minecraft:chunk_batch_received" {
        /// Desired chunks per tick.
        chunks_per_tick: f32,
    }
}

packet! {
    /// End of a client tick.
    ClientTickEnd = "minecraft:client_tick_end" {}
}

packet! {
    /// Acknowledges `start_configuration`.
    ConfigurationAcknowledged = "minecraft:configuration_acknowledged" {}
}

packet! {
    /// Right-click on an entity.
    Interact = "minecraft:interact" {
        /// Target.
        entity_id: i32 as VarInt,
        /// 0 main hand, 1 off hand.
        hand: i32 as VarInt,
        /// Cursor offset from the entity position.
        target_offset: DVec3 as LpVec3,
        /// Sneak key pressed.
        sneaking: bool,
    }
}

packet! {
    /// Position update.
    MovePlayerPos = "minecraft:move_player_pos" {
        /// Feet position.
        pos: DVec3,
        /// [`move_flags`] bits.
        flags: u8,
    }
}

packet! {
    /// Position and rotation update.
    MovePlayerPosRot = "minecraft:move_player_pos_rot" {
        /// Feet position.
        pos: DVec3,
        /// Yaw.
        yaw: f32,
        /// Pitch.
        pitch: f32,
        /// [`move_flags`] bits.
        flags: u8,
    }
}

packet! {
    /// Rotation update.
    MovePlayerRot = "minecraft:move_player_rot" {
        /// Yaw.
        yaw: f32,
        /// Pitch.
        pitch: f32,
        /// [`move_flags`] bits.
        flags: u8,
    }
}

packet! {
    /// Flags-only update.
    MovePlayerStatusOnly = "minecraft:move_player_status_only" {
        /// [`move_flags`] bits.
        flags: u8,
    }
}

/// `player_action` status values. 26.3 inserted `CHANGE_DESTROY_DIRECTION`
/// at 1 and renumbered the rest (observed on the 26.3 server by the player
/// workstream; minecraft.wiki still lists the old order).
pub mod player_action_status {
    /// Start breaking a block.
    pub const START_DESTROY_BLOCK: i32 = 0;
    /// The face being broken changed (new in 26.3).
    pub const CHANGE_DESTROY_DIRECTION: i32 = 1;
    /// Cancel breaking.
    pub const ABORT_DESTROY_BLOCK: i32 = 2;
    /// Finish breaking.
    pub const STOP_DESTROY_BLOCK: i32 = 3;
    /// Drop the whole held stack.
    pub const DROP_ALL_ITEMS: i32 = 4;
    /// Drop one held item.
    pub const DROP_ITEM: i32 = 5;
    /// Release the item in use (bow, shield, ...).
    pub const RELEASE_USE_ITEM: i32 = 6;
    /// Swap main and off hand.
    pub const SWAP_ITEM_WITH_OFFHAND: i32 = 7;
    /// Spear stab.
    pub const STAB: i32 = 8;
}

packet! {
    /// Digging, dropping, swapping hands.
    PlayerAction = "minecraft:player_action" {
        /// A [`player_action_status`] value (26.3 order).
        status: i32 as VarInt,
        /// Block.
        pos: BlockPos,
        /// Face (Direction ordinal).
        face: u8,
        /// Block change sequence.
        sequence: i32 as VarInt,
    }
}

packet! {
    /// Sprinting, leaving bed, horse jumps.
    PlayerCommand = "minecraft:player_command" {
        /// Player entity id (ignored by vanilla).
        entity_id: i32 as VarInt,
        /// 0 leave bed, 1 start sprint, 2 stop sprint, 3/4 horse jump, 5 vehicle inventory, 6 elytra.
        action: i32 as VarInt,
        /// Horse jump boost 0..100.
        jump_boost: i32 as VarInt,
    }
}

/// `player_input` flag bits.
pub mod input_flags {
    #![allow(missing_docs)]
    pub const FORWARD: u8 = 0x01;
    pub const BACKWARD: u8 = 0x02;
    pub const LEFT: u8 = 0x04;
    pub const RIGHT: u8 = 0x08;
    pub const JUMP: u8 = 0x10;
    pub const SNEAK: u8 = 0x20;
    pub const SPRINT: u8 = 0x40;
}

packet! {
    /// Held movement keys.
    PlayerInput = "minecraft:player_input" {
        /// [`input_flags`] bits.
        flags: u8,
    }
}

packet! {
    /// The client finished loading terrain.
    PlayerLoaded = "minecraft:player_loaded" {}
}

packet! {
    /// Arm swing.
    Punch = "minecraft:punch" {}
}

packet! {
    /// Selected hotbar slot.
    SetCarriedItem = "minecraft:set_carried_item" {
        /// Slot 0..8.
        slot: i16,
    }
}

packet! {
    /// Use the held item on a block.
    UseItemOn = "minecraft:use_item_on" {
        /// 0 main hand, 1 off hand.
        hand: i32 as VarInt,
        /// Block.
        pos: BlockPos,
        /// Face (Direction ordinal).
        face: i32 as VarInt,
        /// Cursor x in the block, 0..1.
        cursor_x: f32,
        /// Cursor y.
        cursor_y: f32,
        /// Cursor z.
        cursor_z: f32,
        /// Head inside a block.
        inside_block: bool,
        /// World border hit.
        world_border_hit: bool,
        /// Block change sequence.
        sequence: i32 as VarInt,
    }
}

packet! {
    /// Use the held item.
    UseItem = "minecraft:use_item" {
        /// 0 main hand, 1 off hand.
        hand: i32 as VarInt,
        /// Block change sequence.
        sequence: i32 as VarInt,
        /// Yaw.
        yaw: f32,
        /// Pitch.
        pitch: f32,
    }
}

// ---------------------------------------------------------------- remaining serverbound packets

packet! {
    /// F3+I on a block.
    BlockEntityTagQuery = "minecraft:block_entity_tag_query" {
        /// Transaction id.
        transaction_id: i32 as VarInt,
        /// Block.
        pos: BlockPos,
    }
}

packet! {
    /// Selects an item inside a bundle.
    BundleItemSelected = "minecraft:bundle_item_selected" {
        /// Inventory slot of the bundle.
        slot: i32 as VarInt,
        /// Slot inside the bundle.
        slot_in_bundle: i32 as VarInt,
    }
}

packet! {
    /// Changes the difficulty (operators).
    ChangeDifficulty = "minecraft:change_difficulty" {
        /// 0 peaceful .. 3 hard.
        difficulty: u8,
    }
}

packet! {
    /// Requests a game mode (F3+F4).
    ChangeGameMode = "minecraft:change_game_mode" {
        /// Game mode id.
        game_mode: i32 as VarInt,
    }
}

/// A signed command argument.
#[derive(Clone, Debug, PartialEq)]
pub struct ArgumentSignature {
    /// Argument name.
    pub name: String,
    /// 256-byte signature.
    pub signature: Bytes,
}

/// A command with signed arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatCommandSigned {
    /// Command without `/`.
    pub command: String,
    /// Milliseconds since the epoch.
    pub timestamp: i64,
    /// Salt.
    pub salt: i64,
    /// Argument signatures (at most 8).
    pub arguments: Vec<ArgumentSignature>,
    /// Messages seen.
    pub message_count: i32,
    /// Fixed bit set (20 bits).
    pub acknowledged: [u8; 3],
    /// Checksum.
    pub checksum: i8,
}
impl Packet for ChatCommandSigned {
    const NAME: &'static str = "minecraft:chat_command_signed";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let command = r.string()?;
        let timestamp = r.i64()?;
        let salt = r.i64()?;
        let arguments = r.list(8, |r| {
            Ok(ArgumentSignature {
                name: r.string_max(16)?,
                signature: Bytes::copy_from_slice(r.take(256)?),
            })
        })?;
        let message_count = r.var_int()?;
        let a = r.take(3)?;
        Ok(Self {
            command,
            timestamp,
            salt,
            arguments,
            message_count,
            acknowledged: [a[0], a[1], a[2]],
            checksum: r.i8()?,
        })
    }
    fn encode(&self, w: &mut Writer) {
        w.string(&self.command);
        w.i64(self.timestamp);
        w.i64(self.salt);
        w.list(&self.arguments, |w, a| {
            w.string(&a.name);
            w.bytes(&a.signature);
        });
        w.var_int(self.message_count);
        w.bytes(&self.acknowledged);
        w.i8(self.checksum);
    }
}

packet! {
    /// Chat session public key (signed chat).
    ChatSessionUpdate = "minecraft:chat_session_update" {
        /// Session.
        session: ChatSession,
    }
}

packet! {
    /// 0 perform respawn, 1 request stats, 2 request game rule values.
    ClientCommand = "minecraft:client_command" {
        /// Action id.
        action: i32 as VarInt,
    }
}

packet! {
    /// Tab-completion request.
    CommandSuggestion = "minecraft:command_suggestion" {
        /// Transaction id.
        id: i32 as VarInt,
        /// Text before the cursor.
        text: String,
    }
}

packet! {
    /// Container button (enchanting, lectern, stonecutter, loom).
    ContainerButtonClick = "minecraft:container_button_click" {
        /// Window id.
        window_id: i32 as VarInt,
        /// Button id.
        button: i32 as VarInt,
    }
}

/// `Hashed Slot`: an item whose component values are CRC32C hashes.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct HashedSlot {
    /// `(item id, count, added (type, hash), removed types)`, `None` for empty.
    pub item: Option<HashedItem>,
}

/// The non-empty part of a [`HashedSlot`].
#[derive(Clone, Debug, PartialEq)]
pub struct HashedItem {
    /// Index into `minecraft:item`.
    pub item: i32,
    /// Count.
    pub count: i32,
    /// Added components: type id and CRC32C hash.
    pub added: Vec<(i32, i32)>,
    /// Removed component type ids.
    pub removed: Vec<i32>,
}
impl Wire for HashedSlot {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        if !r.bool()? {
            return Ok(Self { item: None });
        }
        Ok(Self {
            item: Some(HashedItem {
                item: r.var_int()?,
                count: r.var_int()?,
                added: r.list(256, |r| Ok((r.var_int()?, r.i32()?)))?,
                removed: r.read_as::<_, List<VarInt>>()?,
            }),
        })
    }
    fn write(&self, w: &mut Writer) {
        match &self.item {
            None => w.bool(false),
            Some(i) => {
                w.bool(true);
                w.var_int(i.item);
                w.var_int(i.count);
                w.list(&i.added, |w, (t, h)| {
                    w.var_int(*t);
                    w.i32(*h);
                });
                w.write_as::<_, List<VarInt>>(&i.removed);
            }
        }
    }
}

/// A changed slot in `container_click`.
#[derive(Clone, Debug, PartialEq)]
pub struct ChangedSlot {
    /// Slot index.
    pub slot: i16,
    /// New content in the client's view.
    pub item: HashedSlot,
}
impl Wire for ChangedSlot {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            slot: r.i16()?,
            item: r.read()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.i16(self.slot);
        w.write(&self.item);
    }
}

/// A click in a container.
#[derive(Clone, Debug, PartialEq)]
pub struct ContainerClick {
    /// Window id.
    pub window_id: i32,
    /// Last state id received.
    pub state_id: i32,
    /// Clicked slot (-999 outside).
    pub slot: i16,
    /// Button.
    pub button: i8,
    /// Mode.
    pub mode: i32,
    /// Changed slots (at most 128).
    pub changed: Vec<ChangedSlot>,
    /// Cursor item.
    pub carried: HashedSlot,
}
impl Packet for ContainerClick {
    const NAME: &'static str = "minecraft:container_click";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            window_id: r.var_int()?,
            state_id: r.var_int()?,
            slot: r.i16()?,
            button: r.i8()?,
            mode: r.var_int()?,
            changed: r.list(128, ChangedSlot::read)?,
            carried: r.read()?,
        })
    }
    fn encode(&self, w: &mut Writer) {
        w.var_int(self.window_id);
        w.var_int(self.state_id);
        w.i16(self.slot);
        w.i8(self.button);
        w.var_int(self.mode);
        w.write(&self.changed);
        w.write(&self.carried);
    }
}

packet! {
    /// Closes a container.
    ContainerClose = "minecraft:container_close" {
        /// Window id.
        window_id: i32 as VarInt,
    }
}

packet! {
    /// Toggles a crafter slot.
    ContainerSlotStateChanged = "minecraft:container_slot_state_changed" {
        /// Slot.
        slot: i32 as VarInt,
        /// Window id.
        window_id: i32 as VarInt,
        /// Enabled.
        enabled: bool,
    }
}

packet! {
    /// Active debug subscriptions.
    DebugSubscriptionRequest = "minecraft:debug_subscription_request" {
        /// Ids in `minecraft:debug_subscription`.
        subscriptions: Vec<i32> as List<VarInt>,
    }
}

packet! {
    /// Edits or signs a book.
    EditBook = "minecraft:edit_book" {
        /// Hotbar slot.
        slot: i32 as VarInt,
        /// Pages.
        pages: Vec<String>,
        /// Title when signing.
        title: Option<String>,
    }
}

packet! {
    /// F3+I on an entity.
    EntityTagQuery = "minecraft:entity_tag_query" {
        /// Transaction id.
        transaction_id: i32 as VarInt,
        /// Entity.
        entity_id: i32 as VarInt,
    }
}

packet! {
    /// Jigsaw generate button.
    JigsawGenerate = "minecraft:jigsaw_generate" {
        /// Block.
        pos: BlockPos,
        /// Levels.
        levels: i32 as VarInt,
        /// Keep jigsaws.
        keep_jigsaws: bool,
    }
}

packet! {
    /// Locks the difficulty.
    LockDifficulty = "minecraft:lock_difficulty" {
        /// Locked.
        locked: bool,
    }
}

packet! {
    /// Client-controlled vehicle movement.
    MoveVehicle = "minecraft:move_vehicle" {
        /// Position.
        pos: DVec3,
        /// Yaw.
        yaw: f32,
        /// Pitch.
        pitch: f32,
        /// On ground.
        on_ground: bool,
    }
}

packet! {
    /// Boat paddle state.
    PaddleBoat = "minecraft:paddle_boat" {
        /// Left paddle turning.
        left: bool,
        /// Right paddle turning.
        right: bool,
    }
}

packet! {
    /// Pick block on a block.
    PickItemFromBlock = "minecraft:pick_item_from_block" {
        /// Block.
        pos: BlockPos,
        /// Include block data (creative).
        include_data: bool,
    }
}

packet! {
    /// Pick block on an entity.
    PickItemFromEntity = "minecraft:pick_item_from_entity" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// Include data.
        include_data: bool,
    }
}

packet! {
    /// Latency ping; answered with `pong_response`.
    PingRequest = "minecraft:ping_request" {
        /// Payload.
        payload: i64,
    }
}

packet! {
    /// Recipe book click.
    PlaceRecipe = "minecraft:place_recipe" {
        /// Window id.
        window_id: i32 as VarInt,
        /// Recipe display id.
        recipe: i32 as VarInt,
        /// Shift held.
        make_all: bool,
    }
}

packet! {
    /// Flying state.
    PlayerAbilities = "minecraft:player_abilities" {
        /// 0x02 flying.
        flags: i8,
    }
}

packet! {
    /// Recipe book settings.
    RecipeBookChangeSettings = "minecraft:recipe_book_change_settings" {
        /// 0 crafting, 1 furnace, 2 blast furnace, 3 smoker.
        book: i32 as VarInt,
        /// Open.
        open: bool,
        /// Filter active.
        filter: bool,
    }
}

packet! {
    /// Marks a recipe as seen.
    RecipeBookSeenRecipe = "minecraft:recipe_book_seen_recipe" {
        /// Recipe display id.
        recipe: i32 as VarInt,
    }
}

packet! {
    /// Anvil rename text.
    RenameItem = "minecraft:rename_item" {
        /// New name.
        name: String,
    }
}

/// Advancement screen tab changes.
#[derive(Clone, Debug, PartialEq)]
pub struct SeenAdvancements {
    /// 0 opened tab, 1 closed screen.
    pub action: i32,
    /// Tab (only for action 0).
    pub tab: Option<Identifier>,
}
impl Packet for SeenAdvancements {
    const NAME: &'static str = "minecraft:seen_advancements";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let action = r.var_int()?;
        let tab = if action == 0 {
            Some(r.identifier()?)
        } else {
            None
        };
        Ok(Self { action, tab })
    }
    fn encode(&self, w: &mut Writer) {
        w.var_int(self.action);
        if let Some(t) = &self.tab {
            w.identifier(t);
        }
    }
}

packet! {
    /// Selects a villager trade.
    SelectTrade = "minecraft:select_trade" {
        /// Trade index.
        slot: i32 as VarInt,
    }
}

packet! {
    /// Beacon effects.
    SetBeacon = "minecraft:set_beacon" {
        /// Primary effect.
        primary: Option<i32> as Opt<VarInt>,
        /// Secondary effect.
        secondary: Option<i32> as Opt<VarInt>,
    }
}

packet! {
    /// Programs a command block.
    SetCommandBlock = "minecraft:set_command_block" {
        /// Block.
        pos: BlockPos,
        /// Command.
        command: String,
        /// 0 chain, 1 repeating, 2 impulse.
        mode: i32 as VarInt,
        /// 0x01 track output, 0x02 conditional, 0x04 automatic.
        flags: i8,
    }
}

packet! {
    /// Programs a command block minecart.
    SetCommandMinecart = "minecraft:set_command_minecart" {
        /// Minecart.
        entity_id: i32 as VarInt,
        /// Command.
        command: String,
        /// Track output.
        track_output: bool,
    }
}

/// An item stack in the "untrusted" encoding where every added component is
/// length-prefixed (as Pumpkin reads `set_creative_mode_slot`; the wiki says
/// plain `Slot`): uncertain, see NOTES.md.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct UntrustedItemStack {
    /// Count (0 = empty, nothing else present).
    pub count: i32,
    /// Index into `minecraft:item`.
    pub item: i32,
    /// Added components: type id and raw data.
    pub added: Vec<(i32, Bytes)>,
    /// Removed component type ids.
    pub removed: Vec<i32>,
}
impl Wire for UntrustedItemStack {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let count = r.var_int()?;
        if count <= 0 {
            return Ok(Self::default());
        }
        let item = r.var_int()?;
        let adds = r.len_prefix(256)?;
        let removes = r.len_prefix(256)?;
        let mut added = Vec::with_capacity(adds.min(r.remaining()));
        for _ in 0..adds {
            added.push((
                r.var_int()?,
                r.byte_array(crate::transport::MAX_PACKET_DATA_SIZE)?,
            ));
        }
        let mut removed = Vec::with_capacity(removes.min(r.remaining()));
        for _ in 0..removes {
            removed.push(r.var_int()?);
        }
        Ok(Self {
            count,
            item,
            added,
            removed,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.var_int(self.count);
        if self.count <= 0 {
            return;
        }
        w.var_int(self.item);
        w.len_prefix(self.added.len());
        w.len_prefix(self.removed.len());
        for (t, d) in &self.added {
            w.var_int(*t);
            w.byte_array(d);
        }
        for t in &self.removed {
            w.var_int(*t);
        }
    }
}

packet! {
    /// Creative inventory slot set.
    SetCreativeModeSlot = "minecraft:set_creative_mode_slot" {
        /// Slot.
        slot: i16,
        /// Item.
        item: UntrustedItemStack,
    }
}

packet! {
    /// Sets game rules (operators).
    SetGameRule = "minecraft:set_game_rule" {
        /// Rules.
        rules: Vec<GameRuleValue>,
    }
}

packet! {
    /// Programs a jigsaw block.
    SetJigsawBlock = "minecraft:set_jigsaw_block" {
        /// Block.
        pos: BlockPos,
        /// Name.
        name: Identifier,
        /// Target.
        target: Identifier,
        /// Pool.
        pool: Identifier,
        /// Final state.
        final_state: String,
        /// `rollable` or `aligned`.
        joint_type: String,
        /// Selection priority.
        selection_priority: i32 as VarInt,
        /// Placement priority.
        placement_priority: i32 as VarInt,
    }
}

packet! {
    /// Programs a structure block.
    SetStructureBlock = "minecraft:set_structure_block" {
        /// Block.
        pos: BlockPos,
        /// Action.
        action: i32 as VarInt,
        /// 0 save, 1 load, 2 corner, 3 data.
        mode: i32 as VarInt,
        /// Structure name.
        name: String,
        /// Offset x.
        offset_x: i8,
        /// Offset y.
        offset_y: i8,
        /// Offset z.
        offset_z: i8,
        /// Size x.
        size_x: i8,
        /// Size y.
        size_y: i8,
        /// Size z.
        size_z: i8,
        /// Mirror.
        mirror: i32 as VarInt,
        /// Rotation.
        rotation: i32 as VarInt,
        /// Metadata.
        metadata: String,
        /// Integrity 0..1.
        integrity: f32,
        /// Seed.
        seed: i64 as VarLong,
        /// 0x01 ignore entities, 0x02 show air, 0x04 show bounding box, 0x08 strict.
        flags: i8,
    }
}

packet! {
    /// Updates a test block.
    SetTestBlock = "minecraft:set_test_block" {
        /// Block.
        pos: BlockPos,
        /// 0 start, 1 log, 2 fail, 3 accept.
        mode: i32 as VarInt,
        /// Message.
        message: String,
    }
}

packet! {
    /// Sign text (26.3: the text slot follows the lines).
    SignUpdate = "minecraft:sign_update" {
        /// Sign.
        pos: BlockPos,
        /// Line 1.
        line1: String,
        /// Line 2.
        line2: String,
        /// Line 3.
        line3: String,
        /// Line 4.
        line4: String,
        /// 0 back, 1 front.
        text_slot: i32 as VarInt,
    }
}

packet! {
    /// Spectator left click.
    SpectatorAction = "minecraft:spectator_action" {
        /// Targeted entity id + 1, 0 for none.
        entity: i32 as VarInt,
    }
}

packet! {
    /// Spectator teleport to an entity.
    TeleportToEntity = "minecraft:teleport_to_entity" {
        /// Target UUID.
        target: Uuid,
    }
}

packet! {
    /// Test instance block action. Kept raw: the wiki (error message as a
    /// text component) and Pumpkin (as a string) disagree.
    TestInstanceBlockAction = "minecraft:test_instance_block_action" {
        /// Raw body.
        body: Bytes as Rest,
    }
}

#[cfg(test)]
mod more_tests {
    use super::*;
    use crate::packets::test_util::round_trip;

    #[test]
    fn round_trips() {
        round_trip(&BlockEntityTagQuery {
            transaction_id: 1,
            pos: BlockPos::new(1, 2, 3),
        });
        round_trip(&BundleItemSelected {
            slot: 3,
            slot_in_bundle: -1,
        });
        round_trip(&ChangeDifficulty { difficulty: 2 });
        round_trip(&ChangeGameMode { game_mode: 1 });
        round_trip(&ChatCommandSigned {
            command: "msg a hi".into(),
            timestamp: 1,
            salt: 2,
            arguments: vec![ArgumentSignature {
                name: "message".into(),
                signature: Bytes::from(vec![3; 256]),
            }],
            message_count: 0,
            acknowledged: [0; 3],
            checksum: 0,
        });
        round_trip(&ChatSessionUpdate {
            session: ChatSession {
                session_id: Uuid::from_u128(1),
                expires_at: 2,
                public_key: Bytes::from_static(&[1, 2]),
                key_signature: Bytes::from_static(&[3]),
            },
        });
        round_trip(&ClientCommand { action: 0 });
        round_trip(&CommandSuggestion {
            id: 1,
            text: "/tp ".into(),
        });
        round_trip(&ContainerButtonClick {
            window_id: 1,
            button: 2,
        });
        round_trip(&ContainerClick {
            window_id: 0,
            state_id: 5,
            slot: 36,
            button: 0,
            mode: 0,
            changed: vec![ChangedSlot {
                slot: 36,
                item: HashedSlot {
                    item: Some(HashedItem {
                        item: 1,
                        count: 64,
                        added: vec![(3, 12345)],
                        removed: vec![4],
                    }),
                },
            }],
            carried: HashedSlot::default(),
        });
        round_trip(&ContainerClose { window_id: 0 });
        round_trip(&ContainerSlotStateChanged {
            slot: 1,
            window_id: 2,
            enabled: true,
        });
        round_trip(&DebugSubscriptionRequest {
            subscriptions: vec![0, 3],
        });
        round_trip(&EditBook {
            slot: 0,
            pages: vec!["p1".into()],
            title: Some("t".into()),
        });
        round_trip(&EntityTagQuery {
            transaction_id: 1,
            entity_id: 2,
        });
        round_trip(&JigsawGenerate {
            pos: BlockPos::new(0, 0, 0),
            levels: 3,
            keep_jigsaws: true,
        });
        round_trip(&LockDifficulty { locked: true });
        round_trip(&MoveVehicle {
            pos: DVec3::ONE,
            yaw: 1.0,
            pitch: 2.0,
            on_ground: false,
        });
        round_trip(&PaddleBoat {
            left: true,
            right: false,
        });
        round_trip(&PickItemFromBlock {
            pos: BlockPos::new(1, 1, 1),
            include_data: false,
        });
        round_trip(&PickItemFromEntity {
            entity_id: 5,
            include_data: true,
        });
        round_trip(&PingRequest { payload: 9 });
        round_trip(&PlaceRecipe {
            window_id: 1,
            recipe: 2,
            make_all: false,
        });
        round_trip(&PlayerAbilities { flags: 2 });
        round_trip(&RecipeBookChangeSettings {
            book: 0,
            open: true,
            filter: false,
        });
        round_trip(&RecipeBookSeenRecipe { recipe: 3 });
        round_trip(&RenameItem { name: "x".into() });
        round_trip(&SeenAdvancements {
            action: 0,
            tab: Some(Identifier::minecraft("story/root")),
        });
        round_trip(&SeenAdvancements {
            action: 1,
            tab: None,
        });
        round_trip(&SelectTrade { slot: 2 });
        round_trip(&SetBeacon {
            primary: Some(1),
            secondary: None,
        });
        round_trip(&SetCommandBlock {
            pos: BlockPos::new(0, 0, 0),
            command: "say hi".into(),
            mode: 2,
            flags: 1,
        });
        round_trip(&SetCommandMinecart {
            entity_id: 1,
            command: "say".into(),
            track_output: true,
        });
        round_trip(&SetCreativeModeSlot {
            slot: 36,
            item: UntrustedItemStack {
                count: 1,
                item: 5,
                added: vec![(3, Bytes::from_static(&[7]))],
                removed: vec![2],
            },
        });
        round_trip(&SetCreativeModeSlot {
            slot: -1,
            item: UntrustedItemStack::default(),
        });
        round_trip(&SetGameRule {
            rules: vec![GameRuleValue {
                name: Identifier::minecraft("keep_inventory"),
                value: "true".into(),
            }],
        });
        round_trip(&SetJigsawBlock {
            pos: BlockPos::new(1, 2, 3),
            name: Identifier::minecraft("a"),
            target: Identifier::minecraft("b"),
            pool: Identifier::minecraft("c"),
            final_state: "minecraft:air".into(),
            joint_type: "rollable".into(),
            selection_priority: 0,
            placement_priority: 0,
        });
        round_trip(&SetStructureBlock {
            pos: BlockPos::new(0, 0, 0),
            action: 0,
            mode: 0,
            name: "s".into(),
            offset_x: 0,
            offset_y: 1,
            offset_z: -1,
            size_x: 5,
            size_y: 5,
            size_z: 5,
            mirror: 0,
            rotation: 0,
            metadata: String::new(),
            integrity: 1.0,
            seed: 0,
            flags: 4,
        });
        round_trip(&SetTestBlock {
            pos: BlockPos::new(0, 0, 0),
            mode: 1,
            message: "m".into(),
        });
        round_trip(&SignUpdate {
            pos: BlockPos::new(1, 2, 3),
            line1: "a".into(),
            line2: String::new(),
            line3: String::new(),
            line4: "d".into(),
            text_slot: 1,
        });
        round_trip(&SpectatorAction { entity: 0 });
        round_trip(&TeleportToEntity {
            target: Uuid::from_u128(4),
        });
        round_trip(&TestInstanceBlockAction {
            body: Bytes::from_static(&[1, 2]),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::test_util::round_trip;

    #[test]
    fn round_trips() {
        round_trip(&AcceptTeleportation {
            teleport_id: 1,
            pos: DVec3::new(0.5, 64.0, 0.5),
            yaw: 90.0,
            pitch: 0.0,
        });
        round_trip(&Attack { entity_id: 5 });
        round_trip(&ChatAck { message_count: 2 });
        round_trip(&ChatCommand {
            command: "time set day".into(),
        });
        round_trip(&Chat {
            message: "hi".into(),
            timestamp: 1,
            salt: 2,
            signature: None,
            message_count: 0,
            acknowledged: [0; 3],
            checksum: 0,
        });
        round_trip(&Chat {
            message: "hi".into(),
            timestamp: 1,
            salt: 2,
            signature: Some(Bytes::from(vec![1; 256])),
            message_count: 1,
            acknowledged: [1, 0, 0],
            checksum: 5,
        });
        round_trip(&ChunkBatchReceived {
            chunks_per_tick: 64.0,
        });
        round_trip(&ClientTickEnd {});
        round_trip(&ClientInformation::defaults("en_us", 8));
        round_trip(&ConfigurationAcknowledged {});
        round_trip(&Interact {
            entity_id: 1,
            hand: 0,
            target_offset: DVec3::ZERO,
            sneaking: false,
        });
        round_trip(&MovePlayerPos {
            pos: DVec3::new(1.0, 2.0, 3.0),
            flags: move_flags::ON_GROUND,
        });
        round_trip(&MovePlayerPosRot {
            pos: DVec3::ZERO,
            yaw: 1.0,
            pitch: 2.0,
            flags: 0,
        });
        round_trip(&MovePlayerRot {
            yaw: 1.0,
            pitch: 2.0,
            flags: 3,
        });
        round_trip(&MovePlayerStatusOnly { flags: 1 });
        round_trip(&PlayerAction {
            status: 0,
            pos: BlockPos::new(1, 2, 3),
            face: 1,
            sequence: 4,
        });
        round_trip(&PlayerCommand {
            entity_id: 1,
            action: 1,
            jump_boost: 0,
        });
        round_trip(&PlayerInput {
            flags: input_flags::FORWARD | input_flags::SPRINT,
        });
        round_trip(&PlayerLoaded {});
        round_trip(&Punch {});
        round_trip(&SetCarriedItem { slot: 3 });
        round_trip(&UseItemOn {
            hand: 0,
            pos: BlockPos::new(0, 64, 0),
            face: 1,
            cursor_x: 0.5,
            cursor_y: 1.0,
            cursor_z: 0.5,
            inside_block: false,
            world_border_hit: false,
            sequence: 7,
        });
        round_trip(&UseItem {
            hand: 1,
            sequence: 8,
            yaw: 0.0,
            pitch: 0.0,
        });
    }
}
