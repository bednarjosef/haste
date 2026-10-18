// This file is derived from Pomme (https://github.com/josefbednar/pomme),
// pomme-client/src/renderer/chunk/mesher.rs (snapshot capture), reused under
// GPL-3.0 and reworked for Haste. See THIRD_PARTY.md.

use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId, SectionPos};

use crate::vertex::pack_light;

/// A snapshot of the 3x3x3 section neighbourhood, captured on the render/game
/// thread and sent to the mesh workers. Cheap random access, no locks.
pub struct MeshSnapshot {
    /// Interleaved (state, sky, block) per cell, offsets -1..17 relative to the
    /// section origin: the section plus one-block borders into all neighbours
    /// (blocks for face culling and AO). Y is clamped at the world bounds (air
    /// outside). One array so a cell lookup touches one cache line.
    pub(super) cells: Vec<Cell>,
    /// Biome cells around the section (tint blending).
    pub(super) biomes: crate::tint::BiomeGrid,
    /// Section origin in world blocks.
    origin: BlockPos,
    /// World bounds for y clamping (kept for debug/documentation).
    #[allow(dead_code)]
    min_y: i32,
    #[allow(dead_code)]
    height: i32,
}

/// One captured cell: block state plus its two light nibbles.
#[derive(Clone, Copy)]
pub(super) struct Cell {
    pub(super) state: BlockStateId,
    /// (sky << 4) | block, pre-packed like the vertex format's light byte.
    pub(super) light: u8,
}

/// The section plus a one-block border (face culling, AO and smooth light only look one block
/// out): 18^3 = 5832 cells instead of the whole 3x3x3 neighbourhood.
pub(super) const EXT: i32 = 18;
const EXT2: i32 = EXT * EXT;
const EXT3: usize = (EXT2 * EXT) as usize;

impl MeshSnapshot {
    /// Capture the neighbourhood of `pos` from a `BlockView`. Reads
    /// 18^3 = 5832 block states + light (the section and a one-block border),
    /// amortized by the dirty-section pipeline.
    pub fn capture(view: &dyn BlockView, pos: SectionPos) -> MeshSnapshot {
        Self::capture_with_biomes(view, None, 0, pos)
    }

    /// `capture` plus the biome cells within `blend_radius` blocks of the section (for
    /// tints); without a biome source every block is biome 0.
    pub fn capture_with_biomes(
        view: &dyn BlockView,
        biome_source: Option<&dyn crate::tint::BiomeSource>,
        blend_radius: i32,
        pos: SectionPos,
    ) -> MeshSnapshot {
        let biomes = match biome_source {
            Some(src) => crate::tint::BiomeGrid::capture(src, pos.block_min(), blend_radius),
            None => crate::tint::BiomeGrid::uniform(haste_core::BiomeId(0)),
        };
        let origin = pos.block_min().add(-1, -1, -1);
        let mut cells = Vec::with_capacity(EXT3);
        for dy in 0..EXT {
            let y = origin.y + dy;
            let in_bounds = y >= view.min_y() && y < view.min_y() + view.height();
            for dz in 0..EXT {
                for dx in 0..EXT {
                    if in_bounds {
                        let p = BlockPos::new(origin.x + dx, y, origin.z + dz);
                        cells.push(Cell {
                            state: view.block_state(p),
                            light: pack_light(view.sky_light(p), view.block_light(p)),
                        });
                    } else {
                        cells.push(Cell {
                            state: BlockStateId::AIR,
                            light: pack_light(15, 0),
                        });
                    }
                }
            }
        }
        MeshSnapshot {
            cells,
            biomes,
            origin,
            min_y: view.min_y(),
            height: view.height(),
        }
    }

    #[inline]
    pub(super) fn cell(&self, dx: i32, dy: i32, dz: i32) -> Cell {
        debug_assert!((0..EXT).contains(&dx) && (0..EXT).contains(&dy) && (0..EXT).contains(&dz));
        self.cells[(dy * EXT2 + dz * EXT + dx) as usize]
    }

    /// Block state at world position; out-of-bounds y reads as air.
    pub fn block_state(&self, p: BlockPos) -> BlockStateId {
        let (dx, dy, dz) = (
            p.x - self.origin.x,
            p.y - self.origin.y,
            p.z - self.origin.z,
        );
        if (0..EXT).contains(&dx) && (0..EXT).contains(&dy) && (0..EXT).contains(&dz) {
            self.cell(dx, dy, dz).state
        } else {
            BlockStateId::AIR
        }
    }

    /// (sky, block) light at world position; 15/0 outside.
    pub fn light(&self, p: BlockPos) -> (u8, u8) {
        let (dx, dy, dz) = (
            p.x - self.origin.x,
            p.y - self.origin.y,
            p.z - self.origin.z,
        );
        if (0..EXT).contains(&dx) && (0..EXT).contains(&dy) && (0..EXT).contains(&dz) {
            let light = self.cell(dx, dy, dz).light;
            (light >> 4, light & 0xF)
        } else {
            (15, 0)
        }
    }

    #[allow(dead_code)]
    pub fn origin(&self) -> BlockPos {
        self.origin
    }
}
