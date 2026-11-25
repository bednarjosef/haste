//! Light storage, vanilla `DataLayer`'s layout: per section either no array at all —
//! uniform sky light is one value, absent sections read through the walk-up — or one
//! owned 2048-byte nibble array, sky and block separately.
//!
//! `level_chunk_with_light` / `light_update` semantics: bit sets index sections from
//! `min_section - 1` to `max_section + 1` (one extra section on each end). A section whose bit
//! is set in the (non-empty) mask receives the next array in order; a section set in the *empty*
//! mask is explicitly all-zero. Sky light is absent where the dimension has no skylight; reads
//! in an absent sky section go through `crate::sky_light` (the bottom row of the nearest section
//! above with data), 15 only above the column's top data section.
//!
//! The client-side light engine (vanilla `BlockLightEngine`/`SkyLightEngine`, run from
//! `World::set_block` because the server only sends `light_update` for border chunks) lives
//! in the child modules:
//!
//! - `engine` — the propagation queues (`LightEngine`)
//! - `storage` — the engine's view of the light arrays (`LayerLightSectionStorage`)
//! - `block_light` — the block layer's check (`BlockLightEngine.checkNode`)
//! - `sky_light_engine` — the sky layer's check and column sources (`SkyLightEngine`)
//! - `sky_sources` — per-column sky source heights (`ChunkSkyLightSources`)
//! - `update` — the per-block-change entry (`LevelLightEngine.checkBlock`+`runLightUpdates`)

mod block_light;
mod engine;
mod sky_light_engine;
mod sky_sources;
mod storage;
#[cfg(test)]
mod storage_tests;
mod update;

pub(crate) use update::{light_properties_differ, run_for_block};

use std::sync::{Arc, OnceLock};

use bytes::Bytes;
use haste_core::chunk::LightData;

use crate::chunk::Chunk;

/// The shared all-zero array (vanilla's `new DataLayer()`): a default `NibbleArray`
/// allocates nothing, and `is_empty` recognises it in O(1).
fn zero() -> Arc<[u8; 2048]> {
    static ZERO: OnceLock<Arc<[u8; 2048]>> = OnceLock::new();
    ZERO.get_or_init(|| Arc::new([0u8; 2048])).clone()
}

/// 4096 nibbles (4-bit values), one per block in a section (vanilla `DataLayer`'s array
/// state). A shared, always-owned 2048-byte array: wire bytes are copied in, so a stored
/// array never pins the whole chunk packet it arrived in, and cloning for a meshing
/// snapshot is a refcount bump.
#[derive(Clone)]
pub struct NibbleArray(Arc<[u8; 2048]>);

impl Default for NibbleArray {
    fn default() -> Self {
        Self(zero())
    }
}

impl NibbleArray {
    /// Copies a wire array into an owned section array. Wire arrays are at most 2048
    /// bytes (the reader caps them); a short one keeps reading 0 past its end.
    pub fn from_wire(bytes: &[u8]) -> Self {
        let mut data = [0u8; 2048];
        let n = bytes.len().min(2048);
        data[..n].copy_from_slice(&bytes[..n]);
        Self(Arc::new(data))
    }

    /// Takes an owned overlay (engine output) as the section's array.
    pub fn from_vec(v: Vec<u8>) -> Self {
        let data: [u8; 2048] = v
            .try_into()
            .unwrap_or_else(|_| panic!("light section is 2048 bytes"));
        Self(Arc::new(data))
    }

    /// The section's bytes (a copy; the engine edits overlays, tests build arrays).
    pub fn to_vec(&self) -> Vec<u8> {
        self.0[..].to_vec()
    }

    /// Vanilla's "no layer" marker: the shared zero array, never a real stored one
    /// (wire and engine all-zero arrays map to it), so this stays O(1).
    pub fn is_empty(&self) -> bool {
        Arc::ptr_eq(&self.0, &zero())
    }

    /// Nibble at flat index `y*256 + z*16 + x` (low nibble of byte i/2 when i is even).
    pub fn get(&self, x: u8, y: u8, z: u8) -> u8 {
        let i = (y as usize) * 256 + (z as usize) * 16 + x as usize;
        let b = self.0[i / 2];
        if i.is_multiple_of(2) {
            b & 0x0F
        } else {
            b >> 4
        }
    }

    pub fn all_zero(&self) -> bool {
        Self::all_bytes(&self.0[..], 0)
    }

    /// Every nibble is 15 (full light). Such an array can be stored as absent sky
    /// without changing any read: sky light 15 only propagates straight down, so the
    /// bottom row the absent-section walk (`crate::sky_light`) finds above it is all
    /// 15 too.
    pub fn all_full(&self) -> bool {
        Self::all_bytes(&self.0[..], 0xFF)
    }

    /// The one value every nibble holds, when the section is uniform.
    pub fn uniform_value(&self) -> Option<u8> {
        uniform_value(&self.0[..])
    }

    /// `data` is `fill` repeated (word-wide compare; a byte loop costs ~1 ns/byte on
    /// 2 KB arrays and would dominate chunk load).
    fn all_bytes(data: &[u8], fill: u8) -> bool {
        let word = u64::from_ne_bytes([fill; 8]);
        let (chunks, rem) = data.as_chunks::<8>();
        chunks.iter().all(|c| u64::from_ne_bytes(*c) == word) && rem.iter().all(|&b| b == fill)
    }
}

/// The one value every nibble of a packed light array holds, when they all match.
fn uniform_value(data: &[u8]) -> Option<u8> {
    let Some(&first) = data.first() else {
        return Some(0);
    };
    let word = u64::from_ne_bytes([first; 8]);
    let (chunks, rem) = data[1..].as_chunks::<8>();
    if !chunks.iter().all(|c| u64::from_ne_bytes(*c) == word) || !rem.iter().all(|&b| b == first) {
        return None;
    }
    let (lo, hi) = (first & 0x0F, first >> 4);
    (lo == hi).then_some(lo)
}

/// One section's sky light: vanilla `DataLayer`'s states plus the absent section.
/// A uniform section keeps one value and no array (vanilla's null data + default
/// value); a section without a layer is simply not stored (vanilla
/// `SkyLightSectionStorage` keeps no map entry for it) and reads resolve through the
/// walk-up in `crate::sky_light`. Only a non-uniform section holds the 2048-byte array.
#[derive(Clone, Default)]
pub enum SkyLight {
    /// No layer for the section: reads walk up to the nearest layer above
    /// (`crate::sky_light`), 15 when there is none.
    #[default]
    Absent,
    /// Every cell reads this value: a uniform wire array (all-15 arrays are stored
    /// `Absent` — the walk-up reads the same) or an explicitly-cleared section.
    Uniform(u8),
    /// A non-uniform section's array.
    Array(NibbleArray),
}

impl SkyLight {
    /// The section's stored value at a cell; `None` when the section has no layer (the
    /// caller walks up, `crate::sky_light::absent_value`).
    pub fn value(&self, x: u8, y: u8, z: u8) -> Option<u8> {
        match self {
            SkyLight::Absent => None,
            SkyLight::Uniform(v) => Some(*v),
            SkyLight::Array(a) => Some(a.get(x, y, z)),
        }
    }

    /// The value the section's bottom row reads at (x, z) — the walk-up's target — or
    /// `None` when the section has no layer.
    pub fn row_value(&self, x: u8, z: u8) -> Option<u8> {
        match self {
            SkyLight::Absent => None,
            SkyLight::Uniform(v) => Some(*v),
            SkyLight::Array(a) => Some(a.get(x, 0, z)),
        }
    }

    /// Whether the section has a layer (one half of vanilla's
    /// `storingLightForSection` test; holding blocks is the other half).
    pub fn is_storing(&self) -> bool {
        !matches!(self, SkyLight::Absent)
    }

    /// Classifies a packed array (wire bytes or an engine overlay): all-15 reads the
    /// same as `Absent`, a uniform array becomes one value with no array, anything
    /// else an owned copy of the bytes.
    pub fn from_packed(bytes: &[u8]) -> SkyLight {
        match uniform_value(bytes) {
            Some(15) => SkyLight::Absent,
            Some(v) => SkyLight::Uniform(v),
            None => SkyLight::Array(NibbleArray::from_wire(bytes)),
        }
    }
}

/// Nibble get/set over a plain byte array (engine overlays and tests).
pub fn nibble_get(arr: &[u8], x: u8, y: u8, z: u8) -> u8 {
    let i = (y as usize) * 256 + (z as usize) * 16 + x as usize;
    let b = arr[i / 2];
    if i.is_multiple_of(2) {
        b & 0x0F
    } else {
        b >> 4
    }
}

pub fn nibble_set(arr: &mut [u8], x: u8, y: u8, z: u8, v: u8) {
    let i = (y as usize) * 256 + (z as usize) * 16 + x as usize;
    let half = i / 2;
    let b = arr[half];
    arr[half] = if i.is_multiple_of(2) {
        (b & 0xF0) | (v & 0x0F)
    } else {
        (b & 0x0F) | ((v & 0x0F) << 4)
    };
}

/// Iterates set bits of a bit set (u64 words, bit i of word i/64).
pub fn set_bits(mask: &[u64]) -> impl Iterator<Item = usize> + '_ {
    mask.iter().enumerate().flat_map(|(w, &word)| {
        (0..64).filter_map(move |b| {
            if word >> b & 1 == 1 {
                Some(w * 64 + b)
            } else {
                None
            }
        })
    })
}

/// Applies `light` to `chunk`. Section indices in the masks run over
/// `min_section - 1 ..= min_section + len`, i.e. one extra on each end; the extras are ignored
/// for storage (we keep exactly one light entry per real section).
///
/// Uniform arrays are deduplicated (all-15 sky stored absent, other uniform sky as one
/// value, all-0 block light as the shared no-layer array) and the rest are copied out of
/// the packet — reads are unchanged, and a stored section costs its 2048 bytes and
/// nothing else, instead of pinning the whole chunk packet it arrived in.
pub(crate) fn apply_to_chunk(chunk: &mut Chunk, light: &LightData) {
    let n = chunk.sections.len();
    let sky_slots = &mut chunk.light.sky;
    let block_slots = &mut chunk.light.block;
    apply_side(
        &light.sky_mask,
        &light.empty_sky_mask,
        &light.sky_arrays,
        sky_slots,
        n,
    );
    apply_side_optional(
        &light.block_mask,
        &light.empty_block_mask,
        &light.block_arrays,
        block_slots,
        n,
    );
}

#[cfg(test)]
mod ab_tests {
    use super::*;
    use crate::chunk::ChunkLight;

    /// A/B: decoding the same chunk with and without the uniform-light dedup must produce
    /// identical reads (sky 15 / block 0 where the array was uniform), and the dedup must
    /// actually drop the memory.
    #[test]
    fn uniform_light_drops_storage_without_changing_reads() {
        use crate::chunk::Section;
        let mk = |sky_fill: u8, block_fill: u8| LightData {
            sky_mask: vec![0b10], // wire index 1 = the first real section
            block_mask: vec![0b10],
            empty_sky_mask: vec![0],
            empty_block_mask: vec![0],
            sky_arrays: vec![bytes::Bytes::from(vec![sky_fill; 2048])],
            block_arrays: vec![bytes::Bytes::from(vec![block_fill; 2048])],
        };
        let mut chunk = Chunk {
            pos: haste_core::ChunkPos::new(0, 0),
            min_section: -4,
            sections: vec![Section {
                block_count: 0,
                fluid_count: 0,
                blocks: crate::palette::BlockContainer::single(0),
                biomes: crate::palette::BiomeContainer::single_biome(0),
            }],
            heightmaps: vec![],
            block_entities: Default::default(),
            light: ChunkLight::new(1),
            sky_sources: None,
            dirty: vec![0; 1],
        };
        // uniform full sky: stored absent (an absent sky section reads 15, same as the
        // array); uniform zero block light: the shared no-layer array (reads 0)
        chunk.apply_light(&mk(0xFF, 0x00));
        assert!(matches!(chunk.light.sky[0], SkyLight::Absent));
        assert!(chunk.light.block[0].all_zero());
        assert!(chunk.light.block[0].is_empty());
        // a uniform value that is neither: one byte, no array
        chunk.apply_light(&mk(0x55, 0x00));
        assert!(matches!(chunk.light.sky[0], SkyLight::Uniform(5)));
        // non-uniform arrays are kept (an owned copy, not the packet's bytes)
        chunk.apply_light(&mk(0x12, 0x34));
        assert!(matches!(chunk.light.sky[0], SkyLight::Array(_)));
        assert_eq!(chunk.light.sky[0].value(0, 0, 0), Some(0x2));
        assert!(!chunk.light.block[0].all_zero());
    }
}

fn apply_side(
    mask: &[u64],
    empty_mask: &[u64],
    arrays: &[Bytes],
    slots: &mut [SkyLight],
    n_sections: usize,
) {
    // Wire indices cover -1..=n (n+2 slots); our slots are 0..n (shift by one).
    let wire_len = n_sections + 2;
    let set = |i: usize, bit: usize| -> bool { i < mask.len() && (mask[i] >> bit) & 1 == 1 };
    let empty =
        |i: usize, bit: usize| -> bool { i < empty_mask.len() && (empty_mask[i] >> bit) & 1 == 1 };
    let mut array_iter = arrays.iter();
    for wire_idx in 0..wire_len {
        let our_idx = wire_idx as i64 - 1; // -1 or n_sections are outside our storage
        let m = set(wire_idx / 64, wire_idx % 64);
        let e = empty(wire_idx / 64, wire_idx % 64);
        if m {
            if let Some(bytes) = array_iter.next() {
                if our_idx >= 0 && (our_idx as usize) < n_sections {
                    // uniform arrays (all-15 daylight above ground, or any other
                    // uniform value) keep no array, and the rest are copied out of the
                    // packet: a stored section costs its 2048 bytes and nothing else
                    slots[our_idx as usize] = SkyLight::from_packed(bytes);
                }
            }
        } else if e && our_idx >= 0 && (our_idx as usize) < n_sections {
            // Explicitly cleared: all-zero light. Vanilla stores `new DataLayer()`
            // (default 0) for an empty-marked section (ClientPacketListener
            // .readSectionList), so sky reads 0 — an absent section instead reads the
            // layer above (`crate::sky_light`).
            slots[our_idx as usize] = SkyLight::Uniform(0);
        }
        // neither set nor empty: section keeps its current light (light_update partial semantics)
    }
}

/// Same as `apply_side` but for plain (non-optional) slots: block light defaults to zero
/// arrays, so "explicitly cleared" also means the zero array.
fn apply_side_optional(
    mask: &[u64],
    empty_mask: &[u64],
    arrays: &[Bytes],
    slots: &mut [NibbleArray],
    n_sections: usize,
) {
    let wire_len = n_sections + 2;
    let set = |i: usize, bit: usize| -> bool { i < mask.len() && (mask[i] >> bit) & 1 == 1 };
    let empty =
        |i: usize, bit: usize| -> bool { i < empty_mask.len() && (empty_mask[i] >> bit) & 1 == 1 };
    let mut array_iter = arrays.iter();
    for wire_idx in 0..wire_len {
        let our_idx = wire_idx as i64 - 1;
        let m = set(wire_idx / 64, wire_idx % 64);
        let e = empty(wire_idx / 64, wire_idx % 64);
        if m {
            if let Some(bytes) = array_iter.next() {
                if our_idx >= 0 && (our_idx as usize) < n_sections {
                    // all-0 block light reads the same as the default empty array (0):
                    // skip storing it (dark sections below ground cost no memory)
                    slots[our_idx as usize] = if uniform_value(bytes) == Some(0) {
                        NibbleArray::default()
                    } else {
                        NibbleArray::from_wire(bytes)
                    };
                }
            }
        } else if e && our_idx >= 0 && (our_idx as usize) < n_sections {
            slots[our_idx as usize] = NibbleArray::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nibble_get_set() {
        let mut arr = vec![0u8; 2048];
        nibble_set(&mut arr, 0, 0, 0, 5);
        nibble_set(&mut arr, 1, 0, 0, 9);
        nibble_set(&mut arr, 15, 15, 15, 7);
        let n = NibbleArray::from_vec(arr);
        assert_eq!(n.get(0, 0, 0), 5);
        assert_eq!(n.get(1, 0, 0), 9);
        assert_eq!(n.get(15, 15, 15), 7);
        assert_eq!(n.get(2, 0, 0), 0);
        assert!(!n.all_zero());
        let z = NibbleArray::from_vec(vec![0u8; 2048]);
        assert!(z.all_zero());
    }

    #[test]
    fn uniform_value_detects_uniform_sections() {
        let u = |fill: u8| NibbleArray::from_vec(vec![fill; 2048]);
        assert_eq!(u(0x00).uniform_value(), Some(0));
        assert_eq!(u(0x55).uniform_value(), Some(5));
        assert_eq!(u(0xFF).uniform_value(), Some(15));
        // nibble positions differ: not uniform
        assert_eq!(u(0x12).uniform_value(), None);
        // mixed bytes: not uniform
        let mut mixed = vec![0x55; 2048];
        mixed[2047] = 0x00;
        assert_eq!(NibbleArray::from_vec(mixed).uniform_value(), None);
        // the shared default array is uniform zero
        assert_eq!(NibbleArray::default().uniform_value(), Some(0));
    }

    #[test]
    fn set_bits_iterates() {
        let mask = [0b1011u64, 1 << 63];
        let bits: Vec<usize> = set_bits(&mask).collect();
        assert_eq!(bits, vec![0, 1, 3, 127]);
    }
}
