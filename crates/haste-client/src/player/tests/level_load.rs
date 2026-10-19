//! The unloaded-chunk travel step and the level-load gate: what keeps a player whose
//! chunks are still streaming in (after a login, a respawn or a far teleport) from
//! falling through the world. Vanilla `LivingEntity.travelInAir`'s pin for the
//! teleport case, `LocalPlayer.tick`'s `hasClientLoaded` hold for the login and respawn
//! cases (see `player/level_load.rs`).

use super::*;

/// Vanilla `LivingEntity.travelInAir`: with the chunk below not loaded the client skips gravity
/// and pins the vertical movement to -0.1 (the 0.98 vertical drag on top of it) while the player
/// is above the world's minimum; at or below the minimum it is 0.0. The pin is the velocity for
/// the next tick — the move of each tick goes by the previous velocity first (the move inside
/// `handleRelativeFrictionAndCalculateMovement`), so a teleport that zeroed the velocity does
/// not move on its first tick — and like every downward move the sink accumulates fall distance
/// and leaves the player off the ground.
#[test]
fn unloaded_chunk_below_falls_at_vanillas_rate() {
    assert!((UNLOADED_CHUNK_FALL_SPEED + 0.098).abs() < 1.0e-8);
    let (mut p, _) = spawned();
    // One tick to settle on the floor after the spawn teleport.
    tick(&mut p, &InputState::default());
    assert!(p.state.on_ground);
    let resting = p.state.velocity.y;
    let w = Unloaded::new(-64);
    let mut out = Vec::new();
    p.tick_with(
        &InputState::default(),
        &w,
        &TestShapes::default(),
        &TestShapes::default(),
        None,
        &mut out,
    );
    assert_eq!(p.state.velocity.y, UNLOADED_CHUNK_FALL_SPEED);
    assert!(!p.state.on_ground);
    // The first unloaded tick moves by the resting velocity, like vanilla's move would.
    assert!((p.state.pos.y - resting).abs() < 1.0e-9);
    assert!((p.state.fall_distance + resting as f32).abs() < 1.0e-6);
    // Gravity stays skipped: the same step every tick, until the chunk is loaded again.
    for _ in 0..3 {
        p.tick_with(
            &InputState::default(),
            &w,
            &TestShapes::default(),
            &TestShapes::default(),
            None,
            &mut out,
        );
    }
    assert!((p.state.pos.y - (resting + 3.0 * UNLOADED_CHUNK_FALL_SPEED)).abs() < 1.0e-12);
    assert!(
        (p.state.fall_distance + (resting + 3.0 * UNLOADED_CHUNK_FALL_SPEED) as f32).abs() < 1.0e-4
    );

    // At or below the world's minimum the vertical movement is 0.0: the player stays put.
    let (mut p, mut out) = spawned();
    assert!(p.handle(&teleport(2, DVec3::new(0.5, -65.0, 0.5), 0.0, 0), &mut out));
    p.tick_with(
        &InputState::default(),
        &w,
        &TestShapes::default(),
        &TestShapes::default(),
        None,
        &mut out,
    );
    assert_eq!(p.state.velocity.y, 0.0);
    assert_eq!(p.state.pos.y, -65.0);
}

/// Vanilla `LocalPlayer.tick` holds the whole tick while `hasClientLoaded` is false: after a
/// respawn whose chunks have not streamed in yet the player does not sink (`travelInAir`'s pin
/// never runs), nothing but `client_tick_end` leaves the client, and `player_loaded` goes out
/// in the tick the chunk arrives, with the movement following it.
#[test]
fn respawn_holds_the_tick_until_the_chunk_at_the_player_arrives() {
    let (mut p, mut out) = spawned();
    let mut w = UnloadedOrLoaded::default();
    let spawn = haste_protocol::packets::play::clientbound::SpawnInfo {
        dimension_type: 0,
        dimension: Identifier::minecraft("overworld"),
        hashed_seed: 0,
        game_mode: GameMode::Survival as i32,
        previous_game_mode: 0,
        is_debug: false,
        is_flat: true,
        death_location: None,
        portal_cooldown: 0,
        sea_level: 63,
    };
    assert!(p.handle(
        &ClientboundPlay::Respawn(cb::Respawn {
            spawn,
            data_kept: 0
        }),
        &mut out,
    ));
    assert!(p.handle(&teleport(2, DVec3::new(0.5, 80.0, 0.5), 0.0, 0), &mut out));
    // The accept waits one full tick; the level is not ready yet either way.
    assert_eq!(names(&tick_world(&mut p, &w)), ["tick_end"]);
    assert_eq!(names(&tick_world(&mut p, &w)), ["accept", "tick_end"]);
    // Still held: the chunk at the player has not arrived, so vanilla's
    // `hasClientLoaded` gate keeps the player (and its sink) out of the world.
    for _ in 0..5 {
        assert_eq!(names(&tick_world(&mut p, &w)), ["tick_end"]);
        assert_eq!(p.state.pos.y, 80.0);
    }
    // The chunk arrives: `player_loaded` with it, and the first `sendPosition` carries
    // the teleport's position (xLast was untouched by it), like the join flow's first
    // move.
    w.loaded = true;
    assert_eq!(
        names(&tick_world(&mut p, &w)),
        ["loaded", "input", "pos", "tick_end"]
    );
    // The player is in the air over the floor: it now falls like vanilla.
    assert!(p.state.velocity.y < 0.0);
}

/// A world whose chunk at the player's position is loaded only when `loaded` is set.
struct UnloadedOrLoaded {
    floor: TestWorld,
    loaded: bool,
}

impl Default for UnloadedOrLoaded {
    fn default() -> Self {
        Self {
            floor: TestWorld::flat(0, states::STONE),
            loaded: false,
        }
    }
}

impl BlockView for UnloadedOrLoaded {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.floor.block_state(pos)
    }
    fn is_chunk_loaded(&self, _pos: ChunkPos) -> bool {
        self.loaded
    }
    fn sky_light(&self, pos: BlockPos) -> u8 {
        self.floor.sky_light(pos)
    }
    fn block_light(&self, pos: BlockPos) -> u8 {
        self.floor.block_light(pos)
    }
    fn min_y(&self) -> i32 {
        -64
    }
    fn height(&self) -> i32 {
        384
    }
}

/// One tick against a world the caller keeps switching (the respawn test's stream-in).
fn tick_world(p: &mut LocalPlayer, w: &dyn BlockView) -> Vec<ServerboundPlay> {
    let mut out = Vec::new();
    p.tick_with(
        &InputState::default(),
        w,
        &TestShapes::default(),
        &TestShapes::default(),
        None,
        &mut out,
    );
    out
}
