//! The unloaded-chunk step of vanilla `LivingEntity.travelInAir` for the local player:
//! when the chunk at the player's position is not loaded, gravity is skipped and the
//! vertical movement is pinned instead — a same-dimension teleport into chunks that
//! are still streaming in sinks slowly instead of falling through the world. (`Player`
//! calls `travel` even while flying, then keeps the flying player's own vertical
//! movement, so the step is skipped for a flying player like vanilla's is.)

use haste_core::view::BlockView;

use super::LocalPlayer;

/// Vertical movement while the chunk at the player is not loaded: vanilla
/// `LivingEntity.travelInAir` pins the vertical movement to -0.1 (0.0 at or below the
/// world's minimum), with the 0.98 vertical drag applied on top.
pub const UNLOADED_CHUNK_FALL_SPEED: f64 = -0.1 * haste_physics::movement::VERTICAL_DRAG as f64;

impl LocalPlayer {
    /// One tick of [`UNLOADED_CHUNK_FALL_SPEED`] travel with the chunk at the player
    /// not loaded. The pin is the velocity for the NEXT tick — vanilla moves by the
    /// previous tick's velocity first (the move inside
    /// `handleRelativeFrictionAndCalculateMovement`, collision-free in an unloaded
    /// chunk), so a teleport that zeroed the velocity does not move on its first tick,
    /// and every later tick sinks `UNLOADED_CHUNK_FALL_SPEED`. Like every downward
    /// move, the sink accumulates fall distance (`Entity.checkFallDamage` inside the
    /// move) and leaves the player off the ground.
    pub(super) fn unloaded_chunk_step(&mut self, world: &dyn BlockView) {
        if self.state.flying {
            return;
        }
        let dy = self.state.velocity.y;
        if dy < 0.0 {
            self.state.pos.y += dy;
            self.state.fall_distance -= dy as f32;
            self.state.on_ground = false;
        }
        let falling = self.state.pos.y > f64::from(world.min_y());
        self.state.velocity.y = if falling {
            UNLOADED_CHUNK_FALL_SPEED
        } else {
            0.0
        };
    }
}
