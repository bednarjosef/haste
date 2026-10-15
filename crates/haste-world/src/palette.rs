//! Vanilla's paletted container wire format (since 1.16: entries never span longs).
//!
//! Layout on the wire:
//! - `bits_per_entry = 0`: single-value palette, no data array follows (removed in 1.21.5).
//! - indirect (blocks 4..=8, biomes 1..=3): palette length varint, then palette entries (varint
//!   global ids), then the packed data longs. Since 1.21.5 there is NO data-array length prefix
//!   and the array length is implicit from bits-per-entry (ceil(4096 * bpe / 64) longs).
//! - direct (blocks > 8): no palette, entries are raw global ids packed at
//!   `ceil(log2(global_count))` bits. Palette length is implicit (0) since 1.21.5.
//!
//! Verified against the wiki's Chunk Format page (1.21.5+ section layout) and the 26.3
//! protocol docs (see NOTES.md for what was checked).

use std::sync::atomic::AtomicBool;

use haste_core::BlockStateId;

/// Number of entries in a block section container (16^3) and a biome container (4^3 per section).
pub const BLOCKS_PER_SECTION: usize = 4096;
pub const BIOMES_PER_SECTION: usize = 64;

/// Errors from decoding/encoding paletted containers.
#[derive(thiserror::Error, Debug)]
pub enum PaletteError {
    #[error("unexpected end of section payload")]
    UnexpectedEof,
    #[error("bits per entry {0} out of range (0..=16)")]
    BadBits(u8),
    #[error("palette index {idx} out of range (palette len {len})")]
    PaletteIndexOutOfRange { idx: usize, len: usize },
    #[error("cannot encode global id {0}: exceeds direct bits")]
    IdTooLarge(u32),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Reader over the wire payload.
pub struct WireReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> WireReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Vanilla varint: up to 5 bytes, little-endian 7-bit groups.
    pub fn varint(&mut self) -> Result<u32, PaletteError> {
        let mut value: u32 = 0;
        for i in 0..5 {
            let b = *self.data.get(self.pos).ok_or(PaletteError::UnexpectedEof)?;
            self.pos += 1;
            value |= ((b & 0x7F) as u32) << (i * 7);
            if b & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(PaletteError::BadBits(0)) // too many continuation bits
    }

    /// A big-endian i16 (the section header's block and fluid counts).
    pub fn i16_be(&mut self) -> Result<i16, PaletteError> {
        let b = self
            .data
            .get(self.pos..self.pos + 2)
            .ok_or(PaletteError::UnexpectedEof)?;
        self.pos += 2;
        Ok(i16::from_be_bytes([b[0], b[1]]))
    }

    /// A single i64 in big-endian (packed data longs are BE).
    pub fn i64_be(&mut self) -> Result<i64, PaletteError> {
        let b = self
            .data
            .get(self.pos..self.pos + 8)
            .ok_or(PaletteError::UnexpectedEof)?;
        self.pos += 8;
        Ok(i64::from_be_bytes(b.try_into().unwrap()))
    }

    /// Takes `n` bytes in one bounds check (bulk reads: packed data arrays).
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], PaletteError> {
        let end = self.pos.checked_add(n).ok_or(PaletteError::UnexpectedEof)?;
        let s = self
            .data
            .get(self.pos..end)
            .ok_or(PaletteError::UnexpectedEof)?;
        self.pos = end;
        Ok(s)
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
}

/// Writer producing the same wire format (for tests and round trips).
pub struct WireWriter {
    pub data: Vec<u8>,
}

impl Default for WireWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl WireWriter {
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    pub fn varint(&mut self, mut v: u32) {
        loop {
            let b = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                self.data.push(b);
                break;
            }
            self.data.push(b | 0x80);
        }
    }

    pub fn i64_be(&mut self, v: i64) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }

    pub fn i16_be(&mut self, v: i16) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
}

/// Packed long array where entries never span longs (1.16+ layout): each 64-bit long holds
/// `floor(64 / bits)` entries, entry 0 in the least significant bits (vanilla's layout; the
/// unused padding is the high bits of each long).
#[derive(Debug, Clone)]
pub struct PackedArray {
    bits: u8,
    longs: Vec<i64>,
}

impl PackedArray {
    /// Number of longs needed for `entries` entries at `bits` per entry.
    pub fn long_count(bits: u8, entries: usize) -> usize {
        let per_long = 64 / bits as usize;
        entries.div_ceil(per_long)
    }

    /// A zeroed array for `entries` entries at `bits` per entry (tests and benches build
    /// wire payloads with it).
    pub fn zeroed(bits: u8, entries: usize) -> Self {
        Self {
            bits,
            longs: vec![0; Self::long_count(bits, entries)],
        }
    }

    /// Bit width per entry.
    pub fn bits(&self) -> u8 {
        self.bits
    }

    /// The packed longs (big-endian on the wire; entry 0 in the low bits of each long).
    pub fn longs(&self) -> &[i64] {
        &self.longs
    }

    /// Heap bytes of the packed data (the longs' live allocation).
    pub fn memory_bytes(&self) -> u64 {
        self.longs.capacity() as u64 * 8
    }

    pub fn read_at(&self, i: usize) -> u32 {
        // Specialize the common widths: a match on constant bits lets the compiler use
        // shifts/masks instead of runtime div/mod.
        match self.bits {
            4 => Self::read_at_bits::<4>(i, &self.longs),
            5 => Self::read_at_bits::<5>(i, &self.longs),
            6 => Self::read_at_bits::<6>(i, &self.longs),
            7 => Self::read_at_bits::<7>(i, &self.longs),
            8 => Self::read_at_bits::<8>(i, &self.longs),
            _ => {
                let bits = self.bits as usize;
                let per_long = 64 / bits;
                let long = i / per_long;
                let off = (i % per_long) * bits;
                ((self.longs[long] as u64 >> off) as u32) & ((1u32 << bits) - 1)
            }
        }
    }

    fn read_at_bits<const B: u8>(i: usize, longs: &[i64]) -> u32 {
        let per_long = 64 / B as usize;
        let long = i / per_long;
        let off = (i % per_long) * B as usize;
        let v = ((longs[long] as u64) >> off) as u32;
        v & ((1u32 << B) - 1)
    }

    pub fn write_at(&mut self, i: usize, v: u32) {
        let bits = self.bits as usize;
        let per_long = 64 / bits;
        let long = i / per_long;
        let off = (i % per_long) * bits;
        let mask = ((1u64 << bits) - 1) << off;
        let cleared = (self.longs[long] as u64 & !mask) as i64;
        let v64 = ((v as u64) & ((1u64 << bits) - 1)) << off;
        self.longs[long] = (cleared as u64 | v64) as i64;
    }
}

/// Storage strategy of a decoded container.
#[derive(Debug, Clone)]
pub enum Storage {
    /// bits_per_entry 0: every entry is this value. No data array on the wire.
    Single(u32),
    /// Indirect: palette of global ids, data are palette indices.
    Indirect {
        palette: Vec<u32>,
        data: PackedArray,
    },
    /// Direct: data are raw global ids at direct bits.
    Direct(PackedArray),
}

impl Storage {
    /// Heap bytes of the stored data (palette plus packed entries; live allocations).
    pub fn memory_bytes(&self) -> u64 {
        match self {
            Storage::Single(_) => 0,
            Storage::Indirect { palette, data } => {
                palette.capacity() as u64 * 4 + data.memory_bytes()
            }
            Storage::Direct(data) => data.memory_bytes(),
        }
    }
}

/// Conversion between `u32` wire ids and the typed id. Implemented for `BlockStateId` and
/// `BiomeId` here (haste-core keeps its ids conversion-free by design).
pub trait WireId: Copy {
    fn from_u32(v: u32) -> Self;
    fn to_u32(self) -> u32;
}

impl WireId for BlockStateId {
    fn from_u32(v: u32) -> Self {
        BlockStateId(v)
    }
    fn to_u32(self) -> u32 {
        self.0
    }
}

impl WireId for haste_core::BiomeId {
    fn from_u32(v: u32) -> Self {
        haste_core::BiomeId(v)
    }
    fn to_u32(self) -> u32 {
        self.0
    }
}

/// A vanilla paletted container (blocks or biomes), generic over the id type.
#[derive(Clone)]
pub struct PalettedContainer<T> {
    pub storage: Storage,
    /// Capacity (entries): 4096 for block containers, 64 for biome containers.
    capacity: usize,
    _marker: std::marker::PhantomData<T>,
}

impl<T: WireId> PalettedContainer<T> {
    /// Heap bytes of the container's storage (see [`Storage::memory_bytes`]).
    pub fn memory_bytes(&self) -> u64 {
        self.storage.memory_bytes()
    }

    /// Single-value container (every entry `v`). Block capacity.
    pub fn single(v: u32) -> Self {
        Self {
            storage: Storage::Single(v),
            capacity: BLOCKS_PER_SECTION,
            _marker: std::marker::PhantomData,
        }
    }

    /// Single-value biome container (every cell `v`).
    pub fn single_biome(v: u32) -> Self {
        Self {
            storage: Storage::Single(v),
            capacity: BIOMES_PER_SECTION,
            _marker: std::marker::PhantomData,
        }
    }

    /// Indirect container from a palette and per-entry global ids (used by the test encoder).
    /// A palette with a single entry is a single-value container: it is stored as
    /// `Storage::Single`, without a data array, exactly as vanilla's zero-bits
    /// configuration does (`Strategy::ZERO_BITS`).
    pub fn indirect(palette: Vec<u32>, values: &[u32]) -> Self {
        if palette.len() == 1 && values.iter().all(|&v| v == palette[0]) {
            return Self {
                storage: Storage::Single(palette[0]),
                capacity: values.len(),
                _marker: std::marker::PhantomData,
            };
        }
        let bits = bits_for(palette.len());
        let mut data = PackedArray {
            bits,
            longs: vec![0; PackedArray::long_count(bits, values.len())],
        };
        for (i, &v) in values.iter().enumerate() {
            let idx = palette.iter().position(|&p| p == v).unwrap_or(0) as u32;
            data.write_at(i, idx);
        }
        Self {
            storage: Storage::Indirect { palette, data },
            capacity: values.len(),
            _marker: std::marker::PhantomData,
        }
    }

    /// Decodes a container from the wire. `direct_bits` is the bit width used when the wire's
    /// bits-per-entry indicates direct storage (blocks: ceil(log2(global block state count));
    /// biomes have no direct format in practice, servers cap at 3 bits).
    pub fn decode(
        r: &mut WireReader<'_>,
        capacity: usize,
        direct_bits: u8,
    ) -> Result<Self, PaletteError> {
        let bpe = r.varint()? as u8;
        Self::decode_with_bpe(r, bpe, capacity, direct_bits)
    }

    fn decode_with_bpe(
        r: &mut WireReader<'_>,
        bpe: u8,
        capacity: usize,
        direct_bits: u8,
    ) -> Result<Self, PaletteError> {
        let storage = match bpe {
            0 => {
                let v = r.varint()?;
                Storage::Single(v)
            }
            // Indirect up to 8 bits for blocks and 3 for biomes (minecraft.wiki "Chunk Format",
            // paletted container); anything wider is direct at `direct_bits`. Confirmed against
            // the 26.3 server: biome containers above 3 bits are direct at
            // ceil(log2(biome registry size)).
            b if b as usize <= Self::max_indirect_bits(capacity) => {
                // indirect: palette + packed data; no length prefix (1.21.5+)
                let palette_len = r.varint()? as usize;
                let mut palette = Vec::with_capacity(palette_len.min(1024));
                for _ in 0..palette_len {
                    palette.push(r.varint()?);
                }
                let data = Self::read_packed(r, b, capacity)?;
                if palette.is_empty() {
                    return Err(PaletteError::PaletteIndexOutOfRange { idx: 0, len: 0 });
                }
                Storage::Indirect { palette, data }
            }
            _ => {
                // direct: raw global ids at direct_bits (per 1.21.5+, bits-per-entry on the wire
                // is the direct bit width itself; no palette length follows)
                let data = Self::read_packed(r, direct_bits, capacity)?;
                Storage::Direct(data)
            }
        };
        Ok(Self {
            storage,
            capacity,
            _marker: std::marker::PhantomData,
        })
    }

    /// Widest indirect (palette) encoding for a container of `capacity` entries.
    fn max_indirect_bits(capacity: usize) -> usize {
        if capacity == BIOMES_PER_SECTION {
            3
        } else {
            8
        }
    }

    fn read_packed(
        r: &mut WireReader<'_>,
        bits: u8,
        capacity: usize,
    ) -> Result<PackedArray, PaletteError> {
        if bits == 0 || bits > 32 {
            return Err(PaletteError::BadBits(bits));
        }
        let longs = PackedArray::long_count(bits, capacity);
        // one bounds check for the whole array, then a bulk big-endian conversion
        // (a per-long `i64_be` costs a slice `get` + try_into per 8 bytes)
        let bytes = r.take(longs * 8)?;
        let mut v = Vec::with_capacity(longs);
        for c in bytes.as_chunks::<8>().0 {
            v.push(i64::from_be_bytes(*c));
        }
        Ok(PackedArray { bits, longs: v })
    }

    /// Global id at flat index `i` (y*256 + z*16 + x). No allocation.
    pub fn get_index(&self, i: usize) -> u32 {
        match &self.storage {
            Storage::Single(v) => *v,
            Storage::Indirect { palette, data } => {
                let idx = data.read_at(i) as usize;
                palette.get(idx).copied().unwrap_or(0)
            }
            Storage::Direct(data) => data.read_at(i),
        }
    }

    /// Sets the global id at flat index `i`, promoting the palette when needed. Returns the old
    /// id. Panics if `i >= capacity` or the direct bit width cannot hold `v`.
    pub fn set_index(&mut self, i: usize, v: u32, direct_bits: u8) -> u32 {
        let old = self.get_index(i);
        if old == v {
            return old;
        }
        match &mut self.storage {
            Storage::Single(cur) => {
                if *cur == v {
                    return old;
                }
                // promote: single -> indirect with a 2-entry palette (4 bits min for blocks)
                let mut palette = vec![*cur, v];
                let bits = Self::indirect_bits(palette.len());
                let mut data = PackedArray {
                    bits,
                    longs: vec![0; PackedArray::long_count(bits, self.capacity)],
                };
                for j in 0..self.capacity {
                    // only indices == i change; all others are `cur`
                    let val = if j == i { v } else { *cur };
                    let idx = palette.iter().position(|&p| p == val).unwrap_or(0) as u32;
                    data.write_at(j, idx);
                }
                let _ = palette.pop(); // keep vec arithmetic obvious; palette stays [cur, v]
                let _ = &mut palette;
                self.storage = Storage::Indirect {
                    palette: vec![*cur, v],
                    data,
                };
            }
            Storage::Indirect { palette, data } => {
                if let Some(pos) = palette.iter().position(|&p| p == v) {
                    data.write_at(i, pos as u32);
                } else {
                    // grow palette; promote to direct if it would exceed the indirect max
                    if palette.len() + 1 > (1usize << data.bits)
                        || palette.len() + 1 > Self::max_indirect_palette()
                    {
                        let direct = self.to_direct(direct_bits);
                        self.storage = Storage::Direct(direct);
                        return self.set_index(i, v, direct_bits);
                    }
                    palette.push(v);
                    data.write_at(i, (palette.len() - 1) as u32);
                }
            }
            Storage::Direct(data) => {
                if v >= (1u32 << data.bits) {
                    // need wider direct bits: rebuild
                    let new_bits = (u32::BITS - v.leading_zeros()).max(1) as u8;
                    let mut nd = PackedArray {
                        bits: new_bits,
                        longs: vec![0; PackedArray::long_count(new_bits, self.capacity)],
                    };
                    for j in 0..self.capacity {
                        nd.write_at(j, self.get_index(j));
                    }
                    self.storage = Storage::Direct(nd);
                    return self.set_index(i, v, direct_bits);
                }
                data.write_at(i, v);
            }
        }
        old
    }

    fn indirect_bits(palette_len: usize) -> u8 {
        bits_for(palette_len)
    }

    fn max_indirect_palette() -> usize {
        256 // 8 bits max for blocks
    }

    fn to_direct(&self, direct_bits: u8) -> PackedArray {
        let mut data = PackedArray {
            bits: direct_bits,
            longs: vec![0; PackedArray::long_count(direct_bits, self.capacity)],
        };
        for j in 0..self.capacity {
            data.write_at(j, self.get_index(j));
        }
        data
    }

    /// Encodes the container to the wire format (for tests / round trips; mirrors decode).
    pub fn encode(&self, w: &mut WireWriter, direct_bits: u8) {
        match &self.storage {
            Storage::Single(v) => {
                w.varint(0);
                w.varint(*v);
            }
            Storage::Indirect { palette, data } => {
                w.varint(data.bits as u32);
                w.varint(palette.len() as u32);
                for &p in palette {
                    w.varint(p);
                }
                for &l in &data.longs {
                    w.i64_be(l);
                }
            }
            Storage::Direct(data) => {
                w.varint(direct_bits as u32);
                for &l in &data.longs {
                    w.i64_be(l);
                }
            }
        }
    }

    /// Decodes an entire section payload (the 26.3 level_chunk format) into per-section block
    /// and biome containers. Each section is: block count (i16 BE), fluid count (i16 BE),
    /// block container, biome container (verified against the 26.3 server). `direct_bits` is
    /// the block direct width, `biome_bits` the biome direct width
    /// (ceil(log2(synced worldgen/biome registry size))). The containers share the container
    /// type `T`; `Chunk::decode` re-wraps the biome container with `BiomeId`.
    pub fn decode_sections(
        bytes: &[u8],
        section_count: usize,
        direct_bits: u8,
        biome_bits: u8,
    ) -> Result<Vec<DecodedSection<T>>, PaletteError> {
        let mut r = WireReader::new(bytes);
        let mut out = Vec::with_capacity(section_count);
        for _ in 0..section_count {
            let block_count = r.i16_be()?;
            let fluid_count = r.i16_be()?;
            let blocks = Self::decode(&mut r, BLOCKS_PER_SECTION, direct_bits)?;
            let biomes = Self::decode(&mut r, BIOMES_PER_SECTION, biome_bits)?;
            out.push(DecodedSection {
                block_count,
                fluid_count,
                blocks,
                biomes,
            });
        }
        if r.remaining() != 0 {
            // trailing garbage: keep it loud in tests, tolerate in production
            tracing::debug!("decode_sections: {} trailing bytes", r.remaining());
        }
        Ok(out)
    }

    /// Reinterprets a container decoded as `T2` as holding `T` (same layout, no copy of data;
    /// used to give the biome container its `BiomeId` type after decoding both as u32 ids).
    pub fn remap<T2: WireId>(self) -> PalettedContainer<T2> {
        match self.storage {
            Storage::Single(v) => PalettedContainer {
                storage: Storage::Single(v),
                capacity: self.capacity,
                _marker: std::marker::PhantomData,
            },
            Storage::Indirect { palette, data } => PalettedContainer {
                storage: Storage::Indirect { palette, data },
                capacity: self.capacity,
                _marker: std::marker::PhantomData,
            },
            Storage::Direct(data) => PalettedContainer {
                storage: Storage::Direct(data),
                capacity: self.capacity,
                _marker: std::marker::PhantomData,
            },
        }
    }
}

/// One decoded section: block count plus its two containers.
pub struct DecodedSection<T> {
    pub block_count: i16,
    /// Number of fluid blocks (26.x section header); 0 lets meshers skip fluids.
    pub fluid_count: i16,
    pub blocks: PalettedContainer<T>,
    pub biomes: PalettedContainer<T>,
}

/// Concrete typed aliases.
pub type BlockContainer = PalettedContainer<BlockStateId>;
pub type BiomeContainer = PalettedContainer<haste_core::BiomeId>;

impl PalettedContainer<BlockStateId> {
    /// Blocks accessor with (x, y, z) in 0..16.
    pub fn get_block(&self, x: u8, y: u8, z: u8) -> BlockStateId {
        BlockStateId(self.get_index(flat_index(x, y, z)))
    }

    /// Largest block-state id in this container that is outside the local table (`table_len`
    /// states), if any. A modded server can send ids past the end of our data-generator table;
    /// they decode fine and read back as the placeholder state.
    pub fn largest_unknown_id(&self, table_len: u32) -> Option<u32> {
        match &self.storage {
            Storage::Single(v) => (*v >= table_len).then_some(*v),
            Storage::Indirect { palette, .. } => {
                palette.iter().copied().filter(|&v| v >= table_len).max()
            }
            Storage::Direct(data) => (0..self.capacity)
                .map(|i| data.read_at(i))
                .filter(|&v| v >= table_len)
                .max(),
        }
    }
}

/// Set the first time any chunk decodes an out-of-range block-state id: a modded server
/// sent an id our local table does not know. Logged once per session.
static UNKNOWN_BLOCK_STATE_WARNED: AtomicBool = AtomicBool::new(false);

/// Logs a one-per-session warning that a chunk contained a block-state id outside the table,
/// naming the largest id seen and the table size. `warned` is the session flag; returns
/// whether this call logged (false when an earlier call already did).
pub fn warn_unknown_block_state_once(warned: &AtomicBool, largest: u32, table_len: u32) -> bool {
    if warned.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    tracing::warn!(
        largest,
        table_len,
        "chunk block-state id outside the local table (modded server?); \
         unknown ids collide and render as the placeholder"
    );
    true
}

/// Session-wide entry point for the once-per-session unknown-block-state warning.
pub fn warn_unknown_block_state(largest: u32, table_len: u32) {
    warn_unknown_block_state_once(&UNKNOWN_BLOCK_STATE_WARNED, largest, table_len);
}

impl PalettedContainer<haste_core::BiomeId> {
    pub fn get_biome(&self, x: u8, y: u8, z: u8) -> haste_core::BiomeId {
        haste_core::BiomeId(self.get_index(biome_flat_index(x, y, z)))
    }
}

/// Indirect bit width for a palette of `palette_len` entries (blocks: 4..=8; biomes: 1..=3 via
/// the caller choosing the palette size). Vanilla's rule: minimum bits that fit, floored at 4
/// for blocks.
pub fn bits_for(palette_len: usize) -> u8 {
    ((usize::BITS - palette_len.leading_zeros()).max(1) as u8).clamp(4, 8)
}

/// Vanilla flat index: y * 256 + z * 16 + x.
pub fn flat_index(x: u8, y: u8, z: u8) -> usize {
    (y as usize) * 256 + (z as usize) * 16 + x as usize
}

/// Biome containers hold 4x4x4 cells per section; index by cell coords in 0..4.
pub fn biome_flat_index(x: u8, y: u8, z: u8) -> usize {
    (y as usize) * 16 + (z as usize) * 4 + x as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip_direct_bits() -> u8 {
        15 // enough for 35723 states in tests
    }

    #[test]
    fn single_value_container() {
        let mut w = WireWriter::new();
        w.varint(0); // bpe 0
        w.varint(7); // single value
        let mut r = WireReader::new(&w.data);
        let c: BlockContainer = PalettedContainer::decode(&mut r, BLOCKS_PER_SECTION, 15).unwrap();
        assert_eq!(c.get_index(0), 7);
        assert_eq!(c.get_index(4095), 7);
    }

    /// Vanilla's `Strategy::createForBiomes` maps bits-per-entry 0 to the zero-bits
    /// configuration: a single value with NO data array on the wire. Our biome storage
    /// must be `Single` too (no palette, no packed longs), for block capacity and for
    /// the 64-entry biome capacity alike.
    #[test]
    fn single_value_biome_container_has_no_data_array() {
        for (value, bits) in [(0u32, 3u8), (42, 7)] {
            let mut w = WireWriter::new();
            w.varint(0); // bpe 0
            w.varint(value);
            let mut r = WireReader::new(&w.data);
            let c: BiomeContainer =
                PalettedContainer::decode(&mut r, BIOMES_PER_SECTION, bits).unwrap();
            assert!(matches!(c.storage, Storage::Single(v) if v == value));
            // no data array was on the wire: nothing left to read
            assert_eq!(r.remaining(), 0);
            // every cell reads the value, at cell coordinates and flat
            for &(x, y, z) in &[(0u8, 0u8, 0u8), (3, 3, 3), (1, 2, 0)] {
                assert_eq!(c.get_biome(x, y, z), haste_core::BiomeId(value));
            }
            assert_eq!(c.get_index(0), value);
            assert_eq!(c.get_index(63), value);
            // round trip: encode writes bpe 0 + the value and nothing else
            let mut w2 = WireWriter::new();
            c.encode(&mut w2, bits);
            assert_eq!(w2.data, w.data);
        }
    }

    /// The one-entry palette of `indirect` is the same thing as a single-value container.
    #[test]
    fn indirect_with_one_entry_is_single_value() {
        let blocks = BlockContainer::indirect(vec![9], &[9; BLOCKS_PER_SECTION]);
        assert!(matches!(blocks.storage, Storage::Single(9)));
        assert_eq!(blocks.get_index(1234), 9);
        let biomes = BiomeContainer::indirect(vec![4], &[4, 4, 4, 4]);
        assert!(matches!(biomes.storage, Storage::Single(4)));
        assert_eq!(biomes.get_index(3), 4);
        // a differing entry keeps the indirect form (the palette cannot express it)
        let mixed = BlockContainer::indirect(vec![9], &[9, 9, 8]);
        assert!(matches!(mixed.storage, Storage::Indirect { .. }));
    }

    /// A biome container stored as a single value promotes to an indirect palette on the
    /// first `set_index`, keeping the 64-entry capacity.
    #[test]
    fn single_value_biome_promotes_on_set() {
        let mut c = BiomeContainer::single_biome(3);
        assert_eq!(c.set_index(biome_flat_index(1, 2, 3), 5, 3), 3);
        assert!(matches!(c.storage, Storage::Indirect { .. }));
        assert_eq!(c.get_index(biome_flat_index(1, 2, 3)), 5);
        assert_eq!(c.get_index(0), 3);
        assert_eq!(c.get_index(63), 3);
        let mut c = BlockContainer::single(6);
        assert_eq!(c.set_index(1000, 6, 15), 6); // same value: no promotion
        assert!(matches!(c.storage, Storage::Single(6)));
    }

    #[test]
    fn indirect_roundtrip() {
        for bits in [4u8, 5, 6, 7, 8] {
            let palette: Vec<u32> = (0..(1 << bits)).map(|i| i as u32 * 3 + 1).collect();
            let mut data = PackedArray {
                bits,
                longs: vec![0; PackedArray::long_count(bits, BLOCKS_PER_SECTION)],
            };
            for i in 0..BLOCKS_PER_SECTION {
                data.write_at(i, (i % palette.len()) as u32);
            }
            let mut w = WireWriter::new();
            w.varint(bits as u32);
            w.varint(palette.len() as u32);
            for &p in &palette {
                w.varint(p);
            }
            for &l in &data.longs {
                w.i64_be(l);
            }
            let mut r = WireReader::new(&w.data);
            let c: BlockContainer =
                PalettedContainer::decode(&mut r, BLOCKS_PER_SECTION, 15).unwrap();
            for i in [0usize, 1, 255, 256, 4095, 1000] {
                let want = palette[i % palette.len()];
                assert_eq!(c.get_index(i), want, "bits {bits} index {i}");
            }
            // round trip back through encode
            let mut w2 = WireWriter::new();
            c.encode(&mut w2, roundtrip_direct_bits());
            let mut r2 = WireReader::new(&w2.data);
            let c2: BlockContainer =
                PalettedContainer::decode(&mut r2, BLOCKS_PER_SECTION, 15).unwrap();
            for i in [0usize, 1, 255, 256, 4095, 1000] {
                assert_eq!(c2.get_index(i), c.get_index(i));
            }
        }
    }

    #[test]
    fn direct_roundtrip() {
        let bits = 15u8;
        let mut data = PackedArray {
            bits,
            longs: vec![0; PackedArray::long_count(bits, BLOCKS_PER_SECTION)],
        };
        for i in 0..BLOCKS_PER_SECTION {
            data.write_at(i, (i as u32 * 7919) % 32768);
        }
        let mut w = WireWriter::new();
        w.varint(bits as u32); // direct: bpe field carries the direct width
        for &l in &data.longs {
            w.i64_be(l);
        }
        let mut r = WireReader::new(&w.data);
        let c: BlockContainer = PalettedContainer::decode(&mut r, BLOCKS_PER_SECTION, 15).unwrap();
        assert!(matches!(c.storage, Storage::Direct(_)));
        for i in [0usize, 1, 999, 4095] {
            assert_eq!(c.get_index(i), (i as u32 * 7919) % 32768);
        }
    }

    #[test]
    fn set_index_promotes_palette() {
        let mut c: BlockContainer = PalettedContainer {
            storage: Storage::Single(1),
            capacity: BLOCKS_PER_SECTION,
            _marker: std::marker::PhantomData,
        };
        assert_eq!(c.set_index(10, 2, 15), 1);
        assert_eq!(c.get_index(10), 2);
        assert_eq!(c.get_index(0), 1);
        assert_eq!(c.set_index(20, 3, 15), 1);
        assert_eq!(c.get_index(20), 3);
        // rewrite existing
        assert_eq!(c.set_index(10, 3, 15), 2);
        assert_eq!(c.get_index(10), 3);
    }

    #[test]
    fn sections_payload_roundtrip() {
        // build a 2-section payload with our encoder and decode it back
        let mut w = WireWriter::new();
        // section 0: block_count 3, fluid_count 0, blocks single 5, biomes single 1
        w.i16_be(3);
        w.i16_be(0);
        w.varint(0); // bpe 0
        w.varint(5); // value
        w.varint(0); // bpe 0
        w.varint(1); // biome
                     // section 1: indirect 4-bit blocks, biomes single
        let palette: Vec<u32> = vec![1, 2, 3, 5];
        let bits = 4u8;
        let longs = PackedArray::long_count(bits, BLOCKS_PER_SECTION);
        w.i16_be(4096 - 3);
        w.i16_be(0);
        w.varint(bits as u32);
        w.varint(palette.len() as u32);
        for &p in &palette {
            w.varint(p);
        }
        let mut data = PackedArray {
            bits,
            longs: vec![0; longs],
        };
        for i in 0..BLOCKS_PER_SECTION {
            data.write_at(i, (i % 4) as u32);
        }
        for &l in &data.longs {
            w.i64_be(l);
        }
        w.varint(0);
        w.varint(42);
        let sections: Vec<DecodedSection<BlockStateId>> =
            PalettedContainer::decode_sections(&w.data, 2, 15, 7).unwrap();
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].block_count, 3);
        assert_eq!(sections[0].blocks.get_index(0), 5);
        assert_eq!(sections[0].biomes.get_index(0), 1);
        assert_eq!(sections[1].block_count, 4093);
        // index i has palette index i%4; i=5 -> 1 -> palette[1] = 2
        assert_eq!(sections[1].blocks.get_index(5), 2);
        assert_eq!(w.data.len(), {
            let mut r = WireReader::new(&w.data);
            for _ in 0..2 {
                r.i16_be().unwrap();
                r.i16_be().unwrap();
                PalettedContainer::<BlockStateId>::decode(&mut r, BLOCKS_PER_SECTION, 15).unwrap();
                PalettedContainer::<BlockStateId>::decode(&mut r, BIOMES_PER_SECTION, 7).unwrap();
            }
            w.data.len() - r.remaining()
        });
    }

    /// Regression (26.3 server): the section header is block count and fluid count, two
    /// big-endian i16s. A full section (4096 = 0x1000) read as a VarInt desynchronised the
    /// whole payload.
    #[test]
    fn section_header_is_two_big_endian_shorts() {
        let mut w = WireWriter::new();
        // full stone section with water counted: 4096 blocks, 300 fluids
        w.data.extend_from_slice(&[0x10, 0x00, 0x01, 0x2C]);
        w.varint(0);
        w.varint(1);
        w.varint(0);
        w.varint(2);
        // second section: empty
        w.data.extend_from_slice(&[0, 0, 0, 0]);
        w.varint(0);
        w.varint(0);
        w.varint(0);
        w.varint(2);
        let s: Vec<DecodedSection<BlockStateId>> =
            PalettedContainer::decode_sections(&w.data, 2, 15, 7).unwrap();
        assert_eq!((s[0].block_count, s[0].fluid_count), (4096, 300));
        assert_eq!(s[0].blocks.get_index(4095), 1);
        assert_eq!((s[1].block_count, s[1].fluid_count), (0, 0));
        assert_eq!(s[1].biomes.get_index(63), 2);
    }

    /// Regression (26.3 server, 67 biomes): biome containers wider than 3 bits are direct at
    /// ceil(log2(biome registry size)) bits, not at 3; 3 bits and below stay indirect.
    #[test]
    fn biome_containers_direct_above_three_bits() {
        let mut w = WireWriter::new();
        w.i16_be(1);
        w.i16_be(0);
        w.varint(0);
        w.varint(1);
        // direct biomes at 7 bits: 9 entries per long, 64 entries -> 8 longs
        let mut data = PackedArray {
            bits: 7,
            longs: vec![0; PackedArray::long_count(7, BIOMES_PER_SECTION)],
        };
        for i in 0..BIOMES_PER_SECTION {
            data.write_at(i, i as u32 + 3);
        }
        w.varint(7);
        for &l in &data.longs {
            w.i64_be(l);
        }
        // second section: indirect 3-bit biomes still decode with biome_bits 7
        w.i16_be(0);
        w.i16_be(0);
        w.varint(0);
        w.varint(0);
        w.varint(3);
        let palette = [10u32, 20, 30, 40, 50];
        w.varint(palette.len() as u32);
        for p in palette {
            w.varint(p);
        }
        let mut idx = PackedArray {
            bits: 3,
            longs: vec![0; PackedArray::long_count(3, BIOMES_PER_SECTION)],
        };
        for i in 0..BIOMES_PER_SECTION {
            idx.write_at(i, (i % 5) as u32);
        }
        for &l in &idx.longs {
            w.i64_be(l);
        }
        let s: Vec<DecodedSection<BlockStateId>> =
            PalettedContainer::decode_sections(&w.data, 2, 15, 7).unwrap();
        assert_eq!(s[0].biomes.get_index(0), 3);
        assert_eq!(s[0].biomes.get_index(63), 66);
        assert_eq!(s[1].biomes.get_index(7), 30);
        // With the old fixed 3-bit width the first section's 8 longs are misread.
        assert!(
            PalettedContainer::<BlockStateId>::decode_sections(&w.data, 2, 15, 3)
                .map_or(true, |s| s[0].biomes.get_index(63) != 66)
        );
    }

    /// A modded server can send ids past the end of the local table: the container decodes
    /// them fine and reports the largest one for the once-per-session warning.
    #[test]
    fn largest_unknown_id_per_storage() {
        // single-value
        assert_eq!(BlockContainer::single(999).largest_unknown_id(6), Some(999));
        assert_eq!(BlockContainer::single(3).largest_unknown_id(6), None);
        // indirect: the palette holds the global ids
        let c = BlockContainer::indirect(vec![1, 2, 999], &[1, 999, 2]);
        assert_eq!(c.largest_unknown_id(6), Some(999));
        assert_eq!(
            BlockContainer::indirect(vec![1, 2, 3], &[1, 2, 3]).largest_unknown_id(6),
            None
        );
        // direct: raw ids in the packed array
        let mut data = PackedArray {
            bits: 15,
            longs: vec![0; PackedArray::long_count(15, BLOCKS_PER_SECTION)],
        };
        data.write_at(100, 4242);
        let c = BlockContainer {
            storage: Storage::Direct(data),
            capacity: BLOCKS_PER_SECTION,
            _marker: std::marker::PhantomData,
        };
        assert_eq!(c.largest_unknown_id(6), Some(4242));
        assert_eq!(c.largest_unknown_id(5000), None);
    }

    /// The unknown-block-state warning fires once per session, naming the first (largest) id.
    #[test]
    fn unknown_block_state_warning_fires_once() {
        let warned = AtomicBool::new(false);
        assert!(warn_unknown_block_state_once(&warned, 999, 6));
        assert!(!warn_unknown_block_state_once(&warned, 1234, 6));
        assert!(warned.load(std::sync::atomic::Ordering::Relaxed));
    }
}

#[cfg(test)]
mod bit_order_tests {
    use super::PackedArray;

    /// Vanilla packs entry 0 into the least significant bits of each long. Found against the
    /// 26.3 server: a 3x3 pad at x 20..22 decoded mirrored with the old MSB-first order.
    #[test]
    fn entry_zero_is_in_the_low_bits() {
        let a = PackedArray {
            bits: 4,
            longs: vec![0x0000_0000_0000_0321],
        };
        assert_eq!(
            [a.read_at(0), a.read_at(1), a.read_at(2), a.read_at(3)],
            [1, 2, 3, 0]
        );
        // 5 bits: 12 entries per long, 4 padding bits at the top.
        let b = PackedArray {
            bits: 5,
            longs: vec![(7 << 5) | 3],
        };
        assert_eq!([b.read_at(0), b.read_at(1)], [3, 7]);
        let mut c = PackedArray {
            bits: 9,
            longs: vec![0],
        };
        c.write_at(1, 0x1ff);
        assert_eq!(c.longs[0], 0x1ff << 9);
    }
}
