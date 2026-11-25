//! Sky light for a section with no stored layer.
//!
//! Vanilla `SkyLightSectionStorage.getLightValue` flattens the queried position's y and,
//! while the section has no data layer, walks up the column section by section to the
//! nearest one that has a layer, reading its bottom row (local y = 0) at the same x, z;
//! only at or above the column's top data section is the value 15.
//!
//! light.rs stores uniform sections without an array. A section deduplicated that way
//! still reads 15 through this walk: sky light 15 only propagates straight down, so a 15
//! anywhere in a column implies 15 in every cell above it, and the bottom row the walk
//! finds above an all-15 section is all 15 too.

use crate::chunk::Chunk;
use crate::light::{nibble_set, NibbleArray, SkyLight};

/// Sky light at local (x, z) of the absent section `idx` of `chunk`: the bottom row of
/// the nearest section above with a layer, or 15 when no section above has one.
pub(crate) fn absent_value(chunk: &Chunk, idx: usize, x: u8, z: u8) -> u8 {
    chunk
        .light
        .sky
        .get(idx + 1..)
        .into_iter()
        .flatten()
        .find_map(|s| s.row_value(x, z))
        .unwrap_or(15)
}

/// The stand-in layer for absent section `idx` (what the mesher's snapshot captures):
/// every y row repeats the bottom row of the nearest section above with a layer (the
/// walk flattens y, so all rows of the absent section read the same value per column).
/// No layer above means full sky; a uniform row is one value with no array (same read,
/// no allocation).
pub(crate) fn absent_section_light(chunk: &Chunk, idx: usize) -> SkyLight {
    let Some(above) = chunk
        .light
        .sky
        .get(idx + 1..)
        .and_then(|sections| sections.iter().find(|s| s.is_storing()))
    else {
        return SkyLight::Absent;
    };
    let mut row = [0u8; 256];
    for z in 0..16u8 {
        for x in 0..16u8 {
            row[z as usize * 16 + x as usize] = above.row_value(x, z).unwrap_or(15);
        }
    }
    let first = row[0];
    if row.iter().all(|&v| v == first) {
        return SkyLight::Uniform(first);
    }
    let mut bytes = vec![0u8; 2048];
    for z in 0..16u8 {
        for x in 0..16u8 {
            let v = row[z as usize * 16 + x as usize];
            for y in 0..16u8 {
                nibble_set(&mut bytes, x, y, z, v);
            }
        }
    }
    SkyLight::Array(NibbleArray::from_vec(bytes))
}
