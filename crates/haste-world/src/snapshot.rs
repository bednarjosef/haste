//! Section snapshots for meshing threads: a section plus its six one-block border faces
//! and its light (moved out of `world.rs` as a pure move).

use haste_core::{BiomeId, BlockStateId};

use crate::light::{NibbleArray, SkyLight};
use crate::palette::{BiomeContainer, BlockContainer};

/// A section plus its six one-block border faces (from the six neighbours), light included.
/// Cloning the containers is a deep copy; the `PalettedContainer`'s palette and data are cloned.
#[derive(Clone)]
pub struct SnapshotSection {
    pub block_count: i16,
    /// Fluid count from the server's section header (0: no fluids to mesh).
    pub fluid_count: i16,
    pub blocks: BlockContainer,
    pub biomes: BiomeContainer,
}

/// Border faces of a snapshot, each 16x16 (or 16x16 for top/bottom), row-major in the loop
/// order used to gather them (see `World::snapshot_section`).
#[derive(Default, Clone)]
pub struct BorderCells {
    pub neg_x: Vec<BlockStateId>,
    pub pos_x: Vec<BlockStateId>,
    pub neg_y: Vec<BlockStateId>,
    pub pos_y: Vec<BlockStateId>,
    pub neg_z: Vec<BlockStateId>,
    pub pos_z: Vec<BlockStateId>,
}

/// What meshing threads get: a section, its border, its light. The sky light shares the
/// stored section's array (a refcount bump) or the stand-in built for an absent section.
pub struct SectionSnapshot {
    pub pos: haste_core::SectionPos,
    pub section: SnapshotSection,
    pub border: BorderCells,
    pub sky: SkyLight,
    pub block: NibbleArray,
}

impl SectionSnapshot {
    /// Block at (x, y, z) with x/y/z in -1..=16 (the section plus one-block border).
    pub fn get(&self, x: i8, y: i8, z: i8) -> BlockStateId {
        if (0..16).contains(&x) && (0..16).contains(&y) && (0..16).contains(&z) {
            return self.section.blocks.get_block(x as u8, y as u8, z as u8);
        }
        let (face, i) = self.border_face_index(x, y, z);
        match face {
            Some(f) => f[i],
            None => BlockStateId::AIR,
        }
    }

    /// Sky light at (x, y, z) in 0..16 coords. A section that arrived absent was filled
    /// from the layer above at snapshot time (see `snapshot_section`); `Absent` here means
    /// no layer anywhere above, which reads as full sky.
    pub fn sky_light(&self, x: u8, y: u8, z: u8) -> u8 {
        self.sky.value(x, y, z).unwrap_or(15)
    }

    pub fn block_light(&self, x: u8, y: u8, z: u8) -> u8 {
        self.block.get(x, y, z)
    }

    fn border_face_index(&self, x: i8, y: i8, z: i8) -> (Option<&Vec<BlockStateId>>, usize) {
        // The gather loops in `snapshot_section` define the index order:
        // neg_x: y*16+z ; pos_x: y*16+z ; neg_z: y*16+x ; pos_z: y*16+x ; neg_y: z*16+x ; pos_y: z*16+x
        let b = &self.border;
        if x == -1 {
            let i = (y as usize) * 16 + z as usize;
            return (Some(&b.neg_x), i);
        }
        if x == 16 {
            let i = (y as usize) * 16 + z as usize;
            return (Some(&b.pos_x), i);
        }
        if z == -1 {
            let i = (y as usize) * 16 + x as usize;
            return (Some(&b.neg_z), i);
        }
        if z == 16 {
            let i = (y as usize) * 16 + x as usize;
            return (Some(&b.pos_z), i);
        }
        if y == -1 {
            let i = (z as usize) * 16 + x as usize;
            return (Some(&b.neg_y), i);
        }
        if y == 16 {
            let i = (z as usize) * 16 + x as usize;
            return (Some(&b.pos_y), i);
        }
        (None, 0)
    }
}

/// Biome of the snapshot's section at cell coords (0..4).
pub fn snapshot_biome(s: &SectionSnapshot, x: u8, y: u8, z: u8) -> BiomeId {
    s.section.biomes.get_biome(x, y, z)
}
