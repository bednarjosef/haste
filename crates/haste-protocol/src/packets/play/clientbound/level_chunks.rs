use super::*;

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
    /// Low disk space warning.
    LowDiskSpaceWarning = "minecraft:low_disk_space_warning" {}
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
