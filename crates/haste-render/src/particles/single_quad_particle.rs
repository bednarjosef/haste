//! Vanilla `SingleQuadParticle`'s quad size at spawn and its `getQuadSize` overrides.

/// Quad size at spawn (vanilla's `quadSize`), as the half-width in blocks.
#[derive(Clone, Copy, Debug)]
pub(super) enum Quad {
    /// `SingleQuadParticle`'s default `0.1 * (r * 0.5 + 0.5) * 2.0` (0.1..=0.2 half-width),
    /// times `scale`, re-randomised by a `lo + r * (hi - lo)` factor (`BubbleParticle`'s
    /// `r * 0.6 + 0.2`, where the default size itself is also random).
    Default { scale: f32, lo: f32, hi: f32 },
    /// `size * (r * hi + lo)`, where a class assigns `quadSize` outright (`PortalParticle`'s
    /// `0.1 * (r * 0.2 + 0.5)`).
    Formula { size: f32, lo: f32, hi: f32 },
    /// `HugeExplosionParticle`: `2.0 * (1.0 - size * 0.5)` with `size` from the packet.
    Explosion,
}

impl Quad {
    /// The quad's half-width in blocks. `r` is the default's random, `r2` the entry's
    /// factor, `speed_x` the packet's x max speed (the explosion size carrier).
    pub(super) fn half_width(self, r: f32, r2: f32, speed_x: f32) -> f32 {
        match self {
            Quad::Default { scale, lo, hi } => {
                0.1 * (r * 0.5 + 0.5) * 2.0 * scale * (lo + r2 * (hi - lo))
            }
            Quad::Formula { size, lo, hi } => size * (r * hi + lo),
            Quad::Explosion => 2.0 * (1.0 - speed_x * 0.5),
        }
    }
}

/// How the quad size changes over the particle's life (`getQuadSize` overrides).
#[derive(Clone, Copy, Debug)]
pub(super) enum QuadGrowth {
    /// Unchanged (`SingleQuadParticle.getQuadSize`).
    Constant,
    /// `clamp((age + a) / lifetime * 32, 0, 1)`: full size after 1/32 of the life
    /// (`BaseAshSmokeParticle`, `CritParticle`, `HeartParticle`, `NoteParticle`,
    /// `DustParticleBase`, `PlayerCloudParticle`).
    GrowEarly,
    /// `FlameParticle`: `quadSize * (1 - s²/2)` with `s = (age + a) / lifetime`.
    ShrinkLate,
    /// `PortalParticle`: `quadSize * (1 - (1 - s)²)` with `s = age / lifetime`.
    Portal,
}

impl QuadGrowth {
    /// The multiplier for a normalized age `s` in 0..=1.
    pub(super) fn scale(self, s: f32) -> f32 {
        match self {
            QuadGrowth::Constant => 1.0,
            QuadGrowth::GrowEarly => (s * 32.0).clamp(0.0, 1.0),
            QuadGrowth::ShrinkLate => 1.0 - s * s * 0.5,
            QuadGrowth::Portal => 1.0 - (1.0 - s) * (1.0 - s),
        }
    }
}
