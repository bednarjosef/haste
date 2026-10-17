use super::*;

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
