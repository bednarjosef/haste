//! Vanilla's `LegacyRandomSource`: the 48-bit linear congruential generator every sound
//! instance draws from (`RandomSource.create(seed)`), so the variant a sound packet's seed
//! picks and the music manager's pauses follow the same sequence as in the vanilla client.
//!
//! The generator is `java.util.Random`'s: the seed is scrambled with the multiplier, each
//! step is `seed * 0x5DEECE66D + 11` modulo 2^48, and `next(bits)` takes the top bits.
//! `next_int(bound)` is `BitRandomSource.nextInt(int)` (the power-of-two shortcut and the
//! rejection loop), `next_int_between` is `Mth.nextInt(RandomSource, int, int)`.

/// The LCG's multiplier.
const MULTIPLIER: i64 = 0x5_DEEC_E66D;
/// The LCG's increment.
const INCREMENT: i64 = 11;
/// The state is 48 bits wide.
const MASK: i64 = (1 << 48) - 1;

/// Vanilla's `LegacyRandomSource` (the generator `RandomSource.create(seed)` returns).
#[derive(Clone, Debug)]
pub struct LegacyRandom {
    seed: i64,
}

impl LegacyRandom {
    /// `RandomSource.create(seed)`: the seed scrambled with the multiplier.
    pub fn new(seed: i64) -> Self {
        Self {
            seed: (seed ^ MULTIPLIER) & MASK,
        }
    }

    /// `LegacyRandomSource.next(bits)`: one LCG step, the top `bits` of the 48-bit state.
    pub fn next(&mut self, bits: u32) -> i32 {
        self.seed = self.seed.wrapping_mul(MULTIPLIER).wrapping_add(INCREMENT) & MASK;
        (self.seed >> (48 - bits)) as i32
    }

    /// `BitRandomSource.nextInt(bound)`: uniform in `0..bound`. A bound below 1 answers 0
    /// (vanilla throws; no caller here passes one).
    pub fn next_int(&mut self, bound: i32) -> i32 {
        if bound <= 0 {
            return 0;
        }
        if bound & (bound - 1) == 0 {
            return ((i64::from(bound) * i64::from(self.next(31))) >> 31) as i32;
        }
        loop {
            let sample = self.next(31);
            let modulo = sample % bound;
            // The rejection test relies on Java's wrapping int addition.
            if sample.wrapping_sub(modulo).wrapping_add(bound - 1) >= 0 {
                return modulo;
            }
        }
    }

    /// `Mth.nextInt(random, min, max)`: uniform in `min..=max`; `min` when `min >= max`.
    pub fn next_int_between(&mut self, min: i32, max: i32) -> i32 {
        if min >= max {
            min
        } else {
            self.next_int(max - min + 1) + min
        }
    }

    /// `BitRandomSource.nextLong()`: two 32-bit draws, the upper one shifted.
    pub fn next_long(&mut self) -> i64 {
        let upper = i64::from(self.next(32));
        let lower = i64::from(self.next(32));
        (upper << 32).wrapping_add(lower)
    }

    /// `BitRandomSource.nextFloat()`: 24 bits scaled into `0.0..1.0`.
    pub fn next_float(&mut self) -> f32 {
        self.next(24) as f32 * 5.960_464_5E-8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The expected values were produced by `java.util.Random`, whose generator vanilla's
    // `LegacyRandomSource` is (the JDK in `cache/jdk`, one fresh `Random(seed)` per row
    // drawing nextInt(3), nextInt(4), nextInt(100), nextLong in that order).
    #[test]
    fn matches_java_util_random() {
        let rows: [(i64, i32, i32, i32, i64); 5] = [
            (0, 0, 3, 29, -7_261_648_964_369_397_258),
            (1, 0, 0, 47, 7_515_937_759_503_895_804),
            (42, 2, 0, 48, 884_324_181_205_335_268),
            (-7, 1, 3, 90, -998_974_524_265_310_116),
            (123_456_789, 1, 3, 83, 4_701_514_676_984_888_228),
        ];
        for (seed, three, four, hundred, long) in rows {
            let mut r = LegacyRandom::new(seed);
            assert_eq!(r.next_int(3), three, "seed {seed} nextInt(3)");
            assert_eq!(r.next_int(4), four, "seed {seed} nextInt(4)");
            assert_eq!(r.next_int(100), hundred, "seed {seed} nextInt(100)");
            assert_eq!(r.next_long(), long, "seed {seed} nextLong");
        }
    }

    #[test]
    fn next_int_between_is_mth_next_int() {
        // java.util.Random(5): nextInt(12001) + 12000, eight times.
        let mut r = LegacyRandom::new(5);
        let got: Vec<i32> = (0..8).map(|_| r.next_int_between(12_000, 24_000)).collect();
        assert_eq!(
            got,
            [20767, 18491, 13681, 23945, 19021, 20510, 13212, 18830]
        );
        // min >= max answers min without drawing.
        assert_eq!(r.next_int_between(7, 7), 7);
        assert_eq!(r.next_int_between(9, 3), 9);
    }
}
