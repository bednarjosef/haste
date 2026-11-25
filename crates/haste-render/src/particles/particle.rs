//! Vanilla `Particle`'s lifetime roll and spawn-time velocity handling, as the per-type
//! behaviour table parameterises them.

/// Lifetime in ticks, as the vanilla classes compute it.
#[derive(Clone, Copy, Debug)]
pub(super) enum Lifetime {
    /// `(int)(base / (r * (1.0 - low) + low)) * scale + add`, at least `min` ticks: the
    /// `(int)(base / (r * jitter + low))` form every vanilla particle class uses.
    Rolled {
        base: f32,
        low: f32,
        scale: f32,
        add: f32,
        min: f32,
    },
    /// A fixed number of ticks.
    Fixed(u32),
    /// `(int)(r * max) + base` (`PortalParticle`, `HugeExplosionParticle`).
    Rand { base: u32, max: u32 },
}

impl Lifetime {
    /// Rolls this lifetime with `r`, never below one tick.
    pub(super) fn roll(self, r: f32) -> f32 {
        match self {
            Lifetime::Rolled {
                base,
                low,
                scale,
                add,
                min,
            } => {
                let rolled = (base / (r * (1.0 - low) + low)).floor();
                (rolled * scale + add).max(min).max(1.0)
            }
            Lifetime::Fixed(ticks) => ticks as f32,
            Lifetime::Rand { base, max } => (base + (r * max as f32) as u32) as f32,
        }
    }
}

/// Spawn-time velocity handling, as the vanilla classes' constructors do it. `speed` is the
/// packet's velocity (`offset * max_speed` for count 0, `gaussian * max_speed` for count > 0)
/// and `jittered` is `Particle`'s speed constructor: `speed` plus a ±0.4 random offset per
/// axis, rescaled to `(r+r+1) * 0.15` and lifted by 0.1 on y.
#[derive(Clone, Copy, Debug)]
pub(super) enum Motion {
    /// `jittered`, then the packet speed scaled by `keep` and added back (`SmokeParticle`
    /// 0.1, `CritParticle` 0.1 and its `xa * 0.4`, `SuspendedTownParticle` 0.02,
    /// `DustParticleBase` 0.1, `PlayerCloudParticle` 0.1, `TerrainParticle` 1.0).
    Jitter { keep: f32, add: f32 },
    /// `RisingParticle` (flames): `jittered * keep + speed`, and the spawn position jittered
    /// by ±`spread` per axis.
    Rising { keep: f32, spread: f32 },
    /// The packet speed alone, plus a ±`jitter` random offset per axis (`BubbleParticle`
    /// 0.02, `ExplodeParticle` 0.05, `BreakingItemParticle` 0).
    Scaled { keep: f32, jitter: f32 },
    /// The packet speed verbatim (`PortalParticle`).
    Keep,
    /// No velocity at all (`HugeExplosionParticle`).
    None,
    /// `WaterDropParticle` (rain): x/z speed 0.3x, y speed `r * 0.2 + 0.1`. `SplashParticle`
    /// (`splash` set) replaces that with the packet's x/z speed and a fixed 0.1 on y when it
    /// has no y speed of its own.
    Drop { splash: bool },
    /// The jittered velocity damped to `keep`, with a fixed `lift` on y: `HeartParticle`
    /// (0.01, +0.1) and `NoteParticle` (0.01, +0.2).
    Damped { keep: f32, lift: f32 },
    /// `SpellParticle` (`entity_effect`): the x and z speeds the caller asked for are
    /// replaced by a ±0.5 drift (y is kept) before the speed constructor jitters them, y is
    /// then damped to 20 %, and a zero x and z packet speed damps the drift to 10 %.
    Spell,
}
