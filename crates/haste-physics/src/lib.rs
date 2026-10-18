//! Vanilla-exact player movement and collision, validated against recorded traces. Owner: physics.
//!
//! See docs/INTERFACES.md for the contract this crate implements and AGENTS.md for the rules.
//! Every constant, where it comes from and how sure we are of it is listed in `NOTES.md`.
//!
//! Layout: [`collision`] (pure AABB sweep + step-up), [`movement`] (the tick), [`climbing`]
//! (ladders and vines), [`fluids`] (water and lava), [`flying`] (creative flight), [`elytra`] (gliding), [`attributes`], [`replay`]
//! (trace replay harness), [`testing`] (in-memory world for tests).

pub mod attributes;
pub mod block_effects;
pub mod climbing;
pub mod collision;
pub mod elytra;
pub mod fluids;
pub mod flying;
pub mod mining;
pub mod movement;
pub mod raycast;
pub mod replay;
mod sprint;
pub mod suffocation;
pub mod testing;
mod trig;

use haste_core::{Aabb, BlockStateId, DVec3};
use serde::{Deserialize, Serialize};

pub use attributes::{PlayerAttributes, Pose};
pub use climbing::{on_climbable, ClimbableBlocks, NoClimbables};
pub use collision::{collide, collide_with_step};
pub use elytra::{effective_gravity, glide_velocity, look_angle};
pub use movement::{tick, tick_with};
pub use sprint::can_start_sprinting;
pub use suffocation::move_towards_closest_space;

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
    /// Last tick's collision was minor (input and movement within 8 degrees;
    /// `LocalPlayer.isHorizontalCollisionMinor`). Keeps sprinting.
    #[serde(default)]
    pub minor_horizontal_collision: bool,
    /// Spectator game mode: landing does not end creative flight (`LocalPlayer.aiStep`).
    #[serde(default)]
    pub spectating: bool,
    /// Eyes under the water surface (last tick); slows mining, starts sprint-swimming.
    pub eyes_in_water: bool,
    /// Stuck-in-block speed multiplier set by `makeStuckInBlock` (cobweb, powder snow, sweet
    /// berry bush) at the end of the tick; consumed by the next move (vanilla `Entity.move`).
    pub stuck_multiplier: DVec3,
    /// Gliding with an elytra (vanilla's shared flag 7). While set the player uses the
    /// fall-flying pose box and `elytra::travel` instead of the normal walk/fall.
    pub fall_flying: bool,
    /// Spectator mode (vanilla `Entity.noPhysics`, set from `Player.tick` when
    /// `isSpectator()`): movement adds the motion without any collision, so the player walks
    /// through blocks.
    #[serde(default)]
    pub no_physics: bool,
    /// Levitation effect amplifier (vanilla `getEffect(MobEffects.LEVITATION)`; `None` without).
    /// `LivingEntity.travelInAir` blends the rise towards `0.05 * (amplifier + 1)` per tick and
    /// skips gravity, whatever the amplifier (a negative one makes the player hover or sink).
    #[serde(default)]
    pub levitation: Option<i32>,
    /// Slow falling effect on us (vanilla `LivingEntity.hasEffect(MobEffects.SLOW_FALLING)`):
    /// `getEffectiveGravity` caps the gravity at 0.01 while falling and the fall distance is
    /// reset each tick.
    #[serde(default)]
    pub slow_falling: bool,
    /// Jump boost effect amplifier (vanilla `getEffect(MobEffects.JUMP_BOOST)`; `None` without).
    /// `LivingEntity.getJumpBoostPower` adds `0.1F * (amplifier + 1)` to the jump power.
    #[serde(default)]
    pub jump_boost: Option<i32>,
    /// Sprinting impossible whatever the water (vanilla `LocalPlayer.isSprintingPossible`'s
    /// first terms failing: blind (`Player.isMobilityRestricted`), or not enough food and no
    /// may-fly ability). Set by the client each tick; stops a sprint and keeps it from starting.
    #[serde(default)]
    pub sprint_blocked: bool,
    /// Slowed by the item in use (vanilla `LocalPlayer.isSlowDueToUsingItem`): a sprint cannot
    /// start. Set by the client each tick.
    #[serde(default)]
    pub slow_using_item: bool,
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
    fn water_flow(
        &self,
        _world: &dyn haste_core::view::BlockView,
        _pos: haste_core::BlockPos,
        _state: BlockStateId,
    ) -> Option<DVec3> {
        None
    }
}
