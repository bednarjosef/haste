// Java's linear congruential random, shared by the passes that must reproduce vanilla's
// seeded patterns (lightning bolts, stars).

/// Java's `java.util.Random` LCG, the generator behind vanilla's `SingleThreadedRandomSource`
/// and the bolt's random (`LightningBoltRenderer` seeds it from the bolt's seed). The field is
/// the raw 48-bit state.
pub(crate) struct JavaRandom(pub(crate) u64);

/// The Java LCG's multiplier, 25214903917 (0x5DEECE66D).
const LCG_MULTIPLIER: u64 = 25_214_903_917;
/// The Java LCG's increment.
const LCG_INCREMENT: u64 = 11;
/// The Java LCG's 48-bit mask.
const LCG_MASK: u64 = (1u64 << 48) - 1;

impl JavaRandom {
    pub(crate) fn next_i32(&mut self, bits: u32) -> i32 {
        self.0 = (self
            .0
            .wrapping_mul(LCG_MULTIPLIER)
            .wrapping_add(LCG_INCREMENT))
            & LCG_MASK;
        (self.0 >> (48 - bits)) as i32
    }

    /// `nextInt(bound)` (bounds 11 and 31 are not powers of two: the rejection path).
    pub(crate) fn next_int(&mut self, bound: i32) -> i32 {
        if bound & (bound - 1) == 0 {
            return (((bound as i64) * (self.next_i32(31) as i64)) >> 31) as i32;
        }
        loop {
            let sample = self.next_i32(31);
            let modulo = sample % bound;
            if sample - modulo + (bound - 1) >= 0 {
                return modulo;
            }
        }
    }
}
