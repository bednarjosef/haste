//! Vanilla-exact player movement and collision, validated against recorded traces. Owner: physics.
//!
//! See docs/INTERFACES.md for the contract this crate implements and AGENTS.md for the rules.
//! Every constant, where it comes from and how sure we are of it is listed in `NOTES.md`.
//!
//! Layout: [`collision`] (pure AABB sweep + step-up), [`movement`] (the tick), [`fluids`] (water
//! and lava), [`flying`] (creative flight), [`elytra`] (gliding), [`attributes`], [`replay`]
//! (trace replay harness), [`testing`] (in-memory world for tests).

pub mod attributes;
pub mod collision;
pub mod elytra;
pub mod fluids;
pub mod flying;
pub mod mining;
pub mod movement;
pub mod raycast;
pub mod replay;
pub mod testing;
mod trig;

use haste_core::{Aabb, BlockStateId, DVec3};
use serde::{Deserialize, Serialize};

pub use attributes::{PlayerAttributes, Pose};
pub use collision::{collide, collide_with_step};
pub use elytra::{effective_gravity, glide_velocity, look_angle};
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
    /// Shift is held and the crouching box fits, or standing does not (vanilla
    /// `LocalPlayer.aiStep`'s `crouching` field, `LocalPlayer.isCrouching`): slows the input.
    pub crouching: bool,
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
    /// Eyes under the water surface (last tick); slows mining, starts sprint-swimming.
    pub eyes_in_water: bool,
    /// Gliding with an elytra (vanilla's shared flag 7). While set the player uses the
    /// fall-flying pose box and `elytra::travel` instead of the normal walk/fall.
    pub fall_flying: bool,
}

impl PlayerState {
    pub fn at(pos: DVec3) -> Self {
        Self {
            pos,
            ..Default::default()
        }
    }
    /// Current collision box for this state's pose. While gliding the box is the fall-flying
    /// pose's 0.6 x 0.6 (vanilla's fall_flying dimensions), not the pose's.
    pub fn aabb(&self, attrs: &PlayerAttributes) -> Aabb {
        if self.fall_flying {
            let height = elytra::GLIDER_BOX_HEIGHT * attrs.scale;
            Aabb::entity(self.pos, attrs.box_width(), height as f64)
        } else {
            Aabb::entity(self.pos, attrs.box_width(), attrs.height_for(self.pose))
        }
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

/// Block-dependent properties come through `haste_core::view::BlockPhysics` (implemented by
/// haste-data's `GameData`); re-exported here for convenience.
pub use haste_core::view::{BlockPhysics, FluidInfo, FluidKind};

/// Every block behaves like stone (vanilla defaults: friction 0.6, factors 1.0); no fluids.
pub struct DefaultBlockPhysics;
impl BlockPhysics for DefaultBlockPhysics {
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
    fn light_emission(&self, _state: BlockStateId) -> u8 {
        0
    }
    fn is_air(&self, state: BlockStateId) -> bool {
        state == BlockStateId::AIR
    }
    fn is_opaque_full(&self, state: BlockStateId) -> bool {
        state != BlockStateId::AIR
    }
}
