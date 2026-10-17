//! The particle pool's random source.

/// xorshift64*, deterministically seeded (tests rely on the repetition).
pub(super) struct Rng(pub(super) u64);

impl Rng {
    /// Uniform f32 in [0, 1).
    pub(super) fn next_f32(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / 16_777_216.0
    }

    /// Uniform f64 in [0, 1) (`RandomSource.nextDouble`).
    pub(super) fn next_f64(&mut self) -> f64 {
        self.next_f32() as f64
    }

    /// Approximate standard gaussian: the sum of three uniforms in (-1, 1) (variance 1,
    /// range ±3), standing in for vanilla's `Random.nextGaussian`.
    pub(super) fn gaussian(&mut self) -> f32 {
        self.next_f32() * 2.0 - 1.0 + (self.next_f32() * 2.0 - 1.0) + (self.next_f32() * 2.0 - 1.0)
    }
}
