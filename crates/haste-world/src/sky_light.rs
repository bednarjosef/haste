//! Sky light for a section with no stored array.
//!
//! Vanilla `SkyLightSectionStorage.getLightValue` flattens the queried position's y and,
//! while the section has no data layer, walks up the column section by section to the
//! nearest one that has a layer, reading its bottom row (local y = 0) at the same x, z;
//! only at or above the column's top data section is the value 15.
//!
//! light.rs deduplicates all-15 arrays into absent slots. A section deduplicated that way
//! still reads 15 through this walk: sky light 15 only propagates straight down, so a 15
//! anywhere in a column implies 15 in every cell above it, and the bottom row the walk
//! finds above an all-15 section is all 15 too.

use bytes::Bytes;

use crate::chunk::Chunk;
use crate::light::{nibble_set, NibbleArray};

/// The nearest sky array in a section above `idx`, if any.
fn nearest_above(chunk: &Chunk, idx: usize) -> Option<&NibbleArray> {
    chunk
        .light
        .sky
        .get(idx + 1..)?
        .iter()
        .find_map(|s| s.as_ref())
}

/// Sky light at local (x, z) of section `idx` of `chunk`, which has no array: the bottom
/// row of the nearest section above with one, or 15 when no section above has data.
pub(crate) fn absent_value(chunk: &Chunk, idx: usize, x: u8, z: u8) -> u8 {
    nearest_above(chunk, idx).map_or(15, |n| n.get(x, 0, z))
}

/// A whole-section array standing in for absent section `idx`: every y row repeats the
/// bottom row of the nearest section above with data (the walk flattens y, so all rows of
/// the absent section read the same value per column). None when no section above has
/// data (reads 15), or the row is all 15 (kept absent — same read, same dedup as light.rs).
pub(crate) fn absent_array(chunk: &Chunk, idx: usize) -> Option<NibbleArray> {
    let above = nearest_above(chunk, idx)?;
    let mut bytes = vec![0u8; 2048];
    for z in 0..16u8 {
        for x in 0..16u8 {
            let v = above.get(x, 0, z);
            for y in 0..16u8 {
                nibble_set(&mut bytes, x, y, z, v);
            }
        }
    }
    let array = NibbleArray::from_bytes(Bytes::from(bytes));
    if array.all_full() {
        None
    } else {
        Some(array)
    }
}
