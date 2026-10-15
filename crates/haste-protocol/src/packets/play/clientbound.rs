//! Clientbound play packets.
//!
//! Layouts from the minecraft.wiki "Java Edition protocol/Packets" page for
//! 26.3; see NOTES.md for what was checked against the local server. Packets
//! whose body we do not interpret yet keep it raw (`body: Bytes`).

use bytes::Bytes;
use haste_core::chunk::{BlockChange, BlockEntityData as ChunkBlockEntity, ChunkData, LightData};
use haste_core::entity::{EntitySpawn, ItemStackData, MetadataEntry};
use haste_core::{
    BlockPos, BlockStateId, ChunkPos, Component, DVec3, EntityId, Identifier, RegistryIndex,
    SectionPos,
};
use uuid::Uuid;

use crate::metadata::Metadata;
pub use crate::packets::configuration::clientbound::{
    ClearDialog, CookieRequest, CustomPayload, CustomReportDetails, Disconnect, KeepAlive, Ping,
    PostEffects, ResourcePackPop, ResourcePackPush, ServerLinks, StoreCookie, Transfer, UpdateTags,
};
use crate::packets::{packet, packet_enum, Packet};
use crate::wire::particle::Particle;
use crate::wire::{
    Angle, ByteArray, Codec, DecodeError, DecodeResult, IdOr, List, LpVec3, Opt, ProfileProperty,
    RawNbt, Reader, Rest, SoundEvent, VarInt, VarLong, Wire, Writer,
};

packet_enum! {
    /// Clientbound play packets, in `packets.json` id order. Ids without a
    /// struct (none in 26.3) or failed decodes arrive as `Unknown`.
    ClientboundPlay {
        BundleDelimiter, AddEntity, Animate, AwardStats, BlockChangedAck, BlockDestruction, BlockEntityData,
        BlockEvent, BlockUpdate, BossEvent, ChangeDifficulty, ChunkBatchFinished, ChunkBatchStart, ChunksBiomes,
        ClearTitles, CommandSuggestions, Commands, ContainerClose, ContainerSetContent, ContainerSetData,
        ContainerSetSlot, CookieRequest, Cooldown, CustomChatCompletions, CustomPayload, DamageEvent, DebugBlockValue,
        DebugChunkValue, DebugEntityValue, DebugEvent, DebugSample, DeleteChat, Disconnect, DisguisedChat,
        EntityEvent, EntityPositionSync, Explode, AddTransientBlock, ForgetLevelChunk, GameEvent, GameRuleValues,
        GameTestHighlightPos, MountScreenOpen, HurtAnimation, InitializeBorder, KeepAlive, LevelChunkWithLight,
        LevelEvent, LevelParticles, LightUpdate, Login, LowDiskSpaceWarning, MapItemData, MerchantOffers,
        MoveEntityPos, MoveEntityPosRot, MoveMinecartAlongTrack, MoveEntityRot, MoveVehicle, OpenBook, OpenScreen,
        OpenSignEditor, Ping, PongResponse, PlaceGhostRecipe, PlayerAbilities, PlayerChat, PlayerCombatEnd,
        PlayerCombatEnter, PlayerCombatKill, PlayerInfoRemove, PlayerInfoUpdate, PlayerLookAt, PlayerPosition,
        PlayerRotation, RecipeBookAdd, RecipeBookRemove, RecipeBookSettings, RemoveEntities, RemoveMobEffect,
        ResetScore, ResourcePackPop, ResourcePackPush, PostEffects, Respawn, RotateHead, SectionBlocksUpdate,
        SelectAdvancementsTab, ServerData, SetActionBarText, SetBorderCenter, SetBorderLerpSize, SetBorderSize,
        SetBorderWarningDelay, SetBorderWarningDistance, SetCamera, SetChunkCacheCenter, SetChunkCacheRadius,
        SetCursorItem, SetDefaultSpawnPosition, SetDisplayObjective, SetEntityData, SetEntityLink, SetEntityMotion,
        SetEquipment, SetExperience, SetHealth, SetHeldSlot, SetObjective, SetPassengers, SetPlayerInventory,
        SetPlayerTeam, SetScore, SetSimulationDistance, SetSubtitleText, SetTime, SetTitleText, SetTitlesAnimation,
        SoundEntity, Sound, StartConfiguration, StopSound, StoreCookie, SwingAnimation, SystemChat, TabList, TagQuery,
        TakeItemEntity, TeleportEntity, TestInstanceBlockStatus, TickingState, TickingStep, Transfer,
        UpdateAdvancements, UpdateAttributes, UpdateMobEffect, UpdateRecipes, UpdateTags, ProjectilePower,
        CustomReportDetails, ServerLinks, Waypoint, ClearDialog, ShowDialog,
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
        Ok(Self {
            dimension: r.identifier()?,
            pos: r.block_pos()?,
        })
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
        Ok(Self {
            translation_key: r.string()?,
            parameters: r.read_as::<_, List<VarInt>>()?,
            style: r.nbt_raw()?,
        })
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
        Ok(Self {
            chat: r.read()?,
            narration: r.read()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.write(&self.chat);
        w.write(&self.narration);
    }
}

/// Chunk position sent as one long (`ChunkPos::to_packed`: z high, x low).
pub struct PackedChunkPos;
impl Codec<ChunkPos> for PackedChunkPos {
    fn read(r: &mut Reader<'_>) -> DecodeResult<ChunkPos> {
        Ok(ChunkPos::from_packed(r.i64()?))
    }
    fn write(v: &ChunkPos, w: &mut Writer) {
        w.i64(v.to_packed())
    }
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
        BlockChange {
            pos: self.pos,
            state: BlockStateId(self.state as u32),
        }
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
        Ok(Self {
            entity_id,
            path,
            yaw: r.f32()?,
            pitch: r.f32()?,
            on_ground: r.bool()?,
        })
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
            && a.block_entities
                .iter()
                .zip(&b.block_entities)
                .all(|(x, y)| x.pos == y.pos && x.kind == y.kind && x.nbt == y.nbt)
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
                pos: BlockPos::new(
                    (x << 4) + (xz >> 4) as i32,
                    y as i32,
                    (z << 4) + (xz & 15) as i32,
                ),
                kind: kind as u32,
                nbt,
            })
        })?;
        let light = r.read()?;
        Ok(Self {
            chunk: ChunkData {
                pos: ChunkPos::new(x, z),
                heightmaps,
                sections,
                block_entities,
                light,
            },
        })
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
    fn eq(&self, o: &Self) -> bool {
        self.pos == o.pos && light_eq(&self.light, &o.light)
    }
}
impl Packet for LightUpdate {
    const NAME: &'static str = "minecraft:light_update";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            pos: ChunkPos::new(r.var_int()?, r.var_int()?),
            light: r.read()?,
        })
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
        let d3 =
            |r: &mut Reader<'_>| -> DecodeResult<[i16; 3]> { Ok([r.i16()?, r.i16()?, r.i16()?]) };
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
        Ok(Self {
            entity_id,
            deltas,
            on_ground,
        })
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
        Ok(Self {
            entity_id,
            deltas,
            on_ground,
            yaw: r.angle()?,
            pitch: r.angle()?,
        })
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
            if id == 0 {
                Ok(PreviousMessage::Signature(Bytes::copy_from_slice(
                    r.take(256)?,
                )))
            } else {
                Ok(PreviousMessage::Id(id - 1))
            }
        })?;
        let unsigned_content = r.option(|r| r.component())?;
        let filter_type = r.var_int()?;
        let filter_bits = if filter_type == 2 {
            Some(r.bit_set()?)
        } else {
            None
        };
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
        Ok(Self {
            session_id: r.uuid()?,
            expires_at: r.i64()?,
            public_key: r.byte_array(512)?,
            key_signature: r.byte_array(4096)?,
        })
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
            let mut e = PlayerInfoEntry {
                uuid: r.uuid()?,
                ..Default::default()
            };
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
        SectionPos::new(
            (v >> 42) as i32,
            ((v << 44) >> 44) as i32,
            ((v << 22) >> 42) as i32,
        )
    }
    /// Packs a section position.
    pub fn pack_section(p: SectionPos) -> i64 {
        ((p.x as i64 & 0x3F_FFFF) << 42)
            | (p.y as i64 & 0xF_FFFF)
            | ((p.z as i64 & 0x3F_FFFF) << 20)
    }
    /// The changes as absolute positions.
    pub fn changes(&self) -> impl Iterator<Item = BlockChange> + '_ {
        let base = self.section_pos().block_min();
        self.blocks.iter().map(move |b| BlockChange {
            pos: base.add(
                ((b >> 8) & 15) as i32,
                (b & 15) as i32,
                ((b >> 4) & 15) as i32,
            ),
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
        Ok(Self {
            entity_id,
            equipment,
        })
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
        Ok(Self {
            clock: r.var_int()?,
            time: r.var_long()?,
            fraction: r.f32()?,
            rate: r.f32()?,
        })
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

// ---------------------------------------------------------------- remaining clientbound packets

packet! {
    /// An entity animation (26.3 numbering: 0 leave bed, 1 critical hit,
    /// 2 magic critical hit; arm swings moved to `swing_animation`).
    Animate = "minecraft:animate" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// Animation id.
        animation: u8,
    }
}

/// One statistic in `award_stats`.
#[derive(Clone, Debug, PartialEq)]
pub struct Statistic {
    /// Index into `minecraft:stat_type`.
    pub category: i32,
    /// Id in the category's registry.
    pub stat: i32,
    /// New value.
    pub value: i32,
}
impl Wire for Statistic {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            category: r.var_int()?,
            stat: r.var_int()?,
            value: r.var_int()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.var_int(self.category);
        w.var_int(self.stat);
        w.var_int(self.value);
    }
}

packet! {
    /// Statistics (answer to `client_command` request stats).
    AwardStats = "minecraft:award_stats" {
        /// Changed statistics.
        stats: Vec<Statistic>,
    }
}

packet! {
    /// Acknowledges a client block change sequence.
    BlockChangedAck = "minecraft:block_changed_ack" {
        /// Sequence id.
        sequence: i32 as VarInt,
    }
}

packet! {
    /// Block breaking progress overlay.
    BlockDestruction = "minecraft:block_destruction" {
        /// Breaking entity (any unique id).
        entity_id: i32 as VarInt,
        /// Block.
        pos: BlockPos,
        /// 0..9, other values remove.
        stage: u8,
    }
}

packet! {
    /// Block action (chest lid, note block, piston).
    BlockEvent = "minecraft:block_event" {
        /// Block.
        pos: BlockPos,
        /// Action id.
        action: u8,
        /// Action parameter.
        param: u8,
        /// Index into `minecraft:block` (unused by vanilla clients).
        block: i32 as VarInt,
    }
}

/// The action of a `boss_event`.
#[derive(Clone, Debug, PartialEq)]
pub enum BossEventAction {
    /// 0: add.
    Add {
        /// Title.
        title: Component,
        /// Health 0..1.
        health: f32,
        /// Colour id.
        color: i32,
        /// Division id.
        division: i32,
        /// 0x01 darken sky, 0x02 dragon bar, 0x04 fog.
        flags: u8,
    },
    /// 1: remove.
    Remove,
    /// 2: health.
    UpdateHealth(f32),
    /// 3: title.
    UpdateTitle(Component),
    /// 4: style.
    UpdateStyle {
        /// Colour id.
        color: i32,
        /// Division id.
        division: i32,
    },
    /// 5: flags.
    UpdateFlags(u8),
}

/// Boss bar update.
#[derive(Clone, Debug, PartialEq)]
pub struct BossEvent {
    /// Bar id.
    pub id: Uuid,
    /// What changes.
    pub action: BossEventAction,
}
impl Packet for BossEvent {
    const NAME: &'static str = "minecraft:boss_event";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let id = r.uuid()?;
        let action = match r.var_int()? {
            0 => BossEventAction::Add {
                title: r.component()?,
                health: r.f32()?,
                color: r.var_int()?,
                division: r.var_int()?,
                flags: r.u8()?,
            },
            1 => BossEventAction::Remove,
            2 => BossEventAction::UpdateHealth(r.f32()?),
            3 => BossEventAction::UpdateTitle(r.component()?),
            4 => BossEventAction::UpdateStyle {
                color: r.var_int()?,
                division: r.var_int()?,
            },
            5 => BossEventAction::UpdateFlags(r.u8()?),
            a => return Err(DecodeError::Invalid(format!("boss event action {a}"))),
        };
        Ok(Self { id, action })
    }
    fn encode(&self, w: &mut Writer) {
        w.uuid(&self.id);
        match &self.action {
            BossEventAction::Add {
                title,
                health,
                color,
                division,
                flags,
            } => {
                w.var_int(0);
                w.component(title);
                w.f32(*health);
                w.var_int(*color);
                w.var_int(*division);
                w.u8(*flags);
            }
            BossEventAction::Remove => w.var_int(1),
            BossEventAction::UpdateHealth(h) => {
                w.var_int(2);
                w.f32(*h);
            }
            BossEventAction::UpdateTitle(t) => {
                w.var_int(3);
                w.component(t);
            }
            BossEventAction::UpdateStyle { color, division } => {
                w.var_int(4);
                w.var_int(*color);
                w.var_int(*division);
            }
            BossEventAction::UpdateFlags(f) => {
                w.var_int(5);
                w.u8(*f);
            }
        }
    }
}

/// Biome data of one chunk in `chunks_biomes`.
#[derive(Clone, Debug, PartialEq)]
pub struct ChunkBiomes {
    /// Chunk.
    pub pos: ChunkPos,
    /// Sections with only their biome paletted containers (opaque here).
    pub data: Bytes,
}
impl Wire for ChunkBiomes {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            pos: PackedChunkPos::read(r)?,
            data: r.byte_array(crate::transport::MAX_PACKET_DATA_SIZE)?,
        })
    }
    fn write(&self, w: &mut Writer) {
        PackedChunkPos::write(&self.pos, w);
        w.byte_array(&self.data);
    }
}

packet! {
    /// Biome updates for several chunks.
    ChunksBiomes = "minecraft:chunks_biomes" {
        /// Per chunk.
        chunks: Vec<ChunkBiomes>,
    }
}

packet! {
    /// Clears the title.
    ClearTitles = "minecraft:clear_titles" {
        /// Also reset the title times.
        reset: bool,
    }
}

/// One tab-completion match.
#[derive(Clone, Debug, PartialEq)]
pub struct SuggestionMatch {
    /// Text to insert.
    pub text: String,
    /// Tooltip.
    pub tooltip: Option<Component>,
}
impl Wire for SuggestionMatch {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            text: r.string()?,
            tooltip: r.read()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.string(&self.text);
        w.write(&self.tooltip);
    }
}

packet! {
    /// Command tab-completion answer.
    CommandSuggestions = "minecraft:command_suggestions" {
        /// Transaction id.
        id: i32 as VarInt,
        /// Start of the replaced text.
        start: i32 as VarInt,
        /// Length of the replaced text.
        length: i32 as VarInt,
        /// Matches.
        matches: Vec<SuggestionMatch>,
    }
}

packet! {
    /// Forcibly closes a container.
    ContainerClose = "minecraft:container_close" {
        /// Window id.
        window_id: i32 as VarInt,
    }
}

packet! {
    /// A container property (furnace progress, enchantment seeds, ...).
    ContainerSetData = "minecraft:container_set_data" {
        /// Window id.
        window_id: i32 as VarInt,
        /// Property index.
        property: i16,
        /// Value.
        value: i16,
    }
}

packet! {
    /// Item cooldown.
    Cooldown = "minecraft:cooldown" {
        /// Item id or `use_cooldown` group.
        group: Identifier,
        /// Ticks, 0 clears.
        ticks: i32 as VarInt,
    }
}

packet! {
    /// Custom chat completions (unused by vanilla servers).
    CustomChatCompletions = "minecraft:custom_chat_completions" {
        /// 0 add, 1 remove, 2 set.
        action: i32 as VarInt,
        /// Entries.
        entries: Vec<String>,
    }
}

packet! {
    /// An entity took damage.
    DamageEvent = "minecraft:damage_event" {
        /// Damaged entity.
        entity_id: i32 as VarInt,
        /// Index into `minecraft:damage_type`.
        source_type: i32 as VarInt,
        /// Responsible entity id + 1, 0 for none.
        source_cause: i32 as VarInt,
        /// Direct entity id + 1, 0 for none.
        source_direct: i32 as VarInt,
        /// Source position (`/damage`).
        source_pos: Option<DVec3>,
    }
}

packet! {
    /// Debug subscription value for a block (operators only). The update
    /// data is kept raw.
    DebugBlockValue = "minecraft:debug/block_value" {
        /// Block.
        pos: BlockPos,
        /// `Debug Subscription Update`, raw.
        update: Bytes as Rest,
    }
}

packet! {
    /// Debug subscription value for a chunk (operators only). Raw update.
    DebugChunkValue = "minecraft:debug/chunk_value" {
        /// Chunk.
        pos: ChunkPos as PackedChunkPos,
        /// `Debug Subscription Update`, raw.
        update: Bytes as Rest,
    }
}

packet! {
    /// Debug subscription value for an entity (operators only). Raw update.
    DebugEntityValue = "minecraft:debug/entity_value" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// `Debug Subscription Update`, raw.
        update: Bytes as Rest,
    }
}

packet! {
    /// Debug event (operators only), raw.
    DebugEvent = "minecraft:debug/event" {
        /// `Debug Subscription Event`, raw.
        event: Bytes as Rest,
    }
}

packet! {
    /// Debug sample (tick times) for operators.
    DebugSample = "minecraft:debug_sample" {
        /// Samples.
        sample: Vec<i64>,
        /// 0 tick time.
        sample_type: i32 as VarInt,
    }
}

/// Removes a signed chat message.
#[derive(Clone, Debug, PartialEq)]
pub struct DeleteChat {
    /// The message (cache id or full signature).
    pub message: PreviousMessage,
}
impl Packet for DeleteChat {
    const NAME: &'static str = "minecraft:delete_chat";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let id = r.var_int()?;
        let message = if id == 0 {
            PreviousMessage::Signature(Bytes::copy_from_slice(r.take(256)?))
        } else {
            PreviousMessage::Id(id - 1)
        };
        Ok(Self { message })
    }
    fn encode(&self, w: &mut Writer) {
        match &self.message {
            PreviousMessage::Id(i) => w.var_int(i + 1),
            PreviousMessage::Signature(s) => {
                w.var_int(0);
                w.bytes(s);
            }
        }
    }
}

packet! {
    /// Entity status (animations, effects; meaning per entity type).
    EntityEvent = "minecraft:entity_event" {
        /// Entity.
        entity_id: i32,
        /// Status id.
        status: i8,
    }
}

/// A weighted block particle alternative of an explosion.
#[derive(Clone, Debug, PartialEq)]
pub struct ExplosionParticle {
    /// Particle.
    pub particle: Particle,
    /// Scaling.
    pub scaling: f32,
    /// Speed.
    pub speed: f32,
    /// Weight.
    pub weight: i32,
}
impl Wire for ExplosionParticle {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            particle: r.read()?,
            scaling: r.f32()?,
            speed: r.f32()?,
            weight: r.var_int()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.write(&self.particle);
        w.f32(self.scaling);
        w.f32(self.speed);
        w.var_int(self.weight);
    }
}

packet! {
    /// An explosion.
    Explode = "minecraft:explode" {
        /// Centre.
        center: DVec3,
        /// Radius.
        radius: f32,
        /// Number of affected blocks.
        block_count: i32,
        /// Velocity added to the local player.
        player_knockback: Option<DVec3>,
        /// Explosion particle.
        particle: Particle,
        /// Sound.
        sound: IdOr<SoundEvent>,
        /// Block particle alternatives.
        block_particles: Vec<ExplosionParticle>,
        /// Play the sound (26.3).
        play_sound: bool,
    }
}

packet! {
    /// A block state shown for a limited time.
    AddTransientBlock = "minecraft:add_transient_block" {
        /// Block.
        pos: BlockPos,
        /// Block state.
        state: i32 as VarInt,
    }
}

/// A game rule value in `game_rule_values`.
#[derive(Clone, Debug, PartialEq)]
pub struct GameRuleValue {
    /// Rule.
    pub name: Identifier,
    /// Value as text.
    pub value: String,
}
impl Wire for GameRuleValue {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            name: r.identifier()?,
            value: r.string()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.identifier(&self.name);
        w.string(&self.value);
    }
}

packet! {
    /// Game rule values (answer to `client_command` 2).
    GameRuleValues = "minecraft:game_rule_values" {
        /// Rules.
        rules: Vec<GameRuleValue>,
    }
}

packet! {
    /// Game test highlight.
    GameTestHighlightPos = "minecraft:game_test_highlight_pos" {
        /// Absolute position.
        absolute: BlockPos,
        /// Relative position.
        relative: BlockPos,
    }
}

packet! {
    /// Opens a mount (horse) inventory.
    MountScreenOpen = "minecraft:mount_screen_open" {
        /// Window id.
        window_id: i32 as VarInt,
        /// Inventory columns.
        columns: i32 as VarInt,
        /// Owner entity.
        entity_id: i32,
    }
}

packet! {
    /// Hurt bobbing animation.
    HurtAnimation = "minecraft:hurt_animation" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// Damage direction yaw.
        yaw: f32,
    }
}

packet! {
    /// World border initialisation.
    InitializeBorder = "minecraft:initialize_border" {
        /// Centre x.
        x: f64,
        /// Centre z.
        z: f64,
        /// Current diameter.
        old_diameter: f64,
        /// Target diameter.
        new_diameter: f64,
        /// Milliseconds to reach the target.
        speed: i64 as VarLong,
        /// Portal teleport boundary.
        portal_boundary: i32 as VarInt,
        /// Warning distance in blocks.
        warning_blocks: i32 as VarInt,
        /// Warning time in seconds.
        warning_time: i32 as VarInt,
    }
}

packet! {
    /// Particles.
    LevelParticles = "minecraft:level_particles" {
        /// Particle (type and data).
        particle: Particle,
        /// Override the particle limiter.
        override_limiter: bool,
        /// Always visible.
        always_visible: bool,
        /// Position.
        pos: DVec3,
        /// Offset x.
        offset_x: f32,
        /// Offset y.
        offset_y: f32,
        /// Offset z.
        offset_z: f32,
        /// Max speed x.
        max_speed_x: f32,
        /// Max speed y.
        max_speed_y: f32,
        /// Max speed z.
        max_speed_z: f32,
        /// Count.
        count: i32 as VarInt,
        /// 0 default, 1 alternative, 2 alternative with speed.
        randomization: i32 as VarInt,
    }
}

packet! {
    /// Low disk space warning.
    LowDiskSpaceWarning = "minecraft:low_disk_space_warning" {}
}

/// A map decoration icon.
#[derive(Clone, Debug, PartialEq)]
pub struct MapIcon {
    /// Icon type.
    pub kind: i32,
    /// X (-128..127).
    pub x: i8,
    /// Z.
    pub z: i8,
    /// Direction 0..15.
    pub direction: i8,
    /// Display name.
    pub name: Option<Component>,
}
impl Wire for MapIcon {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            kind: r.var_int()?,
            x: r.i8()?,
            z: r.i8()?,
            direction: r.i8()?,
            name: r.read()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.var_int(self.kind);
        w.i8(self.x);
        w.i8(self.z);
        w.i8(self.direction);
        w.write(&self.name);
    }
}

/// A rectangle of map colours.
#[derive(Clone, Debug, PartialEq)]
pub struct MapPatch {
    /// Columns (> 0).
    pub columns: u8,
    /// Rows.
    pub rows: u8,
    /// X offset.
    pub x: u8,
    /// Z offset.
    pub z: u8,
    /// Colours.
    pub data: Bytes,
}

/// Map item update.
#[derive(Clone, Debug, PartialEq)]
pub struct MapItemData {
    /// Map id.
    pub map_id: i32,
    /// Scale 0..4.
    pub scale: i8,
    /// Locked.
    pub locked: bool,
    /// Icons, if updated.
    pub icons: Option<Vec<MapIcon>>,
    /// Colour patch, if any.
    pub patch: Option<MapPatch>,
}
impl Packet for MapItemData {
    const NAME: &'static str = "minecraft:map_item_data";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let map_id = r.var_int()?;
        let scale = r.i8()?;
        let locked = r.bool()?;
        let icons = r.read()?;
        let columns = r.u8()?;
        let patch = if columns > 0 {
            Some(MapPatch {
                columns,
                rows: r.u8()?,
                x: r.u8()?,
                z: r.u8()?,
                data: r.byte_array(128 * 128)?,
            })
        } else {
            None
        };
        Ok(Self {
            map_id,
            scale,
            locked,
            icons,
            patch,
        })
    }
    fn encode(&self, w: &mut Writer) {
        w.var_int(self.map_id);
        w.i8(self.scale);
        w.bool(self.locked);
        w.write(&self.icons);
        match &self.patch {
            Some(p) => {
                w.u8(p.columns);
                w.u8(p.rows);
                w.u8(p.x);
                w.u8(p.z);
                w.byte_array(&p.data);
            }
            None => w.u8(0),
        }
    }
}

/// A trade cost item (`Trade Item`): item, count and an exact component
/// list kept raw (Prefixed Array of type + data).
#[derive(Clone, Debug, PartialEq)]
pub struct TradeItem {
    /// Index into `minecraft:item`.
    pub item: i32,
    /// Count.
    pub count: i32,
    /// Raw component list including its length prefix.
    pub components_raw: Bytes,
}
impl Wire for TradeItem {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let item = r.var_int()?;
        let count = r.var_int()?;
        let start = r.peek_rest();
        crate::wire::slot::skip_exact_components(r)?;
        let used = start.len() - r.remaining();
        Ok(Self {
            item,
            count,
            components_raw: r.to_bytes(&start[..used]),
        })
    }
    fn write(&self, w: &mut Writer) {
        w.var_int(self.item);
        w.var_int(self.count);
        if self.components_raw.is_empty() {
            w.var_int(0);
        } else {
            w.bytes(&self.components_raw);
        }
    }
}

/// One villager trade.
#[derive(Clone, Debug, PartialEq)]
pub struct MerchantOffer {
    /// First input.
    pub input1: TradeItem,
    /// Output.
    pub output: ItemStackData,
    /// Second input.
    pub input2: Option<TradeItem>,
    /// Disabled.
    pub disabled: bool,
    /// Uses so far.
    pub uses: i32,
    /// Max uses.
    pub max_uses: i32,
    /// Villager XP per trade.
    pub xp: i32,
    /// Special price.
    pub special_price: i32,
    /// Price multiplier.
    pub price_multiplier: f32,
    /// Demand.
    pub demand: i32,
}
impl Wire for MerchantOffer {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            input1: r.read()?,
            output: r.read()?,
            input2: r.read()?,
            disabled: r.bool()?,
            uses: r.i32()?,
            max_uses: r.i32()?,
            xp: r.i32()?,
            special_price: r.i32()?,
            price_multiplier: r.f32()?,
            demand: r.i32()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.write(&self.input1);
        w.write(&self.output);
        w.write(&self.input2);
        w.bool(self.disabled);
        w.i32(self.uses);
        w.i32(self.max_uses);
        w.i32(self.xp);
        w.i32(self.special_price);
        w.f32(self.price_multiplier);
        w.i32(self.demand);
    }
}

packet! {
    /// Villager trades.
    MerchantOffers = "minecraft:merchant_offers" {
        /// Window id.
        window_id: i32 as VarInt,
        /// Trades.
        offers: Vec<MerchantOffer>,
        /// Villager level.
        level: i32 as VarInt,
        /// Villager experience.
        experience: i32 as VarInt,
        /// Regular villager (not wandering trader).
        regular_villager: bool,
        /// Can restock.
        can_restock: bool,
    }
}

/// One step of `move_minecart_along_track`.
#[derive(Clone, Debug, PartialEq)]
pub struct MinecartStep {
    /// Position.
    pub pos: DVec3,
    /// Velocity.
    pub velocity: DVec3,
    /// Yaw.
    pub yaw: f32,
    /// Pitch.
    pub pitch: f32,
    /// Weight.
    pub weight: f32,
}
impl Wire for MinecartStep {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            pos: r.read()?,
            velocity: r.read()?,
            yaw: r.angle()?,
            pitch: r.angle()?,
            weight: r.f32()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.write(&self.pos);
        w.write(&self.velocity);
        w.angle(self.yaw);
        w.angle(self.pitch);
        w.f32(self.weight);
    }
}

packet! {
    /// Minecart movement along a track (new minecart physics).
    MoveMinecartAlongTrack = "minecraft:move_minecart_along_track" {
        /// Minecart.
        entity_id: i32 as VarInt,
        /// Steps.
        steps: Vec<MinecartStep>,
    }
}

packet! {
    /// Teleports the client-controlled vehicle.
    MoveVehicle = "minecraft:move_vehicle" {
        /// Position.
        pos: DVec3,
        /// Yaw.
        yaw: f32,
        /// Pitch.
        pitch: f32,
    }
}

packet! {
    /// Opens a written book.
    OpenBook = "minecraft:open_book" {
        /// 0 main hand, 1 off hand.
        hand: i32 as VarInt,
    }
}

packet! {
    /// Opens a container screen.
    OpenScreen = "minecraft:open_screen" {
        /// Window id.
        window_id: i32 as VarInt,
        /// Index into `minecraft:menu`.
        kind: i32 as VarInt,
        /// Title.
        title: Component,
    }
}

packet! {
    /// Opens the sign editor.
    OpenSignEditor = "minecraft:open_sign_editor" {
        /// Sign.
        pos: BlockPos,
        /// 0 back, 1 front.
        text_slot: i32 as VarInt,
    }
}

packet! {
    /// Answer to `ping_request`.
    PongResponse = "minecraft:pong_response" {
        /// The payload sent.
        payload: i64,
    }
}

packet! {
    /// Ghost recipe in a crafting grid. The `Recipe Display` is kept raw.
    PlaceGhostRecipe = "minecraft:place_ghost_recipe" {
        /// Window id.
        window_id: i32 as VarInt,
        /// Recipe display, raw.
        display: Bytes as Rest,
    }
}

packet! {
    /// Player abilities.
    PlayerAbilities = "minecraft:player_abilities" {
        /// 0x01 invulnerable, 0x02 flying, 0x04 may fly, 0x08 instant build.
        flags: i8,
        /// Flying speed.
        flying_speed: f32,
        /// FOV modifier.
        fov_modifier: f32,
    }
}

packet! {
    /// End of combat (unused by vanilla clients).
    PlayerCombatEnd = "minecraft:player_combat_end" {
        /// Duration in ticks.
        duration: i32 as VarInt,
    }
}

packet! {
    /// Start of combat (unused by vanilla clients).
    PlayerCombatEnter = "minecraft:player_combat_enter" {}
}

packet! {
    /// The player died: death screen.
    PlayerCombatKill = "minecraft:player_combat_kill" {
        /// Player entity id.
        player_id: i32 as VarInt,
        /// Death message.
        message: Component,
    }
}

/// Entity target of `player_look_at`.
#[derive(Clone, Debug, PartialEq)]
pub struct LookAtEntity {
    /// Entity.
    pub entity_id: i32,
    /// 0 feet, 1 eyes.
    pub anchor: i32,
}
impl Wire for LookAtEntity {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            entity_id: r.var_int()?,
            anchor: r.var_int()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.var_int(self.entity_id);
        w.var_int(self.anchor);
    }
}

packet! {
    /// Rotates the player to face a point or entity.
    PlayerLookAt = "minecraft:player_look_at" {
        /// 0 feet, 1 eyes.
        from_anchor: i32 as VarInt,
        /// Target point.
        target: DVec3,
        /// Entity target.
        entity: Option<LookAtEntity>,
    }
}

packet! {
    /// Sets the player's rotation.
    PlayerRotation = "minecraft:player_rotation" {
        /// Yaw.
        yaw: f32,
        /// Yaw is relative.
        relative_yaw: bool,
        /// Pitch.
        pitch: f32,
        /// Pitch is relative.
        relative_pitch: bool,
    }
}

packet! {
    /// Removes a mob effect.
    RemoveMobEffect = "minecraft:remove_mob_effect" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// Index into `minecraft:mob_effect`.
        effect: i32 as VarInt,
    }
}

packet! {
    /// Removes a score.
    ResetScore = "minecraft:reset_score" {
        /// Score holder.
        owner: String,
        /// Objective, or all.
        objective: Option<String>,
    }
}

packet! {
    /// Switches the advancements tab.
    SelectAdvancementsTab = "minecraft:select_advancements_tab" {
        /// Tab root advancement.
        tab: Option<Identifier>,
    }
}

packet! {
    /// Action bar text.
    SetActionBarText = "minecraft:set_action_bar_text" {
        /// Text.
        text: Component,
    }
}

packet! {
    /// World border centre.
    SetBorderCenter = "minecraft:set_border_center" {
        /// X.
        x: f64,
        /// Z.
        z: f64,
    }
}

packet! {
    /// World border resize over time.
    SetBorderLerpSize = "minecraft:set_border_lerp_size" {
        /// Current diameter.
        old_diameter: f64,
        /// Target diameter.
        new_diameter: f64,
        /// Milliseconds.
        speed: i64 as VarLong,
    }
}

packet! {
    /// World border size.
    SetBorderSize = "minecraft:set_border_size" {
        /// Diameter.
        diameter: f64,
    }
}

packet! {
    /// World border warning time.
    SetBorderWarningDelay = "minecraft:set_border_warning_delay" {
        /// Seconds.
        warning_time: i32 as VarInt,
    }
}

packet! {
    /// World border warning distance.
    SetBorderWarningDistance = "minecraft:set_border_warning_distance" {
        /// Blocks.
        warning_blocks: i32 as VarInt,
    }
}

packet! {
    /// Camera entity (spectating).
    SetCamera = "minecraft:set_camera" {
        /// Entity.
        camera_id: i32 as VarInt,
    }
}

packet! {
    /// Item on the cursor.
    SetCursorItem = "minecraft:set_cursor_item" {
        /// Item.
        item: ItemStackData,
    }
}

packet! {
    /// World spawn point.
    SetDefaultSpawnPosition = "minecraft:set_default_spawn_position" {
        /// Dimension.
        dimension: Identifier,
        /// Position.
        pos: BlockPos,
        /// Yaw.
        yaw: f32,
        /// Pitch.
        pitch: f32,
    }
}

packet! {
    /// Shows a scoreboard objective in a display slot.
    SetDisplayObjective = "minecraft:set_display_objective" {
        /// 0 list, 1 sidebar, 2 below name, 3..18 team sidebars.
        position: i32 as VarInt,
        /// Objective name.
        name: String,
    }
}

packet! {
    /// Leash link.
    SetEntityLink = "minecraft:set_entity_link" {
        /// Leashed entity.
        attached: i32,
        /// Holder, -1 to detach.
        holder: i32,
    }
}

/// Score number format.
#[derive(Clone, Debug, PartialEq)]
pub enum NumberFormat {
    /// 0: show nothing.
    Blank,
    /// 1: styled, style compound as raw NBT.
    Styled(Bytes),
    /// 2: fixed text.
    Fixed(Component),
}
impl Wire for NumberFormat {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(match r.var_int()? {
            0 => NumberFormat::Blank,
            1 => NumberFormat::Styled(r.nbt_raw()?),
            2 => NumberFormat::Fixed(r.component()?),
            t => return Err(DecodeError::Invalid(format!("number format {t}"))),
        })
    }
    fn write(&self, w: &mut Writer) {
        match self {
            NumberFormat::Blank => w.var_int(0),
            NumberFormat::Styled(s) => {
                w.var_int(1);
                w.bytes(s);
            }
            NumberFormat::Fixed(c) => {
                w.var_int(2);
                w.component(c);
            }
        }
    }
}

/// Objective display data (modes 0 and 2 of `set_objective`).
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectiveDisplay {
    /// Display text.
    pub value: Component,
    /// 0 integer, 1 hearts.
    pub render_type: i32,
    /// Number format.
    pub number_format: Option<NumberFormat>,
}

/// Creates, removes or updates a scoreboard objective.
#[derive(Clone, Debug, PartialEq)]
pub struct SetObjective {
    /// Objective name.
    pub name: String,
    /// 0 create, 1 remove, 2 update.
    pub mode: i8,
    /// Present for modes 0 and 2.
    pub display: Option<ObjectiveDisplay>,
}
impl Packet for SetObjective {
    const NAME: &'static str = "minecraft:set_objective";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let name = r.string()?;
        let mode = r.i8()?;
        let display = if mode == 0 || mode == 2 {
            Some(ObjectiveDisplay {
                value: r.component()?,
                render_type: r.var_int()?,
                number_format: r.read()?,
            })
        } else {
            None
        };
        Ok(Self {
            name,
            mode,
            display,
        })
    }
    fn encode(&self, w: &mut Writer) {
        w.string(&self.name);
        w.i8(self.mode);
        if let Some(d) = &self.display {
            w.component(&d.value);
            w.var_int(d.render_type);
            w.write(&d.number_format);
        }
    }
}

packet! {
    /// Passengers of a vehicle.
    SetPassengers = "minecraft:set_passengers" {
        /// Vehicle.
        vehicle: i32 as VarInt,
        /// Passenger ids.
        passengers: Vec<i32> as List<VarInt>,
    }
}

packet! {
    /// Sets a player inventory slot directly.
    SetPlayerInventory = "minecraft:set_player_inventory" {
        /// Inventory slot index.
        slot: i32 as VarInt,
        /// Item.
        item: ItemStackData,
    }
}

/// Team display parameters (methods 0 and 2 of `set_player_team`).
///
/// Field order follows the 26.3 wiki; the colour is a Prefixed Optional
/// VarInt as Pumpkin writes it for 26.2+ (the wiki lists a plain VarInt):
/// uncertain, see NOTES.md.
#[derive(Clone, Debug, PartialEq)]
pub struct TeamParameters {
    /// Display name.
    pub display_name: Component,
    /// Prefix.
    pub prefix: Component,
    /// Suffix.
    pub suffix: Component,
    /// 0 always, 1 never, 2 hide for other teams, 3 hide for own team.
    pub name_tag_visibility: i32,
    /// 0 always, 1 never, 2 push other teams, 3 push own team.
    pub collision_rule: i32,
    /// Formatting colour id, if any.
    pub color: Option<i32>,
    /// 0x01 friendly fire, 0x02 see invisible teammates.
    pub flags: i8,
}
impl Wire for TeamParameters {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            display_name: r.component()?,
            prefix: r.component()?,
            suffix: r.component()?,
            name_tag_visibility: r.var_int()?,
            collision_rule: r.var_int()?,
            color: r.option(|r| r.var_int())?,
            flags: r.i8()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.component(&self.display_name);
        w.component(&self.prefix);
        w.component(&self.suffix);
        w.var_int(self.name_tag_visibility);
        w.var_int(self.collision_rule);
        w.option(&self.color, |w, c| w.var_int(*c));
        w.i8(self.flags);
    }
}

/// Team update.
#[derive(Clone, Debug, PartialEq)]
pub struct SetPlayerTeam {
    /// Team name.
    pub name: String,
    /// 0 create, 1 remove, 2 update, 3 add entities, 4 remove entities.
    pub method: i8,
    /// Parameters for methods 0 and 2.
    pub parameters: Option<TeamParameters>,
    /// Entities for methods 0, 3 and 4.
    pub entities: Vec<String>,
}
impl Packet for SetPlayerTeam {
    const NAME: &'static str = "minecraft:set_player_team";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let name = r.string()?;
        let method = r.i8()?;
        let parameters = if method == 0 || method == 2 {
            Some(r.read()?)
        } else {
            None
        };
        let entities = if matches!(method, 0 | 3 | 4) {
            r.read()?
        } else {
            Vec::new()
        };
        Ok(Self {
            name,
            method,
            parameters,
            entities,
        })
    }
    fn encode(&self, w: &mut Writer) {
        w.string(&self.name);
        w.i8(self.method);
        if let Some(p) = &self.parameters {
            w.write(p);
        }
        if matches!(self.method, 0 | 3 | 4) {
            w.write(&self.entities);
        }
    }
}

packet! {
    /// Sets a score.
    SetScore = "minecraft:set_score" {
        /// Score holder.
        owner: String,
        /// Objective.
        objective: String,
        /// Value.
        value: i32 as VarInt,
        /// Display name.
        display_name: Option<Component>,
        /// Number format.
        number_format: Option<NumberFormat>,
    }
}

packet! {
    /// Subtitle.
    SetSubtitleText = "minecraft:set_subtitle_text" {
        /// Text.
        text: Component,
    }
}

packet! {
    /// Title.
    SetTitleText = "minecraft:set_title_text" {
        /// Text.
        text: Component,
    }
}

packet! {
    /// Title fade timings.
    SetTitlesAnimation = "minecraft:set_titles_animation" {
        /// Fade in ticks.
        fade_in: i32,
        /// Stay ticks.
        stay: i32,
        /// Fade out ticks.
        fade_out: i32,
    }
}

/// Stops sounds by category and/or name.
#[derive(Clone, Debug, PartialEq)]
pub struct StopSound {
    /// Sound category, or all.
    pub source: Option<i32>,
    /// Sound name, or all.
    pub sound: Option<Identifier>,
}
impl Packet for StopSound {
    const NAME: &'static str = "minecraft:stop_sound";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let flags = r.u8()?;
        let source = if flags & 1 != 0 {
            Some(r.var_int()?)
        } else {
            None
        };
        let sound = if flags & 2 != 0 {
            Some(r.identifier()?)
        } else {
            None
        };
        Ok(Self { source, sound })
    }
    fn encode(&self, w: &mut Writer) {
        w.u8(self.source.is_some() as u8 | ((self.sound.is_some() as u8) << 1));
        if let Some(s) = self.source {
            w.var_int(s);
        }
        if let Some(s) = &self.sound {
            w.identifier(s);
        }
    }
}

packet! {
    /// An entity swings a hand (26.3).
    SwingAnimation = "minecraft:swing_animation" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// 0 main hand, 1 off hand.
        hand: i32 as VarInt,
        /// 0 none, 1 whack, 2 stab.
        animation: i32 as VarInt,
        /// Duration in ticks.
        duration: i32 as VarInt,
    }
}

packet! {
    /// NBT query answer.
    TagQuery = "minecraft:tag_query" {
        /// Transaction id.
        transaction_id: i32 as VarInt,
        /// NBT (raw; may be TAG_End).
        nbt: Bytes as RawNbt,
    }
}

packet! {
    /// Item pickup animation.
    TakeItemEntity = "minecraft:take_item_entity" {
        /// Picked up entity.
        collected: i32 as VarInt,
        /// Collector.
        collector: i32 as VarInt,
        /// Count.
        count: i32 as VarInt,
    }
}

packet! {
    /// Test instance block screen status. Kept raw: the wiki (status text,
    /// optional size) and Pumpkin (position, status id, optional message)
    /// disagree.
    TestInstanceBlockStatus = "minecraft:test_instance_block_status" {
        /// Raw body.
        body: Bytes as Rest,
    }
}

packet! {
    /// Tick rate and freeze state.
    TickingState = "minecraft:ticking_state" {
        /// Ticks per second.
        tick_rate: f32,
        /// Frozen.
        frozen: bool,
    }
}

packet! {
    /// Steps a frozen game.
    TickingStep = "minecraft:ticking_step" {
        /// Steps.
        steps: i32 as VarInt,
    }
}

/// Advancement display data.
#[derive(Clone, Debug, PartialEq)]
pub struct AdvancementDisplay {
    /// Title.
    pub title: Component,
    /// Description.
    pub description: Component,
    /// Icon (an item stack template on the wire).
    pub icon: ItemStackData,
    /// 0 task, 1 challenge, 2 goal.
    pub frame: i32,
    /// 0x01 background, 0x02 show toast, 0x04 hidden.
    pub flags: i32,
    /// Background texture (present when flags & 1).
    pub background: Option<Identifier>,
}
impl Wire for AdvancementDisplay {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let title = r.component()?;
        let description = r.component()?;
        let icon = r.read_as::<_, crate::wire::slot::ItemStackTemplate>()?;
        let frame = r.var_int()?;
        let flags = r.i32()?;
        let background = if flags & 1 != 0 {
            Some(r.identifier()?)
        } else {
            None
        };
        Ok(Self {
            title,
            description,
            icon,
            frame,
            flags,
            background,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.component(&self.title);
        w.component(&self.description);
        w.write_as::<_, crate::wire::slot::ItemStackTemplate>(&self.icon);
        w.var_int(self.frame);
        w.i32(self.flags);
        if let Some(b) = &self.background {
            w.identifier(b);
        }
    }
}

/// One advancement.
///
/// 26.3 layout as Pumpkin writes it: the tree position (x, y) follows the
/// telemetry flag for every advancement instead of being inside the display
/// (the wiki still shows it in the display): uncertain, see NOTES.md.
#[derive(Clone, Debug, PartialEq)]
pub struct Advancement {
    /// Parent.
    pub parent: Option<Identifier>,
    /// Display.
    pub display: Option<AdvancementDisplay>,
    /// Requirements (AND of ORs of criteria).
    pub requirements: Vec<Vec<String>>,
    /// Sends telemetry.
    pub telemetry: bool,
    /// Tree x.
    pub x: f32,
    /// Tree y.
    pub y: f32,
}
impl Wire for Advancement {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            parent: r.read()?,
            display: r.read()?,
            requirements: r.read()?,
            telemetry: r.bool()?,
            x: r.f32()?,
            y: r.f32()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.write(&self.parent);
        w.write(&self.display);
        w.write(&self.requirements);
        w.bool(self.telemetry);
        w.f32(self.x);
        w.f32(self.y);
    }
}

/// Progress of one criterion.
///
/// The criterion is the plain map key from the advancement's JSON
/// (`AdvancementProgress.STREAM_CODEC` pairs a string with a criterion), so it is carried as
/// the exact string the server wrote. Names made it an `Identifier`, which wrote a
/// `minecraft:` namespace back into the packet: the UI forwards this packet into Java in its
/// wire form, and vanilla's `ClientAdvancements` then drops the progress because the
/// criterion names no longer match the advancement's criteria.
#[derive(Clone, Debug, PartialEq)]
pub struct CriterionProgress {
    /// Criterion name (plain, not namespaced).
    pub criterion: String,
    /// When achieved (ms since epoch).
    pub achieved_at: Option<i64>,
}
impl Wire for CriterionProgress {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            criterion: r.string()?,
            achieved_at: r.read()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.string(&self.criterion);
        w.write(&self.achieved_at);
    }
}

/// Advancement entry in `update_advancements`.
#[derive(Clone, Debug, PartialEq)]
pub struct AdvancementEntry {
    /// Id.
    pub id: Identifier,
    /// Advancement.
    pub advancement: Advancement,
}
impl Wire for AdvancementEntry {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            id: r.identifier()?,
            advancement: r.read()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.identifier(&self.id);
        w.write(&self.advancement);
    }
}

/// Progress entry in `update_advancements`.
#[derive(Clone, Debug, PartialEq)]
pub struct AdvancementProgress {
    /// Advancement id.
    pub id: Identifier,
    /// Criteria.
    pub criteria: Vec<CriterionProgress>,
}
impl Wire for AdvancementProgress {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            id: r.identifier()?,
            criteria: r.read()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.identifier(&self.id);
        w.write(&self.criteria);
    }
}

packet! {
    /// Advancements.
    UpdateAdvancements = "minecraft:update_advancements" {
        /// Reset first.
        reset: bool,
        /// Added advancements.
        added: Vec<AdvancementEntry>,
        /// Removed advancement ids.
        removed: Vec<Identifier>,
        /// Progress.
        progress: Vec<AdvancementProgress>,
        /// Show the advancements toast/screen.
        show_advancements: bool,
    }
}

/// An attribute modifier.
#[derive(Clone, Debug, PartialEq)]
pub struct AttributeModifier {
    /// Modifier id.
    pub id: Identifier,
    /// Amount.
    pub amount: f64,
    /// 0 add, 1 multiply base, 2 multiply total.
    pub operation: u8,
}
impl Wire for AttributeModifier {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            id: r.identifier()?,
            amount: r.f64()?,
            operation: r.u8()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.identifier(&self.id);
        w.f64(self.amount);
        w.u8(self.operation);
    }
}

/// An attribute value with modifiers.
#[derive(Clone, Debug, PartialEq)]
pub struct AttributeProperty {
    /// Index into `minecraft:attribute`.
    pub id: i32,
    /// Base value.
    pub value: f64,
    /// Modifiers.
    pub modifiers: Vec<AttributeModifier>,
}
impl Wire for AttributeProperty {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            id: r.var_int()?,
            value: r.f64()?,
            modifiers: r.read()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.var_int(self.id);
        w.f64(self.value);
        w.write(&self.modifiers);
    }
}

packet! {
    /// Entity attributes.
    UpdateAttributes = "minecraft:update_attributes" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// Attributes.
        properties: Vec<AttributeProperty>,
    }
}

packet! {
    /// Adds or updates a mob effect.
    UpdateMobEffect = "minecraft:update_mob_effect" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// Index into `minecraft:mob_effect`.
        effect: i32 as VarInt,
        /// Amplifier (level - 1).
        amplifier: i32 as VarInt,
        /// Duration in ticks, -1 infinite.
        duration: i32 as VarInt,
        /// 0x01 ambient, 0x02 show particles, 0x04 show icon, 0x08 blend.
        flags: u8,
    }
}

packet! {
    /// Projectile acceleration power.
    ProjectilePower = "minecraft:projectile_power" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// Power.
        power: f64,
    }
}

packet! {
    /// Locator bar waypoint. Kept raw: the wiki and Pumpkin disagree on the
    /// identifier, colour and waypoint type encodings.
    Waypoint = "minecraft:waypoint" {
        /// Raw body.
        body: Bytes as Rest,
    }
}

/// A dialog as `ID or NBT`: a `minecraft:dialog` registry id or an inline
/// definition kept as raw network NBT.
#[derive(Clone, Debug, PartialEq)]
pub struct DialogNbt(pub Bytes);
impl Wire for DialogNbt {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self(r.nbt_raw()?))
    }
    fn write(&self, w: &mut Writer) {
        w.bytes(&self.0)
    }
}

packet! {
    /// Shows a dialog.
    ShowDialog = "minecraft:show_dialog" {
        /// Registry id or inline NBT.
        dialog: IdOr<DialogNbt>,
    }
}

#[cfg(test)]
mod more_tests {
    use super::*;
    use crate::packets::test_util::round_trip;
    use crate::packets::ProtocolTables;

    fn tables() -> ProtocolTables {
        ProtocolTables::for_tests(&[(3, "minecraft:damage")]).with_particles_for_tests(&[
            (0, "minecraft:explosion"),
            (1, "minecraft:block"),
            (2, "minecraft:dust"),
        ])
    }

    fn round_trip_tables<P: Packet + PartialEq + std::fmt::Debug>(p: &P) {
        let t = tables();
        let mut w = Writer::new();
        p.encode(&mut w);
        let b = Bytes::from(w.into_inner());
        let mut r = Reader::from_bytes(&b).with_tables(&t);
        let back = P::decode_exact(&mut r).unwrap_or_else(|e| panic!("{}: {e}", P::NAME));
        assert_eq!(&back, p, "{}", P::NAME);
    }

    fn item() -> ItemStackData {
        ItemStackData {
            count: 1,
            item: Some(RegistryIndex(7)),
            components_raw: Bytes::from_static(&[0, 0]),
        }
    }
    fn nbt() -> Bytes {
        let mut c = crate::nbt::NbtCompound::new();
        c.put_string("color", "red".into());
        let mut w = Writer::new();
        w.nbt_compound(Some(&c));
        w.into_inner().into()
    }

    #[test]
    fn round_trips() {
        round_trip(&Animate {
            entity_id: 1,
            animation: 2,
        });
        round_trip(&AwardStats {
            stats: vec![Statistic {
                category: 8,
                stat: 3,
                value: 100,
            }],
        });
        round_trip(&BlockChangedAck { sequence: 5 });
        round_trip(&BlockDestruction {
            entity_id: 1,
            pos: BlockPos::new(1, 2, 3),
            stage: 4,
        });
        round_trip(&BlockEvent {
            pos: BlockPos::new(0, 1, 0),
            action: 1,
            param: 1,
            block: 200,
        });
        let id = Uuid::from_u128(1);
        for action in [
            BossEventAction::Add {
                title: Component::text("Wither"),
                health: 1.0,
                color: 5,
                division: 0,
                flags: 1,
            },
            BossEventAction::Remove,
            BossEventAction::UpdateHealth(0.5),
            BossEventAction::UpdateTitle(Component::text("t")),
            BossEventAction::UpdateStyle {
                color: 1,
                division: 2,
            },
            BossEventAction::UpdateFlags(4),
        ] {
            round_trip(&BossEvent { id, action });
        }
        round_trip(&ChunksBiomes {
            chunks: vec![ChunkBiomes {
                pos: ChunkPos::new(-1, 3),
                data: Bytes::from_static(&[0, 1, 2]),
            }],
        });
        round_trip(&ClearTitles { reset: true });
        round_trip(&CommandSuggestions {
            id: 1,
            start: 2,
            length: 3,
            matches: vec![SuggestionMatch {
                text: "a".into(),
                tooltip: Some(Component::text("t")),
            }],
        });
        round_trip(&ContainerClose { window_id: 3 });
        round_trip(&ContainerSetData {
            window_id: 1,
            property: 2,
            value: -1,
        });
        round_trip(&Cooldown {
            group: Identifier::minecraft("ender_pearl"),
            ticks: 20,
        });
        round_trip(&CustomChatCompletions {
            action: 2,
            entries: vec!["x".into()],
        });
        round_trip(&DamageEvent {
            entity_id: 1,
            source_type: 2,
            source_cause: 0,
            source_direct: 0,
            source_pos: Some(DVec3::ONE),
        });
        round_trip(&DebugBlockValue {
            pos: BlockPos::new(1, 1, 1),
            update: Bytes::from_static(&[0, 0]),
        });
        round_trip(&DebugChunkValue {
            pos: ChunkPos::new(1, 2),
            update: Bytes::from_static(&[1]),
        });
        round_trip(&DebugEntityValue {
            entity_id: 3,
            update: Bytes::new(),
        });
        round_trip(&DebugEvent {
            event: Bytes::from_static(&[0]),
        });
        round_trip(&DebugSample {
            sample: vec![1, 2, 3, 4],
            sample_type: 0,
        });
        round_trip(&DeleteChat {
            message: PreviousMessage::Id(4),
        });
        round_trip(&DeleteChat {
            message: PreviousMessage::Signature(Bytes::from(vec![2; 256])),
        });
        round_trip(&EntityEvent {
            entity_id: 5,
            status: 9,
        });
        round_trip_tables(&Explode {
            center: DVec3::new(1.0, 2.0, 3.0),
            radius: 4.0,
            block_count: 10,
            player_knockback: Some(DVec3::X),
            particle: Particle {
                kind: 0,
                data: Bytes::new(),
            },
            sound: IdOr::Id(5),
            block_particles: vec![ExplosionParticle {
                particle: Particle {
                    kind: 1,
                    data: Bytes::from_static(&[9]),
                },
                scaling: 1.0,
                speed: 0.5,
                weight: 3,
            }],
            play_sound: true,
        });
        round_trip(&AddTransientBlock {
            pos: BlockPos::new(0, 0, 0),
            state: 1,
        });
        round_trip(&GameRuleValues {
            rules: vec![GameRuleValue {
                name: Identifier::minecraft("keep_inventory"),
                value: "true".into(),
            }],
        });
        round_trip(&GameTestHighlightPos {
            absolute: BlockPos::new(1, 2, 3),
            relative: BlockPos::new(0, 0, 1),
        });
        round_trip(&MountScreenOpen {
            window_id: 1,
            columns: 5,
            entity_id: 9,
        });
        round_trip(&HurtAnimation {
            entity_id: 1,
            yaw: 90.0,
        });
        round_trip(&InitializeBorder {
            x: 0.0,
            z: 0.0,
            old_diameter: 1e7,
            new_diameter: 1e7,
            speed: 0,
            portal_boundary: 29999984,
            warning_blocks: 5,
            warning_time: 15,
        });
        round_trip_tables(&LevelParticles {
            particle: Particle {
                kind: 2,
                data: Bytes::from_static(&[0, 0, 0xff, 0, 0x3f, 0x80, 0, 0]),
            },
            override_limiter: false,
            always_visible: true,
            pos: DVec3::ONE,
            offset_x: 0.1,
            offset_y: 0.2,
            offset_z: 0.3,
            max_speed_x: 1.0,
            max_speed_y: 1.0,
            max_speed_z: 1.0,
            count: 10,
            randomization: 0,
        });
        round_trip(&LowDiskSpaceWarning {});
        round_trip(&MapItemData {
            map_id: 1,
            scale: 0,
            locked: false,
            icons: Some(vec![MapIcon {
                kind: 0,
                x: 1,
                z: -1,
                direction: 3,
                name: None,
            }]),
            patch: Some(MapPatch {
                columns: 1,
                rows: 2,
                x: 0,
                z: 0,
                data: Bytes::from_static(&[4, 5]),
            }),
        });
        round_trip(&MapItemData {
            map_id: 1,
            scale: 4,
            locked: true,
            icons: None,
            patch: None,
        });
        round_trip_tables(&MerchantOffers {
            window_id: 1,
            offers: vec![MerchantOffer {
                input1: TradeItem {
                    item: 1,
                    count: 12,
                    components_raw: Bytes::from_static(&[1, 3, 5]),
                },
                output: item(),
                input2: None,
                disabled: false,
                uses: 0,
                max_uses: 12,
                xp: 1,
                special_price: 0,
                price_multiplier: 0.05,
                demand: 0,
            }],
            level: 1,
            experience: 0,
            regular_villager: true,
            can_restock: true,
        });
        round_trip(&MoveMinecartAlongTrack {
            entity_id: 1,
            steps: vec![MinecartStep {
                pos: DVec3::ONE,
                velocity: DVec3::ZERO,
                yaw: 90.0,
                pitch: 0.0,
                weight: 1.0,
            }],
        });
        round_trip(&MoveVehicle {
            pos: DVec3::ONE,
            yaw: 1.0,
            pitch: 2.0,
        });
        round_trip(&OpenBook { hand: 0 });
        round_trip(&OpenScreen {
            window_id: 1,
            kind: 2,
            title: Component::text("Chest"),
        });
        round_trip(&OpenSignEditor {
            pos: BlockPos::new(1, 2, 3),
            text_slot: 1,
        });
        round_trip(&PongResponse { payload: 77 });
        round_trip(&PlaceGhostRecipe {
            window_id: 1,
            display: Bytes::from_static(&[3, 0]),
        });
        round_trip(&PlayerAbilities {
            flags: 0x0d,
            flying_speed: 0.05,
            fov_modifier: 0.1,
        });
        round_trip(&PlayerCombatEnd { duration: 5 });
        round_trip(&PlayerCombatEnter {});
        round_trip(&PlayerCombatKill {
            player_id: 1,
            message: Component::text("died"),
        });
        round_trip(&PlayerLookAt {
            from_anchor: 1,
            target: DVec3::ONE,
            entity: Some(LookAtEntity {
                entity_id: 3,
                anchor: 0,
            }),
        });
        round_trip(&PlayerLookAt {
            from_anchor: 0,
            target: DVec3::ZERO,
            entity: None,
        });
        round_trip(&PlayerRotation {
            yaw: 10.0,
            relative_yaw: true,
            pitch: -5.0,
            relative_pitch: false,
        });
        round_trip(&RemoveMobEffect {
            entity_id: 1,
            effect: 2,
        });
        round_trip(&ResetScore {
            owner: "Steve".into(),
            objective: Some("kills".into()),
        });
        round_trip(&SelectAdvancementsTab {
            tab: Some(Identifier::minecraft("story/root")),
        });
        round_trip(&SetActionBarText {
            text: Component::text("a"),
        });
        round_trip(&SetBorderCenter { x: 1.0, z: 2.0 });
        round_trip(&SetBorderLerpSize {
            old_diameter: 1.0,
            new_diameter: 2.0,
            speed: 1000,
        });
        round_trip(&SetBorderSize { diameter: 100.0 });
        round_trip(&SetBorderWarningDelay { warning_time: 15 });
        round_trip(&SetBorderWarningDistance { warning_blocks: 5 });
        round_trip(&SetCamera { camera_id: 1 });
        round_trip(&SetCursorItem { item: item() });
        round_trip(&SetDefaultSpawnPosition {
            dimension: Identifier::minecraft("overworld"),
            pos: BlockPos::new(0, 64, 0),
            yaw: 0.0,
            pitch: 0.0,
        });
        round_trip(&SetDisplayObjective {
            position: 1,
            name: "obj".into(),
        });
        round_trip(&SetEntityLink {
            attached: 1,
            holder: -1,
        });
        round_trip(&SetObjective {
            name: "o".into(),
            mode: 0,
            display: Some(ObjectiveDisplay {
                value: Component::text("O"),
                render_type: 1,
                number_format: Some(NumberFormat::Styled(nbt())),
            }),
        });
        round_trip(&SetObjective {
            name: "o".into(),
            mode: 1,
            display: None,
        });
        round_trip(&SetPassengers {
            vehicle: 1,
            passengers: vec![2, 3],
        });
        round_trip(&SetPlayerInventory {
            slot: 40,
            item: item(),
        });
        round_trip(&SetPlayerTeam {
            name: "red".into(),
            method: 0,
            parameters: Some(TeamParameters {
                display_name: Component::text("Red"),
                prefix: Component::default(),
                suffix: Component::default(),
                name_tag_visibility: 0,
                collision_rule: 0,
                color: Some(12),
                flags: 3,
            }),
            entities: vec!["Steve".into()],
        });
        round_trip(&SetPlayerTeam {
            name: "red".into(),
            method: 1,
            parameters: None,
            entities: vec![],
        });
        round_trip(&SetPlayerTeam {
            name: "red".into(),
            method: 4,
            parameters: None,
            entities: vec!["Alex".into()],
        });
        round_trip(&SetScore {
            owner: "Steve".into(),
            objective: "o".into(),
            value: 5,
            display_name: None,
            number_format: Some(NumberFormat::Fixed(Component::text("x"))),
        });
        round_trip(&SetScore {
            owner: "a".into(),
            objective: "b".into(),
            value: -1,
            display_name: Some(Component::text("n")),
            number_format: Some(NumberFormat::Blank),
        });
        round_trip(&SetSubtitleText {
            text: Component::text("s"),
        });
        round_trip(&SetTitleText {
            text: Component::text("t"),
        });
        round_trip(&SetTitlesAnimation {
            fade_in: 10,
            stay: 70,
            fade_out: 20,
        });
        round_trip(&StopSound {
            source: Some(3),
            sound: None,
        });
        round_trip(&StopSound {
            source: None,
            sound: Some(Identifier::minecraft("music.game")),
        });
        round_trip(&StopSound {
            source: Some(0),
            sound: Some(Identifier::minecraft("x")),
        });
        round_trip(&SwingAnimation {
            entity_id: 1,
            hand: 0,
            animation: 1,
            duration: 6,
        });
        round_trip(&TagQuery {
            transaction_id: 1,
            nbt: nbt(),
        });
        round_trip(&TagQuery {
            transaction_id: 1,
            nbt: Bytes::from_static(&[0]),
        });
        round_trip(&TakeItemEntity {
            collected: 1,
            collector: 2,
            count: 3,
        });
        round_trip(&TestInstanceBlockStatus {
            body: Bytes::from_static(&[8, 0, 1, b'a', 0]),
        });
        round_trip(&TickingState {
            tick_rate: 20.0,
            frozen: false,
        });
        round_trip(&TickingStep { steps: 3 });
        round_trip(&UpdateAdvancements {
            reset: true,
            added: vec![AdvancementEntry {
                id: Identifier::minecraft("story/root"),
                advancement: Advancement {
                    parent: None,
                    display: Some(AdvancementDisplay {
                        title: Component::text("Minecraft"),
                        description: Component::text("d"),
                        icon: item(),
                        frame: 0,
                        flags: 1,
                        background: Some(Identifier::minecraft(
                            "gui/advancements/backgrounds/stone",
                        )),
                    }),
                    requirements: vec![vec!["crafting_table".into()]],
                    telemetry: true,
                    x: 0.0,
                    y: 1.5,
                },
            }],
            removed: vec![Identifier::minecraft("old")],
            progress: vec![AdvancementProgress {
                id: Identifier::minecraft("story/root"),
                criteria: vec![CriterionProgress {
                    criterion: "crafting_table".into(),
                    achieved_at: Some(1),
                }],
            }],
            show_advancements: false,
        });
        round_trip(&UpdateAttributes {
            entity_id: 1,
            properties: vec![AttributeProperty {
                id: 3,
                value: 0.1,
                modifiers: vec![AttributeModifier {
                    id: Identifier::minecraft("sprinting"),
                    amount: 0.3,
                    operation: 2,
                }],
            }],
        });
        round_trip(&UpdateMobEffect {
            entity_id: 1,
            effect: 2,
            amplifier: 0,
            duration: -1,
            flags: 6,
        });
        round_trip(&ProjectilePower {
            entity_id: 1,
            power: 0.1,
        });
        round_trip(&Waypoint {
            body: Bytes::from_static(&[0]),
        });
        round_trip(&ShowDialog {
            dialog: IdOr::Id(3),
        });
        round_trip(&ShowDialog {
            dialog: IdOr::Inline(DialogNbt(nbt())),
        });
    }

    /// A body written the way vanilla writes it decodes to the same bytes again: the UI
    /// forwards `update_advancements` into Java in its wire form, so a name written
    /// differently than it was read would hand vanilla a different advancement tree.
    #[test]
    fn update_advancements_keeps_the_servers_bytes() {
        let mut w = Writer::new();
        // should_reset, then two added advancements (a root and its child, both without a
        // display, as a vanilla server sends a recipe-only tree).
        w.bool(true);
        w.var_int(2);
        w.identifier(&Identifier::minecraft("story/root"));
        w.bool(false);
        w.bool(false);
        w.var_int(1);
        w.var_int(1);
        w.string("crafting_table");
        w.bool(false);
        w.f32(0.0);
        w.f32(0.0);
        w.identifier(&Identifier::minecraft("story/mine_stone"));
        w.bool(true);
        w.identifier(&Identifier::minecraft("story/root"));
        w.bool(false);
        w.var_int(1);
        w.var_int(1);
        w.string("crafting_table");
        w.bool(false);
        w.f32(0.0);
        w.f32(0.0);
        // one removed advancement.
        w.var_int(1);
        w.identifier(&Identifier::minecraft("story/gone"));
        // progress for the root: its criteria by their plain names, with the obtained
        // instants (a missing instant is a criterion that is not done).
        w.var_int(1);
        w.identifier(&Identifier::minecraft("story/root"));
        w.var_int(2);
        w.string("crafting_table");
        w.bool(true);
        w.i64(1_770_000_000_000);
        w.string("unlock_right_away");
        w.bool(false);
        w.bool(true);
        let body = Bytes::from(w.into_inner());
        let mut r = Reader::from_bytes(&body);
        let packet = UpdateAdvancements::decode_exact(&mut r)
            .unwrap_or_else(|e| panic!("{}: {e}", UpdateAdvancements::NAME));
        let mut w = Writer::new();
        packet.encode(&mut w);
        assert_eq!(w.into_inner(), body, "re-encoded bytes differ");
        assert_eq!(
            packet.progress[0].criteria[0].criterion, "crafting_table",
            "the criterion names are the plain strings the server used"
        );
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
        ItemStackData {
            count: 2,
            item: Some(RegistryIndex(1)),
            components_raw: Bytes::from_static(&[0, 0]),
        }
    }
    fn chat_type() -> ChatType {
        let d = ChatDecoration {
            translation_key: "chat.type.text".into(),
            parameters: vec![0, 2],
            style: Bytes::from_static(&[0]),
        };
        ChatType {
            chat: d.clone(),
            narration: d,
        }
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
            death_location: Some(GlobalPos {
                dimension: Identifier::minecraft("the_nether"),
                pos: BlockPos::new(1, 2, 3),
            }),
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
        round_trip(&BlockEntityData {
            pos: BlockPos::new(1, 2, 3),
            kind: 7,
            nbt: nbt(),
        });
        round_trip(&BlockUpdate {
            pos: BlockPos::new(-1, -64, 5),
            state: 1234,
        });
        round_trip(&ChangeDifficulty {
            difficulty: 2,
            locked: false,
        });
        round_trip(&ChunkBatchFinished { batch_size: 25 });
        round_trip(&ChunkBatchStart {});
        round_trip(&Commands {
            body: Bytes::from_static(&[1, 2, 3]),
        });
        round_trip(&ContainerSetContent {
            window_id: 0,
            state_id: 1,
            slots: vec![ItemStackData::default(), item()],
            carried: ItemStackData::default(),
        });
        round_trip(&ContainerSetSlot {
            window_id: 0,
            state_id: 2,
            slot: 36,
            item: item(),
        });
        round_trip(&DisguisedChat {
            message: Component::text("hi"),
            chat_type: IdOr::Inline(chat_type()),
            sender_name: Component::text("Server"),
            target_name: None,
        });
        round_trip(&EntityPositionSync {
            entity_id: 1,
            path: PositionPath::Linear(DVec3::new(1.0, 2.0, 3.0)),
            yaw: 10.0,
            pitch: -5.0,
            on_ground: true,
        });
        round_trip(&EntityPositionSync {
            entity_id: 1,
            path: PositionPath::Stepped(vec![(DVec3::X, 1), (DVec3::Y, 2)]),
            yaw: 0.0,
            pitch: 0.0,
            on_ground: false,
        });
        round_trip(&ForgetLevelChunk {
            pos: ChunkPos::new(-3, 7),
        });
        round_trip(&GameEvent {
            event: 13,
            value: 0.0,
        });
        round_trip(&LevelChunkWithLight {
            chunk: ChunkData {
                pos: ChunkPos::new(-2, 5),
                heightmaps: vec![(1, vec![1, 2, 3]), (4, vec![])],
                sections: Bytes::from_static(&[0, 1, 0, 0]),
                block_entities: vec![ChunkBlockEntity {
                    pos: BlockPos::new(-32 + 3, -60, 80 + 15),
                    kind: 9,
                    nbt: nbt(),
                }],
                light: LightData {
                    sky_mask: vec![3],
                    block_mask: vec![],
                    empty_sky_mask: vec![4],
                    empty_block_mask: vec![],
                    sky_arrays: vec![Bytes::from(vec![0xff; 2048]), Bytes::from(vec![0; 2048])],
                    block_arrays: vec![],
                },
            },
        });
        round_trip(&LevelEvent {
            event: 1000,
            pos: BlockPos::new(0, 0, 0),
            data: 0,
            global: false,
        });
        round_trip(&LightUpdate {
            pos: ChunkPos::new(1, 2),
            light: LightData::default(),
        });
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
        round_trip(&MoveEntityPos {
            entity_id: 3,
            deltas: MoveDeltas::Linear([4096, -1, 0]),
            on_ground: true,
        });
        round_trip(&MoveEntityPos {
            entity_id: 3,
            deltas: MoveDeltas::Stepped(vec![(0, [1, 2, 3]), (1, [4, 5, 6])]),
            on_ground: false,
        });
        round_trip(&MoveEntityPosRot {
            entity_id: 3,
            deltas: MoveDeltas::Linear([0, 0, 1]),
            on_ground: false,
            yaw: 90.0,
            pitch: 0.0,
        });
        round_trip(&MoveEntityRot {
            entity_id: 3,
            on_ground: true,
            yaw: 45.0,
            pitch: 270.0,
        });
        round_trip(&PlayerChat {
            global_index: 0,
            sender: Uuid::from_u128(1),
            index: 0,
            signature: Some(Bytes::from(vec![7; 256])),
            message: "hello".into(),
            timestamp: 1,
            salt: 2,
            previous: vec![
                PreviousMessage::Id(3),
                PreviousMessage::Signature(Bytes::from(vec![1; 256])),
            ],
            unsigned_content: None,
            filter_type: 2,
            filter_bits: Some(vec![5]),
            chat_type: IdOr::Id(0),
            sender_name: Component::text("Steve"),
            target_name: Some(Component::text("Alex")),
        });
        round_trip(&PlayerInfoRemove {
            uuids: vec![Uuid::from_u128(9)],
        });
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
        round_trip(&PlayerInfoUpdate {
            actions: player_info_actions::UPDATE_LATENCY,
            entries: vec![PlayerInfoEntry {
                uuid: Uuid::from_u128(2),
                latency: Some(5),
                ..Default::default()
            }],
        });
        round_trip(&PlayerPosition {
            teleport_id: 1,
            pos: DVec3::new(0.5, 70.0, 0.5),
            velocity: DVec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            flags: teleport_flags::VEL_X | teleport_flags::YAW,
        });
        round_trip(&RecipeBookAdd {
            body: Bytes::from_static(&[0, 1]),
        });
        round_trip(&RecipeBookRemove { body: Bytes::new() });
        round_trip(&RecipeBookSettings {
            body: Bytes::from_static(&[0; 8]),
        });
        round_trip(&RemoveEntities {
            entity_ids: vec![1, 2, 300],
        });
        round_trip(&Respawn {
            spawn: spawn_info(),
            data_kept: 3,
        });
        round_trip(&RotateHead {
            entity_id: 2,
            head_yaw: 33.75,
        });
        let p = SectionPos::new(-3, -4, 1_000_000);
        let u = SectionBlocksUpdate {
            section: SectionBlocksUpdate::pack_section(p),
            blocks: vec![(5 << 12) | (1 << 8) | (2 << 4) | 3],
        };
        assert_eq!(u.section_pos(), p);
        let c: Vec<_> = u.changes().collect();
        assert_eq!(c[0].pos, BlockPos::new(-48 + 1, -64 + 3, 16_000_000 + 2));
        assert_eq!(c[0].state, BlockStateId(5));
        round_trip(&u);
        round_trip(&ServerData {
            motd: Component::text("A Minecraft Server"),
            icon: None,
        });
        round_trip(&SetChunkCacheCenter { x: -1, z: 2 });
        round_trip(&SetChunkCacheRadius { radius: 10 });
        round_trip(&SetEntityData {
            entity_id: 1,
            metadata: vec![MetadataEntry {
                index: 0,
                value: MetadataValue::Byte(0),
            }],
        });
        round_trip(&SetEntityMotion {
            entity_id: 1,
            velocity: DVec3::ZERO,
        });
        round_trip(&SetEquipment {
            entity_id: 5,
            equipment: vec![(0, item()), (5, ItemStackData::default())],
        });
        round_trip(&SetExperience {
            bar: 0.5,
            level: 3,
            total: 40,
        });
        round_trip(&SetHealth {
            health: 20.0,
            food: 20,
            saturation: 5.0,
        });
        round_trip(&SetHeldSlot { slot: 4 });
        round_trip(&SetSimulationDistance { distance: 12 });
        round_trip(&SetTime {
            world_age: 1000,
            clocks: vec![ClockState {
                clock: 0,
                time: 6000,
                fraction: 0.0,
                rate: 1.0,
            }],
        });
        round_trip(&Sound {
            sound: IdOr::Id(10),
            category: 0,
            x: 8,
            y: 16,
            z: -8,
            volume: 1.0,
            pitch: 1.0,
            seed: 99,
        });
        round_trip(&SoundEntity {
            sound: IdOr::Inline(SoundEvent {
                name: Identifier::minecraft("x"),
                fixed_range: None,
            }),
            category: 5,
            entity_id: 3,
            volume: 0.5,
            pitch: 2.0,
            seed: 0,
        });
        round_trip(&StartConfiguration {});
        round_trip(&SystemChat {
            content: Component::text("sys"),
            overlay: true,
        });
        round_trip(&TabList {
            header: Component::text("h"),
            footer: Component::default(),
        });
        round_trip(&TeleportEntity {
            entity_id: 9,
            pos: DVec3::ONE,
            velocity: DVec3::ZERO,
            yaw: 1.0,
            pitch: 2.0,
            flags: 0,
            on_ground: true,
        });
        round_trip(&UpdateRecipes {
            body: Bytes::from_static(&[0, 0]),
        });
    }

    /// Regression: a normal-terrain chunk, built byte by byte in the 26.3
    /// layout (BitSets are trimmed little-endian byte arrays). Before the fix
    /// the masks were read as VarLongs and every such packet failed.
    #[test]
    fn level_chunk_with_light_golden_bytes() {
        let mut b = vec![];
        b.extend_from_slice(&(-3i32).to_be_bytes());
        b.extend_from_slice(&7i32.to_be_bytes());
        // Heightmaps: 1 entry, type 1 (world surface), 2 longs.
        b.extend_from_slice(&[1, 1, 2]);
        b.extend_from_slice(&0x0102_0304_0506_0708i64.to_be_bytes());
        b.extend_from_slice(&(-1i64).to_be_bytes());
        // Sections blob: 3 opaque bytes.
        b.extend_from_slice(&[3, 0xAA, 0xBB, 0xCC]);
        // One block entity at (x=5, z=9) in the chunk, y = -12, kind 33, empty compound.
        b.extend_from_slice(&[1, 0x59]);
        b.extend_from_slice(&(-12i16).to_be_bytes());
        b.extend_from_slice(&[33, 10, 0]);
        // Sky mask: sections 1..=2 and 25 (0x0200_0006): bytes 06 00 00 02.
        b.extend_from_slice(&[4, 0x06, 0x00, 0x00, 0x02]);
        // Block mask: bit 3 only: one byte.
        b.extend_from_slice(&[1, 0x08]);
        // Empty sky mask: bit 0: one byte. Empty block mask: none.
        b.extend_from_slice(&[1, 0x01, 0]);
        // Three sky arrays, one block array, each VarInt 2048 (0x80 0x10) + bytes.
        b.push(3);
        for v in [0xFFu8, 0x11, 0x22] {
            b.extend_from_slice(&[0x80, 0x10]);
            b.extend(std::iter::repeat_n(v, 2048));
        }
        b.push(1);
        b.extend_from_slice(&[0x80, 0x10]);
        b.extend(std::iter::repeat_n(0x33u8, 2048));

        let p = LevelChunkWithLight::decode_exact(&mut Reader::new(&b)).unwrap();
        let c = &p.chunk;
        assert_eq!(c.pos, ChunkPos::new(-3, 7));
        assert_eq!(c.heightmaps, vec![(1, vec![0x0102_0304_0506_0708, -1])]);
        assert_eq!(&c.sections[..], &[0xAA, 0xBB, 0xCC]);
        assert_eq!(
            c.block_entities[0].pos,
            BlockPos::new(-48 + 5, -12, 112 + 9)
        );
        assert_eq!(c.block_entities[0].kind, 33);
        assert_eq!(c.light.sky_mask, vec![0x0200_0006]);
        assert_eq!(c.light.block_mask, vec![8]);
        assert_eq!(c.light.empty_sky_mask, vec![1]);
        assert!(c.light.empty_block_mask.is_empty());
        assert_eq!(c.light.sky_arrays.len(), 3);
        assert_eq!(c.light.sky_arrays[2][0], 0x22);
        assert_eq!(c.light.block_arrays[0][2047], 0x33);
        // And our encoder writes the same bytes back.
        let mut w = Writer::new();
        p.encode(&mut w);
        assert_eq!(w.into_inner(), b);

        // light_update: VarInt chunk coords + the same light data layout.
        let mut l = vec![0x7F, 0x01]; // x = 127, z = 1
        l.extend_from_slice(&[2, 0xFE, 0x01, 0, 1, 0x01, 0, 1]);
        l.extend_from_slice(&[0x80, 0x10]);
        l.extend(std::iter::repeat_n(0xF0u8, 2048));
        l.push(0);
        let u = LightUpdate::decode_exact(&mut Reader::new(&l)).unwrap();
        assert_eq!(u.pos, ChunkPos::new(127, 1));
        assert_eq!(u.light.sky_mask, vec![0x01FE]);
        assert_eq!(u.light.empty_sky_mask, vec![1]);
        assert_eq!(u.light.sky_arrays.len(), 1);
    }

    #[test]
    fn add_entity_converts_to_spawn() {
        let a = AddEntity {
            entity_id: 7,
            uuid: Uuid::nil(),
            kind: 3,
            pos: DVec3::ONE,
            velocity: DVec3::ZERO,
            pitch: 1.0,
            yaw: 2.0,
            head_yaw: 3.0,
            data: 4,
        };
        let s = a.spawn();
        assert_eq!(s.id, EntityId(7));
        assert_eq!((s.yaw, s.pitch, s.head_yaw), (2.0, 1.0, 3.0));
    }
}
