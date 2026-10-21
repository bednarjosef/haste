//! A per-tick value the renderer interpolates by partial tick.

/// A per-tick value the renderer lerps by partial tick, the way a vanilla render state
/// extracts one (`Mth.lerp(partialTicks, xO, x)`): the value at the start and at the end of the
/// last game tick. The client owns the tick, the renderer only interpolates it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TickPair {
    /// The value at the end of the previous tick (vanilla's `oX` field).
    pub start: f32,
    /// The value at the end of the last tick.
    pub end: f32,
}

impl TickPair {
    /// A value that never changes.
    pub const fn fixed(v: f32) -> TickPair {
        TickPair { start: v, end: v }
    }

    /// `Mth.lerp(partial, start, end)` at `partial` (0..1) through the tick.
    pub fn at(self, partial: f32) -> f32 {
        self.start + (self.end - self.start) * partial
    }
}
