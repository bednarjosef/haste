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
    /// A generator seeded like `SingleThreadedRandomSource.setSeed` (the seed scrambled with
    /// the multiplier).
    pub(crate) fn new(seed: i64) -> JavaRandom {
        JavaRandom((seed as u64 ^ LCG_MULTIPLIER) & LCG_MASK)
    }

    /// `BitRandomSource.nextFloat`: 24 bits times 2^-24.
    pub(crate) fn next_float(&mut self) -> f32 {
        self.next_i32(24) as f32 * 5.960_464_5e-8
    }

    /// `BitRandomSource.nextDouble`: 26 + 27 bits times vanilla's float constant 1.110223e-16
    /// (widened to double).
    pub(crate) fn next_double(&mut self) -> f64 {
        let upper = self.next_i32(26) as i64;
        let lower = self.next_i32(27) as i64;
        ((upper << 27) + lower) as f64 * (1.110_223e-16_f32 as f64)
    }

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
