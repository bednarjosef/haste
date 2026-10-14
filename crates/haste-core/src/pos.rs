use glam::{DVec3, IVec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub struct BlockPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl BlockPos {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }
    pub fn from_dvec3(v: DVec3) -> Self {
        Self::new(v.x.floor() as i32, v.y.floor() as i32, v.z.floor() as i32)
    }
    pub fn chunk(&self) -> ChunkPos {
        ChunkPos::new(self.x >> 4, self.z >> 4)
    }
    pub fn section(&self) -> SectionPos {
        SectionPos::new(self.x >> 4, self.y >> 4, self.z >> 4)
    }
    pub fn offset(&self, d: Direction) -> Self {
        let o = d.offset();
        Self::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
    pub fn add(&self, x: i32, y: i32, z: i32) -> Self {
        Self::new(self.x + x, self.y + y, self.z + z)
    }
    pub fn as_dvec3(&self) -> DVec3 {
        DVec3::new(self.x as f64, self.y as f64, self.z as f64)
    }
    pub fn as_ivec3(&self) -> IVec3 {
        IVec3::new(self.x, self.y, self.z)
    }
    /// Vanilla's packed long encoding (x: 26 bits, z: 26 bits, y: 12 bits).
    pub fn to_packed(&self) -> i64 {
        ((self.x as i64 & 0x3FF_FFFF) << 38)
            | ((self.z as i64 & 0x3FF_FFFF) << 12)
            | (self.y as i64 & 0xFFF)
    }
    pub fn from_packed(v: i64) -> Self {
        Self::new(
            (v >> 38) as i32,
            ((v << 52) >> 52) as i32,
            ((v << 26) >> 38) as i32,
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub struct ChunkPos {
    pub x: i32,
    pub z: i32,
}
impl ChunkPos {
    pub const fn new(x: i32, z: i32) -> Self {
        Self { x, z }
    }
    pub fn to_packed(&self) -> i64 {
        (self.x as i64 & 0xFFFF_FFFF) | ((self.z as i64) << 32)
    }
    pub fn from_packed(v: i64) -> Self {
        Self::new(v as i32, (v >> 32) as i32)
    }
    pub fn block_min(&self) -> BlockPos {
        BlockPos::new(self.x << 4, 0, self.z << 4)
    }
    pub fn distance_sq(&self, o: ChunkPos) -> i64 {
        let dx = (self.x - o.x) as i64;
        let dz = (self.z - o.z) as i64;
        dx * dx + dz * dz
    }
}

/// 16x16x16 section coordinates. `y` is in section units and may be negative (world min y -64 => -4).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub struct SectionPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}
impl SectionPos {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }
    pub fn chunk(&self) -> ChunkPos {
        ChunkPos::new(self.x, self.z)
    }
    pub fn block_min(&self) -> BlockPos {
        BlockPos::new(self.x << 4, self.y << 4, self.z << 4)
    }
}

/// Vanilla direction order (ordinal matters for packets and block models).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum Direction {
    Down = 0,
    Up = 1,
    North = 2,
    South = 3,
    West = 4,
    East = 5,
}
impl Direction {
    pub const ALL: [Direction; 6] = [
        Direction::Down,
        Direction::Up,
        Direction::North,
        Direction::South,
        Direction::West,
        Direction::East,
    ];
    pub fn offset(&self) -> IVec3 {
        match self {
            Direction::Down => IVec3::new(0, -1, 0),
            Direction::Up => IVec3::new(0, 1, 0),
            Direction::North => IVec3::new(0, 0, -1),
            Direction::South => IVec3::new(0, 0, 1),
            Direction::West => IVec3::new(-1, 0, 0),
            Direction::East => IVec3::new(1, 0, 0),
        }
    }
    pub fn opposite(&self) -> Direction {
        match self {
            Direction::Down => Direction::Up,
            Direction::Up => Direction::Down,
            Direction::North => Direction::South,
            Direction::South => Direction::North,
            Direction::West => Direction::East,
            Direction::East => Direction::West,
        }
    }
    pub fn from_ordinal(i: u8) -> Option<Direction> {
        Self::ALL.get(i as usize).copied()
    }
    /// `Direction.get3DDataValue` (the ordinal).
    pub fn to_3d_data_value(&self) -> i32 {
        *self as i32
    }
    /// `Direction.get2DDataValue`: NORTH=2, SOUTH=0, WEST=1, EAST=3; -1 for vertical.
    pub fn to_2d_data_value(&self) -> i32 {
        match self {
            Direction::South => 0,
            Direction::West => 1,
            Direction::North => 2,
            Direction::East => 3,
            Direction::Down | Direction::Up => -1,
        }
    }
    /// `Direction.toYRot`: the horizontal yaw in degrees (180 for NORTH, 0 SOUTH, 90 WEST,
    /// 270 EAST).
    pub fn to_y_rot(&self) -> f32 {
        (self.to_2d_data_value() & 3) as f32 * 90.0
    }
    /// `Direction.getCounterClockWise` for a horizontal direction.
    pub fn counter_clock_wise(&self) -> Direction {
        match self {
            Direction::North => Direction::West,
            Direction::South => Direction::East,
            Direction::West => Direction::South,
            Direction::East => Direction::North,
            Direction::Down | Direction::Up => *self,
        }
    }
    /// `Direction.Axis.isHorizontal`.
    pub fn is_horizontal(&self) -> bool {
        matches!(
            self,
            Direction::North | Direction::South | Direction::West | Direction::East
        )
    }
    /// `Direction.AxisDirection.getStep`: +1 for UP, -1 for DOWN, 0 for horizontal.
    pub fn axis_direction_step(&self) -> i32 {
        match self {
            Direction::Up => 1,
            Direction::Down => -1,
            _ => 0,
        }
    }
    pub fn step_x(&self) -> i32 {
        self.offset().x
    }
    pub fn step_y(&self) -> i32 {
        self.offset().y
    }
    pub fn step_z(&self) -> i32 {
        self.offset().z
    }
    pub fn name(&self) -> &'static str {
        match self {
            Direction::Down => "down",
            Direction::Up => "up",
            Direction::North => "north",
            Direction::South => "south",
            Direction::West => "west",
            Direction::East => "east",
        }
    }
}
