//! The world border (vanilla `WorldBorder`): the client's copy of the server's border packets.
//! The interaction step gates its packets on the bounds (`MultiPlayerGameMode.useItemOn`
//! fails without a packet outside the border, `startDestroyBlock` returns false,
//! `Minecraft.startUseItem`'s ENTITY case returns before the interact packet), the renderer
//! draws the forcefield wall from the bounds and the status (`WorldBorderRenderer`), and the
//! HUD's vignette warns from the distance and the warning settings (`Hud.extractVignette`).
//!
//! The diameter lerps like vanilla's `MovingBorderExtent`: the lerp's length is in game
//! ticks (the packets' `speed`), and the time it has run is measured on the wall clock at
//! 20 ticks a second. Before `initialize_border` arrives the border is vanilla's default:
//! centre (0, 0), a diameter of 59 999 970 blocks, warning 15 ticks and 5 blocks, which no
//! interaction ever crosses.

use haste_core::BlockPos;

/// Vanilla `WorldBorder.absoluteMaxSize`: the bounds are clamped to it.
const ABSOLUTE_MAX_SIZE: f64 = 29_999_984.0;
/// Vanilla `SharedConstants.TICKS_PER_SECOND`.
const TICKS_PER_SECOND: f64 = 20.0;
/// Vanilla `WorldBorder.Settings.DEFAULT`'s size.
const DEFAULT_DIAMETER: f64 = 59_999_970.0;
/// Vanilla `WorldBorder`'s default warning time (ticks) and distance (blocks).
const DEFAULT_WARNING_TIME: i32 = 15;
const DEFAULT_WARNING_BLOCKS: i32 = 5;

/// Vanilla `BorderStatus`: the border is growing, shrinking or still (the wall's tint).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderStatus {
    Growing,
    Shrinking,
    Stationary,
}

impl BorderStatus {
    /// Vanilla `BorderStatus.getColor`: the wall's RGB (0xRRGGBB).
    pub fn color(self) -> u32 {
        match self {
            BorderStatus::Growing => 4_259_712,
            BorderStatus::Shrinking => 16_724_016,
            BorderStatus::Stationary => 2_138_367,
        }
    }
}

/// The world border's centre, lerping diameter and warning settings.
#[derive(Debug)]
pub struct WorldBorder {
    center: (f64, f64),
    /// The diameter's lerp: from, to, when it started, and how many ticks it takes.
    from: f64,
    to: f64,
    started_at: std::time::Instant,
    ticks: f64,
    warning_time: i32,
    warning_blocks: i32,
}

impl Default for WorldBorder {
    fn default() -> Self {
        WorldBorder {
            center: (0.0, 0.0),
            from: DEFAULT_DIAMETER,
            to: DEFAULT_DIAMETER,
            started_at: std::time::Instant::now(),
            ticks: 0.0,
            warning_time: DEFAULT_WARNING_TIME,
            warning_blocks: DEFAULT_WARNING_BLOCKS,
        }
    }
}

impl WorldBorder {
    /// `minecraft:initialize_border` (also the fresh border after a respawn or dimension
    /// change, until it arrives): the centre, the diameter lerping from `old` to `new` over
    /// `speed` ticks.
    pub fn initialize(&mut self, x: f64, z: f64, old: f64, new: f64, speed: i64) {
        self.center = (x, z);
        self.lerp_size(old, new, speed);
    }

    /// `minecraft:set_border_center`.
    pub fn set_center(&mut self, x: f64, z: f64) {
        self.center = (x, z);
    }

    /// `minecraft:set_border_lerp_size`: the diameter lerps from `old` to `new` over `speed`
    /// ticks (vanilla `lerpSizeBetween`; equal sizes make a still border).
    pub fn set_lerp_size(&mut self, old: f64, new: f64, speed: i64) {
        self.lerp_size(old, new, speed);
    }

    fn lerp_size(&mut self, from: f64, to: f64, ticks: i64) {
        self.from = from;
        self.to = to;
        self.started_at = std::time::Instant::now();
        self.ticks = ticks as f64;
    }

    /// `minecraft:set_border_size`: the diameter jumps at once.
    pub fn set_size(&mut self, diameter: f64) {
        self.lerp_size(diameter, diameter, 0);
    }

    /// `minecraft:set_border_warning_delay` (`WorldBorder.setWarningTime`), in ticks.
    pub fn set_warning_time(&mut self, ticks: i32) {
        self.warning_time = ticks;
    }

    /// `minecraft:set_border_warning_distance` (`WorldBorder.setWarningBlocks`).
    pub fn set_warning_blocks(&mut self, blocks: i32) {
        self.warning_blocks = blocks;
    }

    /// The diameter now (`WorldBorder.getSize`, lerped).
    pub fn diameter(&self) -> f64 {
        if self.from == self.to || self.ticks <= 0.0 {
            return self.to;
        }
        let elapsed = self.started_at.elapsed().as_secs_f64() * TICKS_PER_SECOND;
        let progress = (elapsed / self.ticks).clamp(0.0, 1.0);
        self.from + (self.to - self.from) * progress
    }

    /// The diameter the lerp ends at (`WorldBorder.getLerpTarget`).
    pub fn lerp_target(&self) -> f64 {
        self.to
    }

    /// Blocks per tick the diameter moves (vanilla `MovingBorderExtent.getLerpSpeed`; 0
    /// when still).
    pub fn lerp_speed(&self) -> f64 {
        if self.from == self.to || self.ticks <= 0.0 {
            return 0.0;
        }
        (self.from - self.to).abs() / self.ticks
    }

    /// The wall's status (vanilla `MovingBorderExtent.getStatus`: shrinking when the lerp
    /// ends below where it starts, growing otherwise; a still border is stationary).
    pub fn status(&self) -> BorderStatus {
        if self.from == self.to {
            BorderStatus::Stationary
        } else if self.to < self.from {
            BorderStatus::Shrinking
        } else {
            BorderStatus::Growing
        }
    }

    /// The bounds' edges (`WorldBorder.getMinX` … `getMaxZ`): the square around the centre,
    /// clamped to vanilla's absolute maximum.
    pub fn min_x(&self) -> f64 {
        self.edge(self.center.0 - self.diameter() / 2.0)
    }

    pub fn max_x(&self) -> f64 {
        self.edge(self.center.0 + self.diameter() / 2.0)
    }

    pub fn min_z(&self) -> f64 {
        self.edge(self.center.1 - self.diameter() / 2.0)
    }

    pub fn max_z(&self) -> f64 {
        self.edge(self.center.1 + self.diameter() / 2.0)
    }

    fn edge(&self, v: f64) -> f64 {
        v.clamp(-ABSOLUTE_MAX_SIZE, ABSOLUTE_MAX_SIZE)
    }

    /// `WorldBorder.getDistanceToBorder(x, z)`: the distance to the nearest edge (negative
    /// outside the border).
    pub fn distance_to_border(&self, x: f64, z: f64) -> f64 {
        let from_north = z - self.min_z();
        let from_south = self.max_z() - z;
        let from_west = x - self.min_x();
        let from_east = self.max_x() - x;
        from_west.min(from_east).min(from_north).min(from_south)
    }

    /// `Hud.extractVignette`'s warning distance: the warning blocks, or further out when
    /// the border moves faster than the warning time allows.
    pub fn warning_distance(&self) -> f64 {
        let moving = (self.lerp_speed() * self.warning_time as f64)
            .min((self.lerp_target() - self.diameter()).abs());
        (self.warning_blocks as f64).max(moving)
    }

    /// The vignette's warning strength at (x, z): 0 outside the warning distance, rising
    /// to 1 at the edge (`Hud.extractVignette`).
    pub fn warning_strength(&self, x: f64, z: f64) -> f32 {
        let dist = self.distance_to_border(x, z);
        let warning = self.warning_distance();
        if dist < warning {
            (1.0 - dist / warning).clamp(0.0, 1.0) as f32
        } else {
            0.0
        }
    }

    /// `WorldBorder.isWithinBounds(x, z)`: the half-open square.
    pub fn is_within_bounds_xz(&self, x: f64, z: f64) -> bool {
        x >= self.min_x() && x < self.max_x() && z >= self.min_z() && z < self.max_z()
    }

    /// `WorldBorder.isWithinBounds(BlockPos)`.
    pub fn is_within_bounds(&self, pos: BlockPos) -> bool {
        self.is_within_bounds_xz(pos.x as f64, pos.z as f64)
    }
}

#[cfg(test)]
mod tests;
