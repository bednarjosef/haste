// The renderer's resident sections in a dense slab, addressed by
// `SectionPos` through a slot map: the frame cull iterates only the
// occlusion walk's reached sections (the visible subset) and looks each up
// by position, instead of walking every resident section in a hash map.

use haste_core::{ChunkPos, SectionPos};

use crate::arena::ResidentSection;
use crate::quad_format::SpanKind;

/// One resident section: the arena placement plus the SoA columns the cull
/// loop reads (world bounds for the frustum test and the bonus flag for
/// newly uploaded sections next to visible ones).
pub struct SlabSection {
    pub pos: SectionPos,
    pub arena: u32,
    pub base_vertex: u32,
    /// For `MeshArenas::free` (vertex/index/quad ranges of the whole section).
    pub vertex_count: u32,
    pub base_quad: u32,
    /// Quad format: the section's general records.
    pub base_general: u32,
    pub general_count: u32,
    pub quad_count: u32,
    pub index_start: u32,
    pub index_total: u32,
    pub layers_len: u32,
    /// World-space bounds.
    pub aabb: ([f32; 3], [f32; 3]),
    /// SoA twin of `ResidentSection::layers`, indexed by
    /// `layers_base..layers_base + layers_len`.
    layers_base: u32,
    /// Newly uploaded next to a visible section: drawn until the next walk
    /// (vanilla propagates from newly loaded sections).
    pub bonus: bool,
}

/// One section layer to draw: the render layer plus its arena record range.
#[derive(Clone, Copy)]
pub struct SlabLayer {
    pub layer: haste_assets::RenderLayer,
    pub first: u32,
    pub count: u32,
    /// Facing group (`Direction::ALL` index) of this range; 6 = faces off the
    /// axis planes (crossed plants, sloped fluids), never direction-skipped. See `LayerDraw`.
    pub dir: u8,
    /// Which pool `first` addresses (indices or quad-format records).
    pub kind: SpanKind,
}

impl SlabLayer {
    /// Whether this whole direction group is back-facing from the camera:
    /// every quad in it points away, so the draw can skip it (the rasterizer
    /// would cull each quad only after vertex shading). `lo`/`hi` is the
    /// section's camera-relative AABB. A positive-direction face (up, south,
    /// east) is back-facing when the camera is below/before the section's low
    /// side on the face's axis, a negative one when the camera is past the
    /// high side; inside the slab the group is mixed and kept.
    pub fn back_facing(&self, lo: [f32; 3], hi: [f32; 3]) -> bool {
        // Direction axis: down/up = y, north/south = z, west/east = x.
        let (axis, positive) = match self.dir {
            0 => (1, false),
            1 => (1, true),
            2 => (2, false),
            3 => (2, true),
            4 => (0, false),
            _ => (0, true),
        };
        if positive {
            lo[axis] > 0.0
        } else {
            hi[axis] < 0.0
        }
    }
}

/// Dense storage of resident sections with `SectionPos` lookup: a slab of
/// records plus SoA side tables (layers, bounds) the cull reads linearly.
/// Removals are tombstones (`None` slots); a freed slot goes on the free
/// list for reuse, so iteration stays dense only over live sections and the
/// position lookup is one hash of the packed section key.
pub struct SectionSlab {
    slots: Vec<Option<SlabSection>>,
    /// `layers_base` of each live slot -> its layers, packed.
    layers: Vec<SlabLayer>,
    /// Packed section key -> slot index (positions are unique among resident
    /// sections; a re-upload at the same position replaces the record in
    /// place, so no generation tags are needed).
    index: ahash::AHashMap<u64, u32>,
    free: Vec<u32>,
    /// Entries of `layers` no live slot points at (replaced or removed sections whose
    /// range was not at the tail); compacted away once they outnumber the live ones.
    dead_layers: usize,
    /// The compaction's second buffer, kept so a compaction allocates nothing.
    layer_scratch: Vec<SlabLayer>,
}

/// Dead layer entries tolerated before a compaction (a section has at most a few dozen).
const DEAD_LAYERS_MIN: usize = 4096;

fn key(pos: SectionPos) -> u64 {
    // Section coordinates fit in 21 bits each (±1M sections).
    ((pos.x as u64 & 0x1f_ffff) << 42)
        | ((pos.y as u64 & 0x1f_ffff) << 21)
        | (pos.z as u64 & 0x1f_ffff)
}

impl Default for SectionSlab {
    fn default() -> Self {
        Self::new()
    }
}

impl SectionSlab {
    pub fn new() -> Self {
        SectionSlab {
            slots: Vec::new(),
            layers: Vec::new(),
            index: ahash::AHashMap::with_capacity(1024),
            free: Vec::new(),
            dead_layers: 0,
            layer_scratch: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Store `s` at its position, replacing any previous record (its layers
    /// are freed from the packed layer array). Returns the old record when
    /// there was one, so the caller can free its arena ranges.
    pub fn insert(&mut self, s: ResidentSection) -> Option<ResidentSection> {
        let old = self.remove(&s.pos);
        if self.dead_layers >= DEAD_LAYERS_MIN && self.dead_layers * 2 > self.layers.len() {
            self.compact_layers();
        }
        let layers_base = self.layers.len() as u32;
        self.layers.extend(s.layers.iter().map(|l| SlabLayer {
            layer: l.layer,
            first: l.first,
            count: l.count,
            dir: l.dir,
            kind: l.kind,
        }));
        let slot = match self.free.pop() {
            Some(i) => i,
            None => {
                self.slots.push(None);
                self.slots.len() as u32 - 1
            }
        };
        self.slots[slot as usize] = Some(SlabSection {
            pos: s.pos,
            arena: s.arena,
            base_vertex: s.base_vertex,
            vertex_count: s.vertex_count,
            base_quad: s.base_quad,
            quad_count: s.quad_count,
            base_general: s.base_general,
            general_count: s.general_count,
            index_start: s.index_start,
            index_total: s.index_total,
            layers_len: s.layers.len() as u32,
            aabb: s.aabb,
            layers_base,
            bonus: false,
        });
        self.index.insert(key(s.pos), slot);
        old
    }

    /// Remove the record at `pos` (tombstone the slot, free its layers).
    /// Returns the old record's arena identity for freeing (vertex/index
    /// counts live on the caller's side; the returned record carries the
    /// position and arena only).
    pub fn remove(&mut self, pos: &SectionPos) -> Option<ResidentSection> {
        let slot = self.index.remove(&key(*pos))?;
        let s = self.slots[slot as usize].take()?;
        // The newest record's range is the tail and goes at once; any other becomes dead
        // entries until the next compaction.
        let end = (s.layers_base + s.layers_len) as usize;
        if end == self.layers.len() {
            self.layers.truncate(s.layers_base as usize);
        } else {
            self.dead_layers += s.layers_len as usize;
        }
        self.free.push(slot);
        Some(ResidentSection {
            pos: s.pos,
            arena: s.arena,
            base_vertex: s.base_vertex,
            vertex_count: s.vertex_count,
            base_quad: s.base_quad,
            quad_count: s.quad_count,
            base_general: s.base_general,
            general_count: s.general_count,
            index_start: s.index_start,
            index_total: s.index_total,
            layers: Vec::new(),
            aabb: s.aabb,
        })
    }

    /// The record at `pos`, if resident.
    pub fn get(&self, pos: SectionPos) -> Option<&SlabSection> {
        let &slot = self.index.get(&key(pos))?;
        self.slots[slot as usize].as_ref()
    }

    /// The record at `pos` with its slot index (stable while the section stays
    /// resident; the GPU occlusion culling keys its visibility by it).
    pub fn get_with_slot(&self, pos: SectionPos) -> Option<(u32, &SlabSection)> {
        let &slot = self.index.get(&key(pos))?;
        Some((slot, self.slots[slot as usize].as_ref()?))
    }

    /// One past the highest slot index in use.
    pub fn slot_capacity(&self) -> usize {
        self.slots.len()
    }

    /// The record at `pos`, mutably (to set the bonus flag).
    pub fn get_mut(&mut self, pos: SectionPos) -> Option<&mut SlabSection> {
        let &slot = self.index.get(&key(pos))?;
        self.slots[slot as usize].as_mut()
    }

    /// Iterate every live record (clearing all resident sections).
    pub fn iter(&self) -> impl Iterator<Item = &SlabSection> {
        self.slots.iter().flatten()
    }

    /// Iterate every live record mutably (clearing bonus bits).
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut SlabSection> {
        self.slots.iter_mut().flatten()
    }

    /// The packed layers of a record.
    pub fn layers(&self, s: &SlabSection) -> &[SlabLayer] {
        &self.layers[s.layers_base as usize..(s.layers_base + s.layers_len) as usize]
    }

    /// Drop every record (the arenas keep their buffers; a rejoin reuses
    /// them, so the slab is emptied, not freed).
    pub fn clear(&mut self) {
        self.slots.clear();
        self.layers.clear();
        self.index.clear();
        self.free.clear();
        self.dead_layers = 0;
    }

    /// Packs the live slots' layer ranges into the scratch buffer and swaps the two, so
    /// replaced sections' entries do not pile up for the whole session.
    fn compact_layers(&mut self) {
        self.layer_scratch.clear();
        for s in self.slots.iter_mut().flatten() {
            let start = s.layers_base as usize;
            s.layers_base = self.layer_scratch.len() as u32;
            self.layer_scratch
                .extend_from_slice(&self.layers[start..start + s.layers_len as usize]);
        }
        std::mem::swap(&mut self.layers, &mut self.layer_scratch);
        self.dead_layers = 0;
    }

    /// Column key for the per-column visibility mask.
    pub fn column(pos: SectionPos) -> ChunkPos {
        ChunkPos::new(pos.x, pos.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::LayerDraw;

    fn resident(pos: SectionPos, n_layers: usize) -> ResidentSection {
        ResidentSection {
            pos,
            arena: 0,
            base_vertex: 0,
            vertex_count: 0,
            base_quad: 0,
            quad_count: 0,
            base_general: 0,
            general_count: 0,
            index_start: 0,
            index_total: 0,
            layers: (0..n_layers)
                .map(|i| LayerDraw {
                    layer: haste_assets::RenderLayer::Solid,
                    first: i as u32 * 6,
                    count: 6,
                    dir: 6,
                    kind: crate::quad_format::SpanKind::Indexed,
                })
                .collect(),
            aabb: ([0.0; 3], [16.0; 3]),
        }
    }

    #[test]
    fn insert_lookup_remove() {
        let mut slab = SectionSlab::new();
        slab.insert(resident(SectionPos::new(1, 2, 3), 2));
        slab.insert(resident(SectionPos::new(-1, 0, 5), 1));
        assert_eq!(slab.len(), 2);
        assert_eq!(slab.get(SectionPos::new(1, 2, 3)).unwrap().layers_len, 2);
        assert_eq!(slab.get(SectionPos::new(-1, 0, 5)).unwrap().layers_len, 1);
        assert!(slab.get(SectionPos::new(0, 0, 0)).is_none());
        // Replacing a position keeps one record.
        slab.insert(resident(SectionPos::new(1, 2, 3), 3));
        assert_eq!(slab.len(), 2);
        assert_eq!(slab.get(SectionPos::new(1, 2, 3)).unwrap().layers_len, 3);
        // Removal then reuse of the slot.
        slab.remove(&SectionPos::new(1, 2, 3));
        assert!(slab.get(SectionPos::new(1, 2, 3)).is_none());
        slab.insert(resident(SectionPos::new(1, 2, 3), 1));
        assert_eq!(slab.len(), 2);
        assert_eq!(slab.iter().count(), 2);
    }

    #[test]
    fn clear_empties() {
        let mut slab = SectionSlab::new();
        slab.insert(resident(SectionPos::new(0, 0, 0), 1));
        slab.clear();
        assert!(slab.is_empty());
        assert!(slab.get(SectionPos::new(0, 0, 0)).is_none());
        slab.insert(resident(SectionPos::new(0, 0, 0), 1));
        assert_eq!(slab.len(), 1);
    }

    #[test]
    fn layers_slice_matches() {
        let mut slab = SectionSlab::new();
        slab.insert(resident(SectionPos::new(0, 0, 0), 2));
        slab.insert(resident(SectionPos::new(0, 1, 0), 1));
        let s0 = slab.get(SectionPos::new(0, 0, 0)).unwrap();
        assert_eq!(slab.layers(s0).len(), 2);
        assert_eq!(slab.layers(s0)[1].first, 6);
        let s1 = slab.get(SectionPos::new(0, 1, 0)).unwrap();
        assert_eq!(slab.layers(s1).len(), 1);
        // After removing the middle record the other's slice is still right.
        slab.remove(&SectionPos::new(0, 0, 0));
        let s1 = slab.get(SectionPos::new(0, 1, 0)).unwrap();
        assert_eq!(slab.layers(s1).len(), 1);
    }

    #[test]
    fn replaced_sections_do_not_grow_the_layer_array() {
        let mut slab = SectionSlab::new();
        // Re-mesh 64 sections over and over, in an order that never replaces the tail.
        for round in 0..400 {
            for i in 0..64 {
                let n = 1 + (i + round) % 5;
                slab.insert(resident(SectionPos::new(i as i32, 0, 0), n));
            }
        }
        let live: usize = slab.iter().map(|s| s.layers_len as usize).sum();
        assert!(
            slab.layers.len() <= live + 2 * DEAD_LAYERS_MIN,
            "{} entries for {live} live",
            slab.layers.len()
        );
        // Every slice still belongs to its own section.
        for i in 0..64 {
            let s = slab.get(SectionPos::new(i, 0, 0)).unwrap();
            assert_eq!(slab.layers(s).len(), 1 + (i as usize + 399) % 5);
            for (j, l) in slab.layers(s).iter().enumerate() {
                assert_eq!(l.first, j as u32 * 6);
            }
        }
    }
}
