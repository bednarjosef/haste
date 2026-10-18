//! The world border (vanilla `WorldBorder`): the client's copy of the server's
//! `initialize_border` / `set_border_*` packets, and the bounds test the interaction step
//! gates its packets on (`MultiPlayerGameMode.useItemOn` fails without a packet outside the
//! border, `startDestroyBlock` returns false, `Minecraft.startUseItem`'s ENTITY case returns
//! before the interact packet).
//!
//! Only what the gates read is kept: the centre and the (lerping) diameter. The warning
//! distance and time are not modelled yet. Before `initialize_border` arrives the border is
//! vanilla's default — centre (0, 0), a diameter of 59 999 970 blocks — which no interaction
//! ever crosses.

/// The world border's centre and lerping diameter.
#[derive(Debug)]
pub struct WorldBorder {
    center: (f64, f64),
    /// The diameter's lerp: from, to, when it started, over how many milliseconds.
    from: f64,
    to: f64,
    started_at: std::time::Instant,
    millis: f64,
}

/// Vanilla `WorldBorder.Settings.DEFAULT`'s size.
const DEFAULT_DIAMETER: f64 = 59_999_970.0;

impl Default for WorldBorder {
    fn default() -> Self {
        WorldBorder {
            center: (0.0, 0.0),
            from: DEFAULT_DIAMETER,
            to: DEFAULT_DIAMETER,
            started_at: std::time::Instant::now(),
            millis: 0.0,
        }
    }
}

impl WorldBorder {
    /// `minecraft:initialize_border` (also the fresh border after a respawn or dimension
    /// change, until it arrives): the centre, the diameter lerping from `old` to `new` over
    /// `speed` milliseconds.
    pub fn initialize(&mut self, x: f64, z: f64, old: f64, new: f64, speed: i64) {
        self.center = (x, z);
        self.from = old;
        self.to = new;
        self.started_at = std::time::Instant::now();
        self.millis = speed as f64;
    }

    /// `minecraft:set_border_center`.
    pub fn set_center(&mut self, x: f64, z: f64) {
        self.center = (x, z);
    }

    /// `minecraft:set_border_lerp_size`: the diameter lerps from `old` to `new` over `speed`
    /// milliseconds.
    pub fn set_lerp_size(&mut self, old: f64, new: f64, speed: i64) {
        self.from = old;
        self.to = new;
        self.started_at = std::time::Instant::now();
        self.millis = speed as f64;
    }

    /// `minecraft:set_border_size`: the diameter jumps at once.
    pub fn set_size(&mut self, diameter: f64) {
        self.from = diameter;
        self.to = diameter;
        self.millis = 0.0;
    }

    /// The diameter now (`WorldBorder.getSize`, lerped).
    fn diameter(&self) -> f64 {
        if self.millis <= 0.0 {
            return self.to;
        }
        let t = (self.started_at.elapsed().as_secs_f64() * 1000.0 / self.millis).clamp(0.0, 1.0);
        self.from + (self.to - self.from) * t
    }

    /// `WorldBorder.isWithinBounds(x, z)`: the half-open square around the centre.
    pub fn is_within_bounds_xz(&self, x: f64, z: f64) -> bool {
        let half = self.diameter() / 2.0;
        let (cx, cz) = self.center;
        x >= cx - half && x < cx + half && z >= cz - half && z < cz + half
    }

    /// `WorldBorder.isWithinBounds(BlockPos)`.
    pub fn is_within_bounds(&self, pos: haste_core::BlockPos) -> bool {
        self.is_within_bounds_xz(pos.x as f64, pos.z as f64)
    }
}

#[cfg(test)]
mod tests;
