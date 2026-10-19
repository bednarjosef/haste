//! Sky-light storage census (diagnostics): how the loaded chunks' per-section light is
//! stored under `light.rs`'s layout, and what it costs. The headless summary prints it
//! so storage-layout changes get before/after numbers from the same spot.

use crate::light::SkyLight;
use crate::world::World;

/// How the loaded chunks' sky light is stored (see [`World::sky_light_stats`]).
#[derive(Default)]
pub struct SkyLightStats {
    pub sections: u64,
    /// No layer: reads resolve through the walk-up (`crate::sky_light`).
    pub absent: u64,
    /// A layer whose every cell holds one value other than 0.
    pub uniform: u64,
    /// A layer whose every cell is 0 (explicitly-dark sections).
    pub uniform_zero: u64,
    /// A non-uniform 2048-byte array.
    pub arrays: u64,
    /// Bytes the sky arrays cost (2048 per array plus headers).
    pub array_bytes: u64,
    /// Block-light arrays (non-uniform, non-zero).
    pub block_arrays: u64,
    /// Bytes the block arrays cost.
    pub block_array_bytes: u64,
    /// Distinct array contents among all stored arrays (sky + block): what a
    /// content-sharing table could save is `arrays - distinct` arrays.
    pub distinct_arrays: u64,
    /// Bytes of arrays whose contents another stored section already holds.
    pub duplicate_array_bytes: u64,
}

impl World {
    /// The sky-light storage census over the loaded chunks. Block light is counted
    /// alongside (same storage, no walk-up).
    pub fn sky_light_stats(&self) -> SkyLightStats {
        let mut stats = SkyLightStats::default();
        // Contents already seen (a diagnostic: how much content-sharing would save).
        let mut seen: std::collections::HashSet<Vec<u8>> = Default::default();
        let stored = |stats: &mut SkyLightStats,
                      seen: &mut std::collections::HashSet<Vec<u8>>,
                      bytes: Vec<u8>| {
            if seen.insert(bytes) {
                stats.distinct_arrays += 1;
            } else {
                stats.duplicate_array_bytes += 2048 + 16;
            }
        };
        for pos in self.loaded_chunks() {
            let Some(chunk) = self.chunk(pos) else {
                continue;
            };
            for slot in &chunk.light.sky {
                stats.sections += 1;
                match slot {
                    SkyLight::Absent => stats.absent += 1,
                    SkyLight::Uniform(0) => stats.uniform_zero += 1,
                    SkyLight::Uniform(_) => stats.uniform += 1,
                    SkyLight::Array(a) => {
                        stats.arrays += 1;
                        stats.array_bytes += 2048 + 16;
                        stored(&mut stats, &mut seen, a.to_vec());
                    }
                }
            }
            for slot in &chunk.light.block {
                if !(slot.all_zero() || slot.all_full()) {
                    stats.block_arrays += 1;
                    stats.block_array_bytes += 2048 + 16;
                    stored(&mut stats, &mut seen, slot.to_vec());
                }
            }
        }
        stats
    }
}
