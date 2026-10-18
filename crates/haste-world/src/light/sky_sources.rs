//! Per-chunk sky source heights, vanilla `ChunkSkyLightSources`: for each column, the
//! lowest y that still receives full skylight (everything above it is transparent), or
//! [`OPEN_TO_BOTTOM`] when the column is open down to the world's floor. A cell is a sky
//! light source (level 15) exactly when its y is at or above its column's value.
//!
//! The table is computed lazily on the first block change that needs it and then maintained
//! incrementally by [`update_column`] (vanilla `ChunkSkyLightSources.update`, run from
//! `LevelChunk.setBlockState` before the light engines check the block). An edge occludes
//! when the block below it has any light dampening; vanilla's shape-based variant
//! (`useShapeForLightOcclusion`, `Shapes.faceShapeOccludes`) is not modelled, so the block
//! above the edge never matters.

use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId, ChunkPos};
use haste_data::GameData;

use crate::chunk::Chunk;
use crate::world::World;

use super::storage::light_dampening;

/// The column is open to the world's floor: every y is a sky source
/// (vanilla `NEGATIVE_INFINITY`).
pub(super) const OPEN_TO_BOTTOM: i32 = i32::MIN;

/// Index of a column in the chunk's table.
#[inline]
fn index(x: u8, z: u8) -> usize {
    x as usize + z as usize * 16
}

/// Whether the state occludes skylight coming down onto it.
#[inline]
fn occludes(data: &GameData, state: BlockStateId) -> bool {
    light_dampening(data, state) != 0
}

/// One column's lowest source y, walked from the world top downwards
/// (vanilla `findLowestSourceY`): the y above the first occluding block, or
/// [`OPEN_TO_BOTTOM`] when the column never occludes.
fn column_lowest_source(data: &GameData, chunk: &Chunk, x: u8, z: u8) -> i32 {
    for si in (0..chunk.sections.len()).rev() {
        let section = &chunk.sections[si];
        if section.block_count == 0 {
            continue;
        }
        for y in (0..16u8).rev() {
            if occludes(data, section.get(x, y, z)) {
                return chunk.min_section * 16 + si as i32 * 16 + y as i32 + 1;
            }
        }
    }
    OPEN_TO_BOTTOM
}

/// Builds the chunk's table if it does not exist yet.
pub(crate) fn init(world: &mut World, pos: ChunkPos) {
    if world.chunk(pos).is_some_and(|c| c.sky_sources.is_some()) {
        return;
    }
    let data = world.game_data().clone();
    let Some(chunk) = world.chunk_mut(pos) else {
        return;
    };
    let mut table = Box::new([0i32; 256]);
    for z in 0..16u8 {
        for x in 0..16u8 {
            table[index(x, z)] = column_lowest_source(&data, chunk, x, z);
        }
    }
    chunk.sky_sources = Some(table);
}

/// The column's current lowest source y: the chunk's table when it exists (chunks that had
/// a light-relevant change maintain it), otherwise a fresh walk of the column.
/// Unloaded chunks read as open (vanilla's `Integer.MIN_VALUE` default).
pub(super) fn lowest_source_y(world: &World, x: i32, z: i32) -> i32 {
    let Some(chunk) = world.chunk(ChunkPos::new(x >> 4, z >> 4)) else {
        return OPEN_TO_BOTTOM;
    };
    let x = (x & 15) as u8;
    let z = (z & 15) as u8;
    match &chunk.sky_sources {
        Some(table) => table[index(x, z)],
        None => column_lowest_source(world.game_data(), chunk, x, z),
    }
}

/// Vanilla `ChunkSkyLightSources.update` for the column of a just-changed block: moves the
/// column's lowest source y when the change created or removed an occluding edge. The
/// world already holds the new state; the chunk's table must exist ([`init`] ran first).
pub(crate) fn update_column(world: &mut World, pos: BlockPos) {
    let i = index((pos.x & 15) as u8, (pos.z & 15) as u8);
    let Some(current) = world
        .chunk(pos.chunk())
        .and_then(|c| c.sky_sources.as_ref())
        .map(|t| t[i])
    else {
        return;
    };
    // a change strictly below the current lowest source cannot move it (vanilla's early out)
    if pos.y + 1 < current {
        return;
    }
    let data = world.game_data().clone();
    let mid = world.block_state(pos);
    let below = world.block_state(BlockPos::new(pos.x, pos.y - 1, pos.z));
    let mut new: Option<i32> = None;
    // edge between y+1 and y (vanilla tries this one first)
    if occludes(&data, mid) {
        if pos.y + 1 > current {
            new = Some(pos.y + 1);
        }
    } else if pos.y + 1 == current {
        new = Some(next_edge_below(world, pos));
    }
    // edge between y and y-1, when the first edge changed nothing
    if new.is_none() {
        if occludes(&data, below) {
            if pos.y > current {
                new = Some(pos.y);
            }
        } else if pos.y == current {
            new = Some(next_edge_below(
                world,
                BlockPos::new(pos.x, pos.y - 1, pos.z),
            ));
        }
    }
    if let Some(v) = new {
        if let Some(table) = world
            .chunk_mut(pos.chunk())
            .and_then(|c| c.sky_sources.as_mut())
        {
            table[i] = v;
        }
    }
}

/// Vanilla `findLowestSourceBelow`: walks down from `start` (a non-occluding block) to the
/// next occluding edge, or [`OPEN_TO_BOTTOM`] at the world's floor.
fn next_edge_below(world: &World, start: BlockPos) -> i32 {
    let floor = world.dimension().min_y - 1;
    let mut top = start;
    loop {
        let bottom = BlockPos::new(top.x, top.y - 1, top.z);
        if bottom.y < floor {
            return OPEN_TO_BOTTOM;
        }
        if occludes(world.game_data(), world.block_state(bottom)) {
            return top.y;
        }
        top = bottom;
    }
}
