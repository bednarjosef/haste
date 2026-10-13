//! Light storage: one nibble array (2048 bytes) per section, sky and block separately.
//!
//! `level_chunk_with_light` / `light_update` semantics: bit sets index sections from
//! `min_section - 1` to `max_section + 1` (one extra section on each end). A section whose bit
//! is set in the (non-empty) mask receives the next array in order; a section set in the *empty*
//! mask is explicitly all-zero. Sky light is absent where the dimension has no skylight; absent
//! reads return 15 for sky (per `BlockView`).

use bytes::Bytes;
use haste_core::chunk::LightData;

use crate::chunk::Chunk;

/// 4096 nibbles (4-bit values), one per block in a section. Immutable wire bytes; `set` is
/// only used in tests, where the array is materialized.
#[derive(Clone, Default)]
pub struct NibbleArray(pub Bytes);

impl NibbleArray {
    pub fn from_bytes(b: Bytes) -> Self {
        Self(b)
    }

    /// An owned, writable array (2048 zero bytes).
    pub fn owned() -> Vec<u8> {
        vec![0u8; 2048]
    }

    /// Nibble at flat index `y*256 + z*16 + x` (low nibble of byte i/2 when i is even).
    /// A default-constructed (empty) array reads as 0.
    pub fn get(&self, x: u8, y: u8, z: u8) -> u8 {
        let i = (y as usize) * 256 + (z as usize) * 16 + x as usize;
        let Some(&b) = self.0.get(i / 2) else {
            return 0;
        };
        if i.is_multiple_of(2) {
            b & 0x0F
        } else {
            b >> 4
        }
    }

    pub fn all_zero(&self) -> bool {
        Self::all_bytes(&self.0, 0)
    }

    /// Every nibble is 15 (full light). Absent sky light reads as 15, so such an array can
    /// be stored as `None` without changing any read.
    pub fn all_full(&self) -> bool {
        Self::all_bytes(&self.0, 0xFF)
    }

    /// `data` is `fill` repeated (word-wide compare; a byte loop costs ~1 ns/byte on
    /// 2 KB arrays and would dominate chunk load).
    fn all_bytes(data: &[u8], fill: u8) -> bool {
        let word = u64::from_ne_bytes([fill; 8]);
        let (chunks, rem) = data.as_chunks::<8>();
        chunks.iter().all(|c| u64::from_ne_bytes(*c) == word) && rem.iter().all(|&b| b == fill)
    }
}

/// Nibble get/set over a plain byte array (tests; wire arrays are immutable `Bytes`).
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
/// Uniform arrays are deduplicated (all-15 sky stored as absent, all-0 block light stored
/// empty) — reads are unchanged, and the ~1 us/chunk scan buys ~45 KB/chunk: above-ground
/// daylight sections stop pinning the packet's bytes.
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
        };
        // uniform full sky: stored as None (absent sky reads 15, same as the array)
        // uniform zero block light: stored empty (absent block light reads 0)
        chunk.apply_light(&mk(0xFF, 0x00));
        assert!(chunk.light.sky[0].is_none());
        assert!(chunk.light.block[0].all_zero());
        assert!(chunk.light.block[0].0.is_empty());
        // non-uniform arrays are kept
        chunk.apply_light(&mk(0x12, 0x34));
        assert!(chunk.light.sky[0].is_some());
        assert_eq!(chunk.light.sky[0].as_ref().unwrap().get(0, 0, 0), 0x2);
        assert!(!chunk.light.block[0].all_zero());
    }
}

fn apply_side(
    mask: &[u64],
    empty_mask: &[u64],
    arrays: &[Bytes],
    slots: &mut [Option<NibbleArray>],
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
                    // all-15 sky reads the same as absent sky (15): skip storing it, so
                    // uniform-daylight sections (everything above ground) cost no memory
                    // and stop pinning the packet's bytes
                    let n = NibbleArray::from_bytes(bytes.clone());
                    slots[our_idx as usize] = if n.all_full() { None } else { Some(n) };
                }
            }
        } else if e && our_idx >= 0 && (our_idx as usize) < n_sections {
            slots[our_idx as usize] = None; // explicitly cleared
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
                    let n = NibbleArray::from_bytes(bytes.clone());
                    slots[our_idx as usize] = if n.all_zero() {
                        NibbleArray::default()
                    } else {
                        n
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
        let mut arr = NibbleArray::owned();
        nibble_set(&mut arr, 0, 0, 0, 5);
        nibble_set(&mut arr, 1, 0, 0, 9);
        nibble_set(&mut arr, 15, 15, 15, 7);
        let n = NibbleArray::from_bytes(Bytes::from(arr));
        assert_eq!(n.get(0, 0, 0), 5);
        assert_eq!(n.get(1, 0, 0), 9);
        assert_eq!(n.get(15, 15, 15), 7);
        assert_eq!(n.get(2, 0, 0), 0);
        assert!(!n.all_zero());
        let z = NibbleArray::from_bytes(Bytes::from(vec![0u8; 2048]));
        assert!(z.all_zero());
    }

    #[test]
    fn set_bits_iterates() {
        let mask = [0b1011u64, 1 << 63];
        let bits: Vec<usize> = set_bits(&mask).collect();
        assert_eq!(bits, vec![0, 1, 3, 127]);
    }
}
