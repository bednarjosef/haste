//! The wait for the level after a login or respawn (vanilla `LevelLoadTracker` and the
//! `hasClientLoaded` check at the top of `LocalPlayer.tick`): while the chunk at the
//! player's position has not streamed in yet, the player's tick is skipped entirely, so
//! it can never fall through a world that is still loading. The wait starts at the login
//! and at every respawn (vanilla `startWaitingForNewLevel`); a same-dimension teleport
//! does not restart it — there vanilla instead keeps ticking and pins the vertical
//! movement (`LivingEntity.travelInAir`, the caller's unloaded-chunk step).
//!
//! Vanilla waits for the player's *compiled render section* (the loading screen wants
//! the world visible); we wait for the chunk at the player's feet, which is what the
//! movement tick needs. The rest is vanilla's rule: spectators, the dead and players
//! outside the build height are let in at once, and after 30 s everyone is
//! (`LevelLoadTracker.CLIENT_WAIT_TIMEOUT_MS`, "letting the player into the world
//! anyway").

use glam::DVec3;
use haste_core::game::GameMode;
use haste_core::view::BlockView;
use haste_core::BlockPos;
use haste_protocol::packets::play::serverbound as sb;
use haste_protocol::ServerboundPlay;

use super::LocalPlayer;

/// Vanilla `LevelLoadTracker.CLIENT_WAIT_TIMEOUT_MS` (30 s) in ticks.
pub const CLIENT_WAIT_TIMEOUT_TICKS: u32 = 600;

/// One wait, from the login or the respawn that started it.
#[derive(Debug)]
pub struct LevelLoad {
    /// Set once the level is ready; only the next login/respawn starts a new wait.
    ready: bool,
    /// Ticks since the login or respawn (vanilla counts milliseconds; the client ticks
    /// 20 times a second).
    age: u32,
}

impl LevelLoad {
    /// The wait a login or respawn starts.
    pub fn start() -> Self {
        Self {
            ready: false,
            age: 0,
        }
    }

    /// A tick of age for the running wait (the timeout counts from the login/respawn,
    /// like vanilla's `startClientLoad` deadline).
    pub fn tick(&mut self) {
        self.age = self.age.saturating_add(1);
    }

    /// Whether the level is ready for the player at `pos` (vanilla
    /// `LevelLoadTracker.WaitingForPlayerChunk.isReady`): the chunk at its position has
    /// arrived, or the player is a spectator, dead or outside the build height, or the
    /// timeout has passed. Latches: once ready, it stays ready until the next
    /// login/respawn.
    pub fn is_ready(
        &mut self,
        pos: DVec3,
        spectator: bool,
        alive: bool,
        world: &dyn BlockView,
    ) -> bool {
        if self.ready {
            return true;
        }
        let y = f64::from(world.min_y());
        let outside_build_height = pos.y <= y || pos.y >= y + f64::from(world.height());
        let timed_out = self.age >= CLIENT_WAIT_TIMEOUT_TICKS;
        self.ready = timed_out
            || spectator
            || !alive
            || outside_build_height
            || world.is_chunk_loaded(BlockPos::from_dvec3(pos).chunk());
        if self.ready && timed_out {
            tracing::warn!(
                "timed out waiting for the chunk at the player; ticking it anyway \
                 (vanilla LevelLoadTracker lets the player into the world)"
            );
        }
        self.ready
    }
}

impl LocalPlayer {
    /// The level-load gate at the top of the player's tick: vanilla
    /// `ClientPacketListener.tick` ticks the tracker and, once the level is ready,
    /// sends `player_loaded` (`notifyPlayerLoaded`) — which is also what lets
    /// `LocalPlayer.tick` run the player at all (`hasClientLoaded`). Returns whether
    /// this tick may run the player; while it may not, only `client_tick_end` leaves
    /// the client, so a player whose chunks are still streaming in after the login or
    /// a respawn never moves, let alone falls. A teleport whose accept has not gone out
    /// yet holds the tick too (vanilla answers it inside the teleport's own tick,
    /// `handleMovePlayer`, so no `sendChanges` can name the new position before it).
    pub(super) fn level_ready_gate(
        &mut self,
        world: &dyn BlockView,
        out: &mut Vec<ServerboundPlay>,
    ) -> bool {
        self.level_load.tick();
        // While a teleport's accept is still held, so is `player_loaded`: vanilla answers
        // the teleport inside its own tick, before anything else can leave.
        if self.pending_accept.is_some() {
            return false;
        }
        if self.has_position
            && !self.loaded
            && self.level_load.is_ready(
                self.state.pos,
                self.game_mode == GameMode::Spectator,
                self.health > 0.0,
                world,
            )
        {
            out.push(ServerboundPlay::PlayerLoaded(sb::PlayerLoaded {}));
            self.loaded = true;
        }
        self.has_position && self.loaded
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use haste_core::{BlockStateId, ChunkPos};
    use haste_physics::testing::{states, TestWorld};

    /// A flat world that reports only one chunk as loaded.
    struct OneChunk {
        floor: TestWorld,
        loaded: ChunkPos,
    }

    impl BlockView for OneChunk {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            self.floor.block_state(pos)
        }
        fn is_chunk_loaded(&self, pos: ChunkPos) -> bool {
            pos == self.loaded
        }
        fn sky_light(&self, pos: BlockPos) -> u8 {
            self.floor.sky_light(pos)
        }
        fn block_light(&self, pos: BlockPos) -> u8 {
            self.floor.block_light(pos)
        }
        fn min_y(&self) -> i32 {
            self.floor.min_y()
        }
        fn height(&self) -> i32 {
            self.floor.height()
        }
    }

    fn world() -> OneChunk {
        OneChunk {
            floor: TestWorld::flat(0, states::STONE),
            loaded: ChunkPos::new(0, 0),
        }
    }

    /// A position whose chunk is not the loaded one.
    fn beside() -> DVec3 {
        DVec3::new(16.5, 0.0, 0.5)
    }

    #[test]
    fn ready_once_the_players_chunk_is_loaded() {
        let mut l = LevelLoad::start();
        let w = world();
        assert!(!l.is_ready(beside(), false, true, &w));
        assert!(l.is_ready(DVec3::new(0.5, 0.0, 0.5), false, true, &w));
        // Latched: stays ready even where no chunk is loaded.
        assert!(l.is_ready(beside(), false, true, &w));
    }

    #[test]
    fn spectators_dead_and_outside_build_height_are_let_in_at_once() {
        let w = world();
        let mut l = LevelLoad::start();
        assert!(l.is_ready(beside(), true, true, &w), "spectator");
        let mut l = LevelLoad::start();
        assert!(l.is_ready(beside(), false, false, &w), "dead");
        let mut l = LevelLoad::start();
        assert!(
            l.is_ready(
                DVec3::new(0.5, f64::from(w.min_y() + w.height()), 0.5),
                false,
                true,
                &w
            ),
            "above the build height"
        );
        let mut l = LevelLoad::start();
        assert!(
            l.is_ready(
                DVec3::new(0.5, f64::from(w.min_y() - 1), 0.5),
                false,
                true,
                &w
            ),
            "below the build height"
        );
    }

    #[test]
    fn the_timeout_lets_the_player_in_anyway() {
        let mut l = LevelLoad::start();
        let w = world();
        for _ in 0..CLIENT_WAIT_TIMEOUT_TICKS - 1 {
            l.tick();
            assert!(!l.is_ready(beside(), false, true, &w));
        }
        l.tick();
        assert!(l.is_ready(beside(), false, true, &w));
        // And it stays ready.
        assert!(l.is_ready(beside(), false, true, &w));
    }
}
