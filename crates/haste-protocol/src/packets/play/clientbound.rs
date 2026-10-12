//! Clientbound play packets.
//!
//! Layouts from the minecraft.wiki "Java Edition protocol/Packets" page for
//! 26.3; see NOTES.md for what was checked against the local server. Packets
//! whose body we do not interpret yet keep it raw (`body: Bytes`).

use bytes::Bytes;
use haste_core::chunk::{BlockChange, BlockEntityData as ChunkBlockEntity, ChunkData, LightData};
use haste_core::entity::{EntitySpawn, ItemStackData, MetadataEntry};
use haste_core::{BlockPos, BlockStateId, ChunkPos, Component, DVec3, EntityId, Identifier, RegistryIndex, SectionPos};
use uuid::Uuid;

pub use crate::packets::configuration::clientbound::{
    ClearDialog, CookieRequest, CustomPayload, CustomReportDetails, Disconnect, KeepAlive, Ping, PostEffects,
    ResourcePackPop, ResourcePackPush, ServerLinks, StoreCookie, Transfer, UpdateTags,
};
use crate::metadata::Metadata;
use crate::packets::{packet, packet_enum, Packet};
use crate::wire::{
    Angle, ByteArray, Codec, DecodeError, DecodeResult, IdOr, List, LpVec3, Opt, ProfileProperty, RawNbt, Reader,
    Rest, SoundEvent, VarInt, VarLong, Wire, Writer,
};

packet_enum! {
    /// Clientbound play packets. Packets without a struct here arrive as
    /// `Unknown`.
    ClientboundPlay {
        BundleDelimiter, AddEntity, BlockEntityData, BlockUpdate, ChangeDifficulty, ChunkBatchFinished,
        ChunkBatchStart, Commands, ContainerSetContent, ContainerSetSlot, CookieRequest, CustomPayload,
        Disconnect, DisguisedChat, EntityPositionSync, ForgetLevelChunk, GameEvent, KeepAlive,
        LevelChunkWithLight, LevelEvent, LightUpdate, Login, MoveEntityPos, MoveEntityPosRot, MoveEntityRot,
        Ping, PlayerChat, PlayerInfoRemove, PlayerInfoUpdate, PlayerPosition, RecipeBookAdd, RecipeBookRemove,
        RecipeBookSettings, RemoveEntities, Respawn, RotateHead, SectionBlocksUpdate, ServerData,
        SetChunkCacheCenter, SetChunkCacheRadius, SetEntityData, SetEntityMotion, SetEquipment, SetExperience,
        SetHealth, SetHeldSlot, SetSimulationDistance, SetTime, Sound, SoundEntity, StartConfiguration,
        SystemChat, TabList, TeleportEntity, UpdateRecipes, UpdateTags, ClearDialog, CustomReportDetails,
        PostEffects, ResourcePackPop, ResourcePackPush, ServerLinks, StoreCookie, Transfer,
    }
}

// ---------------------------------------------------------------- shared types

/// A dimension and a position in it (`Global Position`).
#[derive(Clone, Debug, PartialEq)]
pub struct GlobalPos {
    /// Dimension name.
    pub dimension: Identifier,
    /// Block position.
    pub pos: BlockPos,
}
impl Wire for GlobalPos {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self { dimension: r.identifier()?, pos: r.block_pos()? })
    }
    fn write(&self, w: &mut Writer) {
        w.identifier(&self.dimension);
        w.block_pos(&self.pos);
    }
}

/// The spawn fields shared by `login` and `respawn`.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnInfo {
    /// Index into the synced `minecraft:dimension_type` registry.
    pub dimension_type: i32,
    /// Dimension name, e.g. `minecraft:overworld`.
    pub dimension: Identifier,
    /// First 8 bytes of SHA-256 of the seed.
    pub hashed_seed: i64,
    /// Game mode id (0 survival .. 3 spectator).
    pub game_mode: i32,
    /// Previous game mode encoded as id + 1, 0 for none.
    pub previous_game_mode: i32,
    /// Debug world.
    pub is_debug: bool,
    /// Superflat world.
    pub is_flat: bool,
    /// Where the player last died.
    pub death_location: Option<GlobalPos>,
    /// Portal cooldown in ticks.
    pub portal_cooldown: i32,
    /// Sea level.
    pub sea_level: i32,
}
impl Wire for SpawnInfo {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            dimension_type: r.var_int()?,
            dimension: r.identifier()?,
            hashed_seed: r.i64()?,
            game_mode: r.var_int()?,
            previous_game_mode: r.var_int()?,
            is_debug: r.bool()?,
            is_flat: r.bool()?,
            death_location: r.read()?,
            portal_cooldown: r.var_int()?,
            sea_level: r.var_int()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.var_int(self.dimension_type);
        w.identifier(&self.dimension);
        w.i64(self.hashed_seed);
        w.var_int(self.game_mode);
        w.var_int(self.previous_game_mode);
        w.bool(self.is_debug);
        w.bool(self.is_flat);
        w.write(&self.death_location);
        w.var_int(self.portal_cooldown);
        w.var_int(self.sea_level);
    }
}

/// Chat decoration (translation key, parameters, style) of an inline chat type.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatDecoration {
    /// Translation key.
    pub translation_key: String,
    /// Parameters: 0 sender, 1 target, 2 content.
    pub parameters: Vec<i32>,
    /// Style as raw NBT.
    pub style: Bytes,
}
impl Wire for ChatDecoration {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self { translation_key: r.string()?, parameters: r.read_as::<_, List<VarInt>>()?, style: r.nbt_raw()? })
    }
    fn write(&self, w: &mut Writer) {
        w.string(&self.translation_key);
        w.write_as::<_, List<VarInt>>(&self.parameters);
        w.bytes(&self.style);
    }
}

/// Inline `Chat Type`.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatType {
    /// Chat decoration.
    pub chat: ChatDecoration,
    /// Narration decoration.
    pub narration: ChatDecoration,
}
impl Wire for ChatType {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self { chat: r.read()?, narration: r.read()? })
    }
    fn write(&self, w: &mut Writer) {
        w.write(&self.chat);
        w.write(&self.narration);
    }
}

/// Chunk position sent as one long (`ChunkPos::to_packed`: z high, x low).
pub struct PackedChunkPos;
impl Codec<ChunkPos> for PackedChunkPos {
    fn read(r: &mut Reader<'_>) -> DecodeResult<ChunkPos> { Ok(ChunkPos::from_packed(r.i64()?)) }
    fn write(v: &ChunkPos, w: &mut Writer) { w.i64(v.to_packed()) }
}

/// Relative-teleport flag bits (`Teleport Flags`).
pub mod teleport_flags {
    #![allow(missing_docs)]
    pub const X: i32 = 0x01;
    pub const Y: i32 = 0x02;
    pub const Z: i32 = 0x04;
    pub const YAW: i32 = 0x08;
    pub const PITCH: i32 = 0x10;
    pub const VEL_X: i32 = 0x20;
    pub const VEL_Y: i32 = 0x40;
    pub const VEL_Z: i32 = 0x80;
    pub const ROTATE_VELOCITY: i32 = 0x100;
}

// ---------------------------------------------------------------- packets

packet! {
    /// Groups the packets up to the next delimiter into one tick.
    BundleDelimiter = "minecraft:bundle_delimiter" {}
}

packet! {
    /// Spawns a non-player-controlled entity (and other players).
    AddEntity = "minecraft:add_entity" {
        /// Network id.
        entity_id: i32 as VarInt,
        /// UUID.
        uuid: Uuid,
        /// Index into `minecraft:entity_type`.
        kind: i32 as VarInt,
        /// Position.
        pos: DVec3,
        /// Velocity in blocks per tick.
        velocity: DVec3 as LpVec3,
        /// Pitch in degrees.
        pitch: f32 as Angle,
        /// Yaw in degrees.
        yaw: f32 as Angle,
        /// Head yaw in degrees.
        head_yaw: f32 as Angle,
        /// Type-specific data (object data).
        data: i32 as VarInt,
    }
}
impl AddEntity {
    /// As the shared `EntitySpawn`.
    pub fn spawn(&self) -> EntitySpawn {
        EntitySpawn {
            id: EntityId(self.entity_id),
            uuid: self.uuid,
            kind: RegistryIndex(self.kind as u32),
            pos: self.pos,
            yaw: self.yaw,
            pitch: self.pitch,
            head_yaw: self.head_yaw,
            data: self.data,
            velocity: self.velocity,
        }
    }
}

packet! {
    /// Sets a block entity's data.
    BlockEntityData = "minecraft:block_entity_data" {
        /// Position.
        pos: BlockPos,
        /// Index into `minecraft:block_entity_type`.
        kind: i32 as VarInt,
        /// Network NBT, raw.
        nbt: Bytes as RawNbt,
    }
}

packet! {
    /// One block changed.
    BlockUpdate = "minecraft:block_update" {
        /// Position.
        pos: BlockPos,
        /// New block state.
        state: i32 as VarInt,
    }
}
impl BlockUpdate {
    /// As the shared `BlockChange`.
    pub fn change(&self) -> BlockChange {
        BlockChange { pos: self.pos, state: BlockStateId(self.state as u32) }
    }
}

packet! {
    /// Difficulty.
    ChangeDifficulty = "minecraft:change_difficulty" {
        /// 0 peaceful .. 3 hard.
        difficulty: u8,
        /// Locked.
        locked: bool,
    }
}

packet! {
    /// End of a chunk batch; the client answers `chunk_batch_received`.
    ChunkBatchFinished = "minecraft:chunk_batch_finished" {
        /// Chunks in the batch.
        batch_size: i32 as VarInt,
    }
}

packet! {
    /// Start of a chunk batch.
    ChunkBatchStart = "minecraft:chunk_batch_start" {}
}

packet! {
    /// The command graph (body kept raw for now).
    Commands = "minecraft:commands" {
        /// Raw body.
        body: Bytes as Rest,
    }
}

packet! {
    /// Replaces a container's contents.
    ContainerSetContent = "minecraft:container_set_content" {
        /// Window id (0 = player inventory).
        window_id: i32 as VarInt,
        /// State id.
        state_id: i32 as VarInt,
        /// Slots.
        slots: Vec<ItemStackData>,
        /// Item on the cursor.
        carried: ItemStackData,
    }
}

packet! {
    /// Sets one container slot.
    ContainerSetSlot = "minecraft:container_set_slot" {
        /// Window id.
        window_id: i32 as VarInt,
        /// State id.
        state_id: i32 as VarInt,
        /// Slot index.
        slot: i16,
        /// Item.
        item: ItemStackData,
    }
}

packet! {
    /// A chat message without signature data (`/say`, console).
    DisguisedChat = "minecraft:disguised_chat" {
        /// Message.
        message: Component,
        /// Chat type (registry id or inline).
        chat_type: IdOr<ChatType>,
        /// Sender name.
        sender_name: Component,
        /// Target name.
        target_name: Option<Component>,
    }
}

/// The position part of `entity_position_sync`.
#[derive(Clone, Debug, PartialEq)]
pub enum PositionPath {
    /// One absolute end position.
    Linear(DVec3),
    /// Absolute positions with tick offsets.
    Stepped(Vec<(DVec3, i32)>),
}

/// Synchronises an entity's absolute position and rotation (the packet
/// Mojang renamed from `teleport_entity` in 1.21.2).
#[derive(Clone, Debug, PartialEq)]
pub struct EntityPositionSync {
    /// Entity.
    pub entity_id: i32,
    /// Position path.
    pub path: PositionPath,
    /// Yaw in degrees.
    pub yaw: f32,
    /// Pitch in degrees.
    pub pitch: f32,
    /// On ground.
    pub on_ground: bool,
}
impl EntityPositionSync {
    /// The final position of the path.
    pub fn end_pos(&self) -> Option<DVec3> {
        match &self.path {
            PositionPath::Linear(p) => Some(*p),
            PositionPath::Stepped(s) => s.last().map(|(p, _)| *p),
        }
    }
}
impl Packet for EntityPositionSync {
    const NAME: &'static str = "minecraft:entity_position_sync";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let entity_id = r.var_int()?;
        let path = match r.var_int()? {
            0 => PositionPath::Linear(r.read()?),
            1 => PositionPath::Stepped(r.list(4096, |r| Ok((r.read::<DVec3>()?, r.var_int()?)))?),
            t => return Err(DecodeError::Invalid(format!("position path type {t}"))),
        };
        Ok(Self { entity_id, path, yaw: r.f32()?, pitch: r.f32()?, on_ground: r.bool()? })
    }
    fn encode(&self, w: &mut Writer) {
        w.var_int(self.entity_id);
        match &self.path {
            PositionPath::Linear(p) => {
                w.var_int(0);
                w.write(p);
            }
            PositionPath::Stepped(s) => {
                w.var_int(1);
                w.list(s, |w, (p, t)| {
                    w.write(p);
                    w.var_int(*t);
                });
            }
        }
        w.f32(self.yaw);
        w.f32(self.pitch);
        w.bool(self.on_ground);
    }
}

packet! {
    /// Unloads a chunk column.
    ForgetLevelChunk = "minecraft:forget_level_chunk" {
        /// Chunk.
        pos: ChunkPos as PackedChunkPos,
    }
}

packet! {
    /// Game state change (weather, game mode, ...).
    GameEvent = "minecraft:game_event" {
        /// Event id (see the wiki table; 13 = start waiting for chunks).
        event: u8,
        /// Event value.
        value: f32,
    }
}

/// A chunk column with light. `chunk.sections` stays opaque here:
/// `haste_world::wire` decodes it.
#[derive(Clone, Debug)]
pub struct LevelChunkWithLight {
    /// The chunk.
    pub chunk: ChunkData,
}
impl PartialEq for LevelChunkWithLight {
    fn eq(&self, o: &Self) -> bool {
        let (a, b) = (&self.chunk, &o.chunk);
        a.pos == b.pos
            && a.heightmaps == b.heightmaps
            && a.sections == b.sections
            && a.block_entities.len() == b.block_entities.len()
            && a.block_entities.iter().zip(&b.block_entities).all(|(x, y)| x.pos == y.pos && x.kind == y.kind && x.nbt == y.nbt)
            && light_eq(&a.light, &b.light)
    }
}
fn light_eq(a: &LightData, b: &LightData) -> bool {
    a.sky_mask == b.sky_mask
        && a.block_mask == b.block_mask
        && a.empty_sky_mask == b.empty_sky_mask
        && a.empty_block_mask == b.empty_block_mask
        && a.sky_arrays == b.sky_arrays
        && a.block_arrays == b.block_arrays
}
impl Packet for LevelChunkWithLight {
    const NAME: &'static str = "minecraft:level_chunk_with_light";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let x = r.i32()?;
        let z = r.i32()?;
        let heightmaps = r.list(16, |r| Ok((r.var_int()?, r.list(1024, |r| r.i64())?)))?;
        let sections = r.byte_array(crate::transport::MAX_PACKET_DATA_SIZE)?;
        let block_entities = r.list(crate::wire::MAX_LIST, |r| {
            let xz = r.u8()?;
            let y = r.i16()?;
            let kind = r.var_int()?;
            let nbt = r.nbt_raw()?;
            Ok(ChunkBlockEntity {
                pos: BlockPos::new((x << 4) + (xz >> 4) as i32, y as i32, (z << 4) + (xz & 15) as i32),
                kind: kind as u32,
                nbt,
            })
        })?;
        let light = r.read()?;
        Ok(Self { chunk: ChunkData { pos: ChunkPos::new(x, z), heightmaps, sections, block_entities, light } })
    }
    fn encode(&self, w: &mut Writer) {
        let c = &self.chunk;
        w.i32(c.pos.x);
        w.i32(c.pos.z);
        w.list(&c.heightmaps, |w, (t, longs)| {
            w.var_int(*t);
            w.list(longs, |w, l| w.i64(*l));
        });
        w.byte_array(&c.sections);
        w.list(&c.block_entities, |w, be| {
            w.u8((((be.pos.x & 15) << 4) | (be.pos.z & 15)) as u8);
            w.i16(be.pos.y as i16);
            w.var_int(be.kind as i32);
            w.bytes(&be.nbt);
        });
        w.write(&c.light);
    }
}

packet! {
    /// A sound/particle world event (door sounds, block break particles, ...).
    LevelEvent = "minecraft:level_event" {
        /// Event id.
        event: i32,
        /// Position.
        pos: BlockPos,
        /// Event data.
        data: i32,
        /// Play at full volume regardless of distance.
        global: bool,
    }
}

/// Light update for one chunk column.
#[derive(Clone, Debug)]
pub struct LightUpdate {
    /// Chunk.
    pub pos: ChunkPos,
    /// Light arrays.
    pub light: LightData,
}
impl PartialEq for LightUpdate {
    fn eq(&self, o: &Self) -> bool { self.pos == o.pos && light_eq(&self.light, &o.light) }
}
impl Packet for LightUpdate {
    const NAME: &'static str = "minecraft:light_update";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self { pos: ChunkPos::new(r.var_int()?, r.var_int()?), light: r.read()? })
    }
    fn encode(&self, w: &mut Writer) {
        w.var_int(self.pos.x);
        w.var_int(self.pos.z);
        w.write(&self.light);
    }
}

packet! {
    /// Joins the game.
    Login = "minecraft:login" {
        /// The player's entity id.
        entity_id: i32,
        /// Hardcore.
        hardcore: bool,
        /// All dimension names on the server.
        dimension_names: Vec<Identifier>,
        /// Ignored by clients.
        max_players: i32 as VarInt,
        /// Server view distance.
        view_distance: i32 as VarInt,
        /// Server simulation distance.
        simulation_distance: i32 as VarInt,
        /// Reduced debug info.
        reduced_debug_info: bool,
        /// Show the respawn screen.
        enable_respawn_screen: bool,
        /// Limited crafting.
        do_limited_crafting: bool,
        /// Dimension and game mode.
        spawn: SpawnInfo,
        /// Online mode (26.x field per the wiki).
        online_mode: bool,
        /// Secure chat enforced.
        enforces_secure_chat: bool,
    }
}

/// Entity movement deltas: one linear delta or stepped deltas (26.3).
/// Units are 1/4096 block.
#[derive(Clone, Debug, PartialEq)]
pub enum MoveDeltas {
    /// One delta.
    Linear([i16; 3]),
    /// Deltas with tick offsets.
    Stepped(Vec<(i32, [i16; 3])>),
}
impl MoveDeltas {
    /// Sum of all deltas in blocks. Approximate for `Stepped`: the wiki does
    /// not say whether step deltas are relative to the previous step (assumed).
    pub fn total(&self) -> DVec3 {
        let to = |d: &[i16; 3]| DVec3::new(d[0] as f64, d[1] as f64, d[2] as f64) / 4096.0;
        match self {
            MoveDeltas::Linear(d) => to(d),
            MoveDeltas::Stepped(s) => s.iter().map(|(_, d)| to(d)).sum(),
        }
    }
    fn read(r: &mut Reader<'_>) -> DecodeResult<(Self, bool)> {
        let props = r.var_int()?;
        let on_ground = props & 1 != 0;
        let steps = (props as u32 >> 1) as usize;
        let d3 = |r: &mut Reader<'_>| -> DecodeResult<[i16; 3]> { Ok([r.i16()?, r.i16()?, r.i16()?]) };
        let m = if steps == 0 {
            MoveDeltas::Linear(d3(r)?)
        } else {
            if steps > 4096 {
                return Err(DecodeError::TooLarge(format!("{steps} movement steps")));
            }
            let mut v = Vec::with_capacity(steps);
            for _ in 0..steps {
                v.push((r.var_int()?, d3(r)?));
            }
            MoveDeltas::Stepped(v)
        };
        Ok((m, on_ground))
    }
    fn write(&self, on_ground: bool, w: &mut Writer) {
        let d3 = |w: &mut Writer, d: &[i16; 3]| d.iter().for_each(|v| w.i16(*v));
        match self {
            MoveDeltas::Linear(d) => {
                w.var_int(on_ground as i32);
                d3(w, d);
            }
            MoveDeltas::Stepped(s) => {
                w.var_int(((s.len() as i32) << 1) | on_ground as i32);
                for (t, d) in s {
                    w.var_int(*t);
                    d3(w, d);
                }
            }
        }
    }
}

/// Small entity move.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveEntityPos {
    /// Entity.
    pub entity_id: i32,
    /// Deltas.
    pub deltas: MoveDeltas,
    /// On ground.
    pub on_ground: bool,
}
impl Packet for MoveEntityPos {
    const NAME: &'static str = "minecraft:move_entity_pos";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let entity_id = r.var_int()?;
        let (deltas, on_ground) = MoveDeltas::read(r)?;
        Ok(Self { entity_id, deltas, on_ground })
    }
    fn encode(&self, w: &mut Writer) {
        w.var_int(self.entity_id);
        self.deltas.write(self.on_ground, w);
    }
}

/// Small entity move with rotation.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveEntityPosRot {
    /// Entity.
    pub entity_id: i32,
    /// Deltas.
    pub deltas: MoveDeltas,
    /// On ground.
    pub on_ground: bool,
    /// Yaw in degrees.
    pub yaw: f32,
    /// Pitch in degrees.
    pub pitch: f32,
}
impl Packet for MoveEntityPosRot {
    const NAME: &'static str = "minecraft:move_entity_pos_rot";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let entity_id = r.var_int()?;
        let (deltas, on_ground) = MoveDeltas::read(r)?;
        Ok(Self { entity_id, deltas, on_ground, yaw: r.angle()?, pitch: r.angle()? })
    }
    fn encode(&self, w: &mut Writer) {
        w.var_int(self.entity_id);
        self.deltas.write(self.on_ground, w);
        w.angle(self.yaw);
        w.angle(self.pitch);
    }
}

packet! {
    /// Entity rotation.
    MoveEntityRot = "minecraft:move_entity_rot" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// On ground (before the angles in 26.3 per the wiki).
        on_ground: bool,
        /// Yaw in degrees.
        yaw: f32 as Angle,
        /// Pitch in degrees.
        pitch: f32 as Angle,
    }
}

/// A previous-message reference in `player_chat`: an id into the client's
/// cache, or a full signature when the id is unknown.
#[derive(Clone, Debug, PartialEq)]
pub enum PreviousMessage {
    /// Cached message id.
    Id(i32),
    /// Full 256-byte signature.
    Signature(Bytes),
}

/// A signed player chat message (decoded, signature not verified).
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerChat {
    /// Global message index.
    pub global_index: i32,
    /// Sender UUID.
    pub sender: Uuid,
    /// Per-sender index.
    pub index: i32,
    /// 256-byte signature, if signed.
    pub signature: Option<Bytes>,
    /// Message text.
    pub message: String,
    /// Timestamp in ms.
    pub timestamp: i64,
    /// Salt.
    pub salt: i64,
    /// Last-seen messages.
    pub previous: Vec<PreviousMessage>,
    /// Unsigned replacement content.
    pub unsigned_content: Option<Component>,
    /// 0 pass through, 1 fully filtered, 2 partially filtered.
    pub filter_type: i32,
    /// Filter mask when partially filtered.
    pub filter_bits: Option<Vec<u64>>,
    /// Chat type.
    pub chat_type: IdOr<ChatType>,
    /// Sender name.
    pub sender_name: Component,
    /// Target name.
    pub target_name: Option<Component>,
}
impl Packet for PlayerChat {
    const NAME: &'static str = "minecraft:player_chat";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let global_index = r.var_int()?;
        let sender = r.uuid()?;
        let index = r.var_int()?;
        let signature = r.option(|r| Ok(Bytes::copy_from_slice(r.take(256)?)))?;
        let message = r.string_max(256)?;
        let timestamp = r.i64()?;
        let salt = r.i64()?;
        let previous = r.list(20, |r| {
            let id = r.var_int()?;
            if id == 0 { Ok(PreviousMessage::Signature(Bytes::copy_from_slice(r.take(256)?))) } else { Ok(PreviousMessage::Id(id - 1)) }
        })?;
        let unsigned_content = r.option(|r| r.component())?;
        let filter_type = r.var_int()?;
        let filter_bits = if filter_type == 2 { Some(r.bit_set()?) } else { None };
        Ok(Self {
            global_index,
            sender,
            index,
            signature,
            message,
            timestamp,
            salt,
            previous,
            unsigned_content,
            filter_type,
            filter_bits,
            chat_type: r.read()?,
            sender_name: r.component()?,
            target_name: r.read()?,
        })
    }
    fn encode(&self, w: &mut Writer) {
        w.var_int(self.global_index);
        w.uuid(&self.sender);
        w.var_int(self.index);
        w.option(&self.signature, |w, s| w.bytes(s));
        w.string(&self.message);
        w.i64(self.timestamp);
        w.i64(self.salt);
        w.list(&self.previous, |w, p| match p {
            PreviousMessage::Id(i) => w.var_int(i + 1),
            PreviousMessage::Signature(s) => {
                w.var_int(0);
                w.bytes(s);
            }
        });
        w.write(&self.unsigned_content);
        w.var_int(self.filter_type);
        if let Some(b) = &self.filter_bits {
            w.bit_set(b);
        }
        w.write(&self.chat_type);
        w.component(&self.sender_name);
        w.write(&self.target_name);
    }
}

packet! {
    /// Removes players from the tab list.
    PlayerInfoRemove = "minecraft:player_info_remove" {
        /// Player UUIDs.
        uuids: Vec<Uuid>,
    }
}

/// Chat session data in `player_info_update` (action 0x02).
#[derive(Clone, Debug, PartialEq)]
pub struct ChatSession {
    /// Session id.
    pub session_id: Uuid,
    /// Key expiry, ms since epoch.
    pub expires_at: i64,
    /// Public key, DER.
    pub public_key: Bytes,
    /// Mojang's signature of the key.
    pub key_signature: Bytes,
}
impl Wire for ChatSession {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self { session_id: r.uuid()?, expires_at: r.i64()?, public_key: r.byte_array(512)?, key_signature: r.byte_array(4096)? })
    }
    fn write(&self, w: &mut Writer) {
        w.uuid(&self.session_id);
        w.i64(self.expires_at);
        w.byte_array(&self.public_key);
        w.byte_array(&self.key_signature);
    }
}

/// `player_info_update` action bits.
pub mod player_info_actions {
    #![allow(missing_docs)]
    pub const ADD_PLAYER: u8 = 0x01;
    pub const INITIALIZE_CHAT: u8 = 0x02;
    pub const UPDATE_GAME_MODE: u8 = 0x04;
    pub const UPDATE_LISTED: u8 = 0x08;
    pub const UPDATE_LATENCY: u8 = 0x10;
    pub const UPDATE_DISPLAY_NAME: u8 = 0x20;
    pub const UPDATE_LIST_PRIORITY: u8 = 0x40;
    pub const UPDATE_HAT: u8 = 0x80;
}

/// One player's entry in `player_info_update`; fields are `Some` exactly
/// when the packet's action mask has the corresponding bit.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PlayerInfoEntry {
    /// Player UUID.
    pub uuid: Uuid,
    /// Add player: name and profile properties.
    pub add: Option<(String, Vec<ProfileProperty>)>,
    /// Initialize chat.
    pub chat_session: Option<Option<ChatSession>>,
    /// Game mode id.
    pub game_mode: Option<i32>,
    /// Listed on the tab list.
    pub listed: Option<bool>,
    /// Latency in ms.
    pub latency: Option<i32>,
    /// Display name.
    pub display_name: Option<Option<Component>>,
    /// Tab list priority.
    pub list_priority: Option<i32>,
    /// Hat layer visible.
    pub show_hat: Option<bool>,
}

/// Adds or updates tab list entries.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerInfoUpdate {
    /// Action bit mask ([`player_info_actions`]).
    pub actions: u8,
    /// Entries.
    pub entries: Vec<PlayerInfoEntry>,
}
impl PlayerInfoUpdate {
    /// As the shared `PlayerInfo` (entries without `ADD_PLAYER` get an empty name).
    pub fn infos(&self) -> Vec<haste_core::entity::PlayerInfo> {
        self.entries
            .iter()
            .map(|e| haste_core::entity::PlayerInfo {
                uuid: e.uuid,
                name: e.add.as_ref().map(|a| a.0.clone()).unwrap_or_default(),
                display_name: e.display_name.clone().flatten(),
                latency: e.latency.unwrap_or(0),
                game_mode: e.game_mode.and_then(haste_core::game::GameMode::from_id),
                listed: e.listed.unwrap_or(false),
                skin_texture_url: None,
            })
            .collect()
    }
}
impl Packet for PlayerInfoUpdate {
    const NAME: &'static str = "minecraft:player_info_update";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        use player_info_actions::*;
        let actions = r.u8()?;
        let entries = r.list(4096, |r| {
            let mut e = PlayerInfoEntry { uuid: r.uuid()?, ..Default::default() };
            if actions & ADD_PLAYER != 0 {
                e.add = Some((r.string_max(16)?, r.list(16, ProfileProperty::read)?));
            }
            if actions & INITIALIZE_CHAT != 0 {
                e.chat_session = Some(r.read()?);
            }
            if actions & UPDATE_GAME_MODE != 0 {
                e.game_mode = Some(r.var_int()?);
            }
            if actions & UPDATE_LISTED != 0 {
                e.listed = Some(r.bool()?);
            }
            if actions & UPDATE_LATENCY != 0 {
                e.latency = Some(r.var_int()?);
            }
            if actions & UPDATE_DISPLAY_NAME != 0 {
                e.display_name = Some(r.read()?);
            }
            if actions & UPDATE_LIST_PRIORITY != 0 {
                e.list_priority = Some(r.var_int()?);
            }
            if actions & UPDATE_HAT != 0 {
                e.show_hat = Some(r.bool()?);
            }
            Ok(e)
        })?;
        Ok(Self { actions, entries })
    }
    fn encode(&self, w: &mut Writer) {
        w.u8(self.actions);
        w.list(&self.entries, |w, e| {
            w.uuid(&e.uuid);
            if let Some((name, props)) = &e.add {
                w.string(name);
                w.write(props);
            }
            if let Some(c) = &e.chat_session {
                w.write(c);
            }
            if let Some(g) = e.game_mode {
                w.var_int(g);
            }
            if let Some(l) = e.listed {
                w.bool(l);
            }
            if let Some(l) = e.latency {
                w.var_int(l);
            }
            if let Some(d) = &e.display_name {
                w.write(d);
            }
            if let Some(p) = e.list_priority {
                w.var_int(p);
            }
            if let Some(h) = e.show_hat {
                w.bool(h);
            }
        });
    }
}

packet! {
    /// Teleports the local player; answered with `accept_teleportation`.
    PlayerPosition = "minecraft:player_position" {
        /// Id to confirm.
        teleport_id: i32 as VarInt,
        /// Position (absolute or relative per `flags`).
        pos: DVec3,
        /// Velocity.
        velocity: DVec3,
        /// Yaw.
        yaw: f32,
        /// Pitch.
        pitch: f32,
        /// [`teleport_flags`] bits.
        flags: i32,
    }
}

packet! {
    /// Recipe book additions (body kept raw).
    RecipeBookAdd = "minecraft:recipe_book_add" {
        /// Raw body.
        body: Bytes as Rest,
    }
}

packet! {
    /// Recipe book removals (body kept raw).
    RecipeBookRemove = "minecraft:recipe_book_remove" {
        /// Raw body.
        body: Bytes as Rest,
    }
}

packet! {
    /// Recipe book settings (body kept raw).
    RecipeBookSettings = "minecraft:recipe_book_settings" {
        /// Raw body.
        body: Bytes as Rest,
    }
}

packet! {
    /// Removes entities.
    RemoveEntities = "minecraft:remove_entities" {
        /// Entity ids.
        entity_ids: Vec<i32> as List<VarInt>,
    }
}

packet! {
    /// Respawn or dimension change.
    Respawn = "minecraft:respawn" {
        /// Dimension and game mode.
        spawn: SpawnInfo,
        /// 0x01 keep attributes, 0x02 keep metadata.
        data_kept: u8,
    }
}

packet! {
    /// Head yaw.
    RotateHead = "minecraft:rotate_head" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// Head yaw in degrees.
        head_yaw: f32 as Angle,
    }
}

packet! {
    /// Several block changes in one section.
    SectionBlocksUpdate = "minecraft:section_blocks_update" {
        /// Packed section position.
        section: i64,
        /// `state << 12 | x << 8 | z << 4 | y` per block.
        blocks: Vec<i64> as List<VarLong>,
    }
}
impl SectionBlocksUpdate {
    /// The section.
    pub fn section_pos(&self) -> SectionPos {
        let v = self.section;
        SectionPos::new((v >> 42) as i32, ((v << 44) >> 44) as i32, ((v << 22) >> 42) as i32)
    }
    /// Packs a section position.
    pub fn pack_section(p: SectionPos) -> i64 {
        ((p.x as i64 & 0x3F_FFFF) << 42) | (p.y as i64 & 0xF_FFFF) | ((p.z as i64 & 0x3F_FFFF) << 20)
    }
    /// The changes as absolute positions.
    pub fn changes(&self) -> impl Iterator<Item = BlockChange> + '_ {
        let base = self.section_pos().block_min();
        self.blocks.iter().map(move |b| BlockChange {
            pos: base.add(((b >> 8) & 15) as i32, (b & 15) as i32, ((b >> 4) & 15) as i32),
            state: BlockStateId((b >> 12) as u32),
        })
    }
}

packet! {
    /// MOTD and icon.
    ServerData = "minecraft:server_data" {
        /// MOTD.
        motd: Component,
        /// PNG icon.
        icon: Option<Bytes> as Opt<ByteArray>,
    }
}

packet! {
    /// Centre of the chunk loading area.
    SetChunkCacheCenter = "minecraft:set_chunk_cache_center" {
        /// Chunk x.
        x: i32 as VarInt,
        /// Chunk z.
        z: i32 as VarInt,
    }
}

packet! {
    /// Server view distance.
    SetChunkCacheRadius = "minecraft:set_chunk_cache_radius" {
        /// Radius in chunks.
        radius: i32 as VarInt,
    }
}

packet! {
    /// Entity metadata update.
    SetEntityData = "minecraft:set_entity_data" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// Changed entries.
        metadata: Vec<MetadataEntry> as Metadata,
    }
}

packet! {
    /// Entity velocity.
    SetEntityMotion = "minecraft:set_entity_motion" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// Velocity in blocks per tick.
        velocity: DVec3 as LpVec3,
    }
}

/// Entity equipment: `(slot, item)` pairs; slot 0 main hand, 1 off hand,
/// 2 boots, 3 leggings, 4 chestplate, 5 helmet, 6 body, 7 saddle.
#[derive(Clone, Debug, PartialEq)]
pub struct SetEquipment {
    /// Entity.
    pub entity_id: i32,
    /// Slots and items.
    pub equipment: Vec<(u8, ItemStackData)>,
}
impl Packet for SetEquipment {
    const NAME: &'static str = "minecraft:set_equipment";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let entity_id = r.var_int()?;
        let mut equipment = Vec::new();
        loop {
            let s = r.u8()?;
            equipment.push((s & 0x7f, ItemStackData::read(r)?));
            if s & 0x80 == 0 {
                break;
            }
            if equipment.len() > 64 {
                return Err(DecodeError::TooLarge("equipment list".into()));
            }
        }
        Ok(Self { entity_id, equipment })
    }
    fn encode(&self, w: &mut Writer) {
        w.var_int(self.entity_id);
        let n = self.equipment.len();
        for (i, (s, item)) in self.equipment.iter().enumerate() {
            w.u8(if i + 1 < n { s | 0x80 } else { *s });
            w.write(item);
        }
    }
}

packet! {
    /// Experience.
    SetExperience = "minecraft:set_experience" {
        /// Bar progress 0..1.
        bar: f32,
        /// Level.
        level: i32 as VarInt,
        /// Total experience.
        total: i32 as VarInt,
    }
}

packet! {
    /// Health and food.
    SetHealth = "minecraft:set_health" {
        /// Health (0 = dead, 20 = full).
        health: f32,
        /// Food 0..20.
        food: i32 as VarInt,
        /// Saturation.
        saturation: f32,
    }
}

packet! {
    /// Selected hotbar slot.
    SetHeldSlot = "minecraft:set_held_slot" {
        /// Slot 0..8.
        slot: i32 as VarInt,
    }
}

packet! {
    /// Simulation distance.
    SetSimulationDistance = "minecraft:set_simulation_distance" {
        /// Distance in chunks.
        distance: i32 as VarInt,
    }
}

/// One clock in `set_time` (26.x world clocks).
#[derive(Clone, Debug, PartialEq)]
pub struct ClockState {
    /// Index into `minecraft:world_clock`.
    pub clock: i32,
    /// Ticks.
    pub time: i64,
    /// Fractional tick.
    pub fraction: f32,
    /// Clock ticks per client tick.
    pub rate: f32,
}
impl Wire for ClockState {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self { clock: r.var_int()?, time: r.var_long()?, fraction: r.f32()?, rate: r.f32()? })
    }
    fn write(&self, w: &mut Writer) {
        w.var_int(self.clock);
        w.var_long(self.time);
        w.f32(self.fraction);
        w.f32(self.rate);
    }
}

packet! {
    /// World age and clocks.
    SetTime = "minecraft:set_time" {
        /// World age in ticks.
        world_age: i64,
        /// Clocks.
        clocks: Vec<ClockState>,
    }
}

packet! {
    /// A sound at a position.
    Sound = "minecraft:sound" {
        /// Sound (registry id or inline).
        sound: IdOr<SoundEvent>,
        /// Category.
        category: i32 as VarInt,
        /// X * 8.
        x: i32,
        /// Y * 8.
        y: i32,
        /// Z * 8.
        z: i32,
        /// Volume.
        volume: f32,
        /// Pitch.
        pitch: f32,
        /// Variant seed.
        seed: i64,
    }
}

packet! {
    /// A sound from an entity.
    SoundEntity = "minecraft:sound_entity" {
        /// Sound.
        sound: IdOr<SoundEvent>,
        /// Category.
        category: i32 as VarInt,
        /// Entity.
        entity_id: i32 as VarInt,
        /// Volume.
        volume: f32,
        /// Pitch.
        pitch: f32,
        /// Variant seed.
        seed: i64,
    }
}

packet! {
    /// Back to configuration; the client acknowledges.
    StartConfiguration = "minecraft:start_configuration" {}
}

packet! {
    /// A system message.
    SystemChat = "minecraft:system_chat" {
        /// Content.
        content: Component,
        /// Action bar instead of chat.
        overlay: bool,
    }
}

packet! {
    /// Tab list header and footer.
    TabList = "minecraft:tab_list" {
        /// Header.
        header: Component,
        /// Footer.
        footer: Component,
    }
}

packet! {
    /// Teleports an entity without resetting its delta base (vehicles).
    TeleportEntity = "minecraft:teleport_entity" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// Position.
        pos: DVec3,
        /// Velocity.
        velocity: DVec3,
        /// Yaw.
        yaw: f32,
        /// Pitch.
        pitch: f32,
        /// [`teleport_flags`] bits.
        flags: i32,
        /// On ground.
        on_ground: bool,
    }
}

packet! {
    /// Recipe property sets and stonecutter recipes (body kept raw).
    UpdateRecipes = "minecraft:update_recipes" {
        /// Raw body.
        body: Bytes as Rest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::test_util::round_trip;
    use haste_core::entity::MetadataValue;

    fn nbt() -> Bytes {
        let mut c = crate::nbt::NbtCompound::new();
        c.put_string("id", "x".into());
        let mut w = Writer::new();
        w.nbt_compound(Some(&c));
        w.into_inner().into()
    }
    fn item() -> ItemStackData {
        ItemStackData { count: 2, item: Some(RegistryIndex(1)), components_raw: Bytes::from_static(&[0, 0]) }
    }
    fn chat_type() -> ChatType {
        let d = ChatDecoration { translation_key: "chat.type.text".into(), parameters: vec![0, 2], style: Bytes::from_static(&[0]) };
        ChatType { chat: d.clone(), narration: d }
    }
    fn spawn_info() -> SpawnInfo {
        SpawnInfo {
            dimension_type: 0,
            dimension: Identifier::minecraft("overworld"),
            hashed_seed: -42,
            game_mode: 1,
            previous_game_mode: 0,
            is_debug: false,
            is_flat: true,
            death_location: Some(GlobalPos { dimension: Identifier::minecraft("the_nether"), pos: BlockPos::new(1, 2, 3) }),
            portal_cooldown: 0,
            sea_level: 63,
        }
    }

    #[test]
    fn round_trips() {
        round_trip(&BundleDelimiter {});
        round_trip(&AddEntity {
            entity_id: 7,
            uuid: Uuid::from_u128(3),
            kind: 140,
            pos: DVec3::new(1.5, 64.0, -3.25),
            velocity: DVec3::ZERO,
            pitch: 45.0,
            yaw: 90.0,
            head_yaw: 180.0,
            data: 0,
        });
        round_trip(&BlockEntityData { pos: BlockPos::new(1, 2, 3), kind: 7, nbt: nbt() });
        round_trip(&BlockUpdate { pos: BlockPos::new(-1, -64, 5), state: 1234 });
        round_trip(&ChangeDifficulty { difficulty: 2, locked: false });
        round_trip(&ChunkBatchFinished { batch_size: 25 });
        round_trip(&ChunkBatchStart {});
        round_trip(&Commands { body: Bytes::from_static(&[1, 2, 3]) });
        round_trip(&ContainerSetContent { window_id: 0, state_id: 1, slots: vec![ItemStackData::default(), item()], carried: ItemStackData::default() });
        round_trip(&ContainerSetSlot { window_id: 0, state_id: 2, slot: 36, item: item() });
        round_trip(&DisguisedChat { message: Component::text("hi"), chat_type: IdOr::Inline(chat_type()), sender_name: Component::text("Server"), target_name: None });
        round_trip(&EntityPositionSync { entity_id: 1, path: PositionPath::Linear(DVec3::new(1.0, 2.0, 3.0)), yaw: 10.0, pitch: -5.0, on_ground: true });
        round_trip(&EntityPositionSync { entity_id: 1, path: PositionPath::Stepped(vec![(DVec3::X, 1), (DVec3::Y, 2)]), yaw: 0.0, pitch: 0.0, on_ground: false });
        round_trip(&ForgetLevelChunk { pos: ChunkPos::new(-3, 7) });
        round_trip(&GameEvent { event: 13, value: 0.0 });
        round_trip(&LevelChunkWithLight {
            chunk: ChunkData {
                pos: ChunkPos::new(-2, 5),
                heightmaps: vec![(1, vec![1, 2, 3]), (4, vec![])],
                sections: Bytes::from_static(&[0, 1, 0, 0]),
                block_entities: vec![ChunkBlockEntity { pos: BlockPos::new(-32 + 3, -60, 80 + 15), kind: 9, nbt: nbt() }],
                light: LightData { sky_mask: vec![3], block_mask: vec![], empty_sky_mask: vec![4], empty_block_mask: vec![], sky_arrays: vec![Bytes::from(vec![0xff; 2048]), Bytes::from(vec![0; 2048])], block_arrays: vec![] },
            },
        });
        round_trip(&LevelEvent { event: 1000, pos: BlockPos::new(0, 0, 0), data: 0, global: false });
        round_trip(&LightUpdate { pos: ChunkPos::new(1, 2), light: LightData::default() });
        round_trip(&Login {
            entity_id: 42,
            hardcore: false,
            dimension_names: vec![Identifier::minecraft("overworld")],
            max_players: 20,
            view_distance: 10,
            simulation_distance: 10,
            reduced_debug_info: false,
            enable_respawn_screen: true,
            do_limited_crafting: false,
            spawn: spawn_info(),
            online_mode: false,
            enforces_secure_chat: false,
        });
        round_trip(&MoveEntityPos { entity_id: 3, deltas: MoveDeltas::Linear([4096, -1, 0]), on_ground: true });
        round_trip(&MoveEntityPos { entity_id: 3, deltas: MoveDeltas::Stepped(vec![(0, [1, 2, 3]), (1, [4, 5, 6])]), on_ground: false });
        round_trip(&MoveEntityPosRot { entity_id: 3, deltas: MoveDeltas::Linear([0, 0, 1]), on_ground: false, yaw: 90.0, pitch: 0.0 });
        round_trip(&MoveEntityRot { entity_id: 3, on_ground: true, yaw: 45.0, pitch: 270.0 });
        round_trip(&PlayerChat {
            global_index: 0,
            sender: Uuid::from_u128(1),
            index: 0,
            signature: Some(Bytes::from(vec![7; 256])),
            message: "hello".into(),
            timestamp: 1,
            salt: 2,
            previous: vec![PreviousMessage::Id(3), PreviousMessage::Signature(Bytes::from(vec![1; 256]))],
            unsigned_content: None,
            filter_type: 2,
            filter_bits: Some(vec![5]),
            chat_type: IdOr::Id(0),
            sender_name: Component::text("Steve"),
            target_name: Some(Component::text("Alex")),
        });
        round_trip(&PlayerInfoRemove { uuids: vec![Uuid::from_u128(9)] });
        round_trip(&PlayerInfoUpdate {
            actions: 0xff,
            entries: vec![PlayerInfoEntry {
                uuid: Uuid::from_u128(1),
                add: Some(("Steve".into(), vec![])),
                chat_session: Some(None),
                game_mode: Some(1),
                listed: Some(true),
                latency: Some(12),
                display_name: Some(Some(Component::text("S"))),
                list_priority: Some(0),
                show_hat: Some(true),
            }],
        });
        round_trip(&PlayerInfoUpdate { actions: player_info_actions::UPDATE_LATENCY, entries: vec![PlayerInfoEntry { uuid: Uuid::from_u128(2), latency: Some(5), ..Default::default() }] });
        round_trip(&PlayerPosition { teleport_id: 1, pos: DVec3::new(0.5, 70.0, 0.5), velocity: DVec3::ZERO, yaw: 0.0, pitch: 0.0, flags: teleport_flags::VEL_X | teleport_flags::YAW });
        round_trip(&RecipeBookAdd { body: Bytes::from_static(&[0, 1]) });
        round_trip(&RecipeBookRemove { body: Bytes::new() });
        round_trip(&RecipeBookSettings { body: Bytes::from_static(&[0; 8]) });
        round_trip(&RemoveEntities { entity_ids: vec![1, 2, 300] });
        round_trip(&Respawn { spawn: spawn_info(), data_kept: 3 });
        round_trip(&RotateHead { entity_id: 2, head_yaw: 33.75 });
        let p = SectionPos::new(-3, -4, 1_000_000);
        let u = SectionBlocksUpdate { section: SectionBlocksUpdate::pack_section(p), blocks: vec![(5 << 12) | (1 << 8) | (2 << 4) | 3] };
        assert_eq!(u.section_pos(), p);
        let c: Vec<_> = u.changes().collect();
        assert_eq!(c[0].pos, BlockPos::new(-48 + 1, -64 + 3, 16_000_000 + 2));
        assert_eq!(c[0].state, BlockStateId(5));
        round_trip(&u);
        round_trip(&ServerData { motd: Component::text("A Minecraft Server"), icon: None });
        round_trip(&SetChunkCacheCenter { x: -1, z: 2 });
        round_trip(&SetChunkCacheRadius { radius: 10 });
        round_trip(&SetEntityData { entity_id: 1, metadata: vec![MetadataEntry { index: 0, value: MetadataValue::Byte(0) }] });
        round_trip(&SetEntityMotion { entity_id: 1, velocity: DVec3::ZERO });
        round_trip(&SetEquipment { entity_id: 5, equipment: vec![(0, item()), (5, ItemStackData::default())] });
        round_trip(&SetExperience { bar: 0.5, level: 3, total: 40 });
        round_trip(&SetHealth { health: 20.0, food: 20, saturation: 5.0 });
        round_trip(&SetHeldSlot { slot: 4 });
        round_trip(&SetSimulationDistance { distance: 12 });
        round_trip(&SetTime { world_age: 1000, clocks: vec![ClockState { clock: 0, time: 6000, fraction: 0.0, rate: 1.0 }] });
        round_trip(&Sound { sound: IdOr::Id(10), category: 0, x: 8, y: 16, z: -8, volume: 1.0, pitch: 1.0, seed: 99 });
        round_trip(&SoundEntity { sound: IdOr::Inline(SoundEvent { name: Identifier::minecraft("x"), fixed_range: None }), category: 5, entity_id: 3, volume: 0.5, pitch: 2.0, seed: 0 });
        round_trip(&StartConfiguration {});
        round_trip(&SystemChat { content: Component::text("sys"), overlay: true });
        round_trip(&TabList { header: Component::text("h"), footer: Component::default() });
        round_trip(&TeleportEntity { entity_id: 9, pos: DVec3::ONE, velocity: DVec3::ZERO, yaw: 1.0, pitch: 2.0, flags: 0, on_ground: true });
        round_trip(&UpdateRecipes { body: Bytes::from_static(&[0, 0]) });
    }

    #[test]
    fn add_entity_converts_to_spawn() {
        let a = AddEntity { entity_id: 7, uuid: Uuid::nil(), kind: 3, pos: DVec3::ONE, velocity: DVec3::ZERO, pitch: 1.0, yaw: 2.0, head_yaw: 3.0, data: 4 };
        let s = a.spawn();
        assert_eq!(s.id, EntityId(7));
        assert_eq!((s.yaw, s.pitch, s.head_yaw), (2.0, 1.0, 3.0));
    }
}
