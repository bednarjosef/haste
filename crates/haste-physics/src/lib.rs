//! Vanilla-exact player movement and collision, validated against recorded traces. Owner: WS5.
//!
//! See docs/INTERFACES.md for the contract this crate implements and AGENTS.md for the rules.
//! Every constant, where it comes from and how sure we are of it is listed in `NOTES.md`.
//!
//! Layout: [`collision`] (pure AABB sweep + step-up), [`movement`] (the tick), [`fluids`] (water
//! and lava), [`flying`] (creative flight), [`attributes`], [`replay`] (trace replay harness),
//! [`oracle`] (interim loaders for the oracle dumps), [`testing`] (in-memory world for tests).

pub mod attributes;
pub mod collision;
pub mod flying;
pub mod fluids;
pub mod movement;
pub mod oracle;
pub mod replay;
pub mod testing;
mod trig;

use haste_core::{Aabb, BlockStateId, DVec3};
use serde::{Deserialize, Serialize};

pub use attributes::{PlayerAttributes, Pose};
pub use collision::{collide, collide_with_step};
pub use movement::tick;

/// The local player's physics state. Positions/velocities are f64 like vanilla; angles f32.
#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct PlayerState {
    /// Feet position (bottom centre of the box).
    pub pos: DVec3,
    /// Blocks per tick.
    pub velocity: DVec3,
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
    pub sprinting: bool,
    pub sneaking: bool,
    pub in_water: bool,
    pub flying: bool,
    pub fall_distance: f32,
    pub in_lava: bool,
    /// Sprint-swimming (uses the swimming pose).
    pub swimming: bool,
    pub pose: Pose,
    /// Ticks until another jump is allowed while jump is held (wiki "Jumping": half a second).
    pub jump_cooldown: u8,
    pub horizontal_collision: bool,
    pub vertical_collision: bool,
}

impl PlayerState {
    pub fn at(pos: DVec3) -> Self {
        Self { pos, ..Default::default() }
    }
    /// Current collision box for this state's pose.
    pub fn aabb(&self, attrs: &PlayerAttributes) -> Aabb {
        Aabb::entity(self.pos, attrs.width as f64, attrs.height_for(self.pose))
    }
}

/// What changed in a tick, for the caller's movement-packet logic.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct TickResult {
    pub moved: bool,
    pub rotated: bool,
    pub on_ground_changed: bool,
    pub collided_horizontally: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum FluidKind {
    Water,
    Lava,
}

/// Fluid in a block state. `height` is the surface height inside the cell when nothing of the same
/// fluid is above (wiki "Water": level/9 rule, source = 8/9); the tick raises it to 1.0 itself
/// when the same fluid is directly above.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FluidInfo {
    pub kind: FluidKind,
    pub height: f32,
}

/// Block-dependent movement properties. Stand-in for `GameData` (not available yet; see the
/// interface request in NOTES.md). Defaults are vanilla's defaults from block_properties.json.
pub trait BlockPhysics {
    /// Slipperiness (block_properties.json `friction`).
    fn friction(&self, _state: BlockStateId) -> f32 {
        0.6
    }
    fn speed_factor(&self, _state: BlockStateId) -> f32 {
        1.0
    }
    fn jump_factor(&self, _state: BlockStateId) -> f32 {
        1.0
    }
    fn fluid(&self, _state: BlockStateId) -> Option<FluidInfo> {
        None
    }
}

/// Every block behaves like stone; no fluids.
pub struct DefaultBlockPhysics;
impl BlockPhysics for DefaultBlockPhysics {}
