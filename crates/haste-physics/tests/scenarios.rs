//! Scenario tests for tick(): every expected number comes from a public page (cited) or is
//! derived from one; NOTES.md lists them.
use haste_core::input::InputState;
use haste_core::DVec3;
use haste_physics::testing::{states::*, TestShapes, TestWorld};
use haste_physics::{tick, PlayerAttributes, PlayerState, Pose};

struct Sim {
    world: TestWorld,
    shapes: TestShapes,
    attrs: PlayerAttributes,
    state: PlayerState,
}

impl Sim {
    fn new(world: TestWorld, pos: DVec3) -> Self {
        let shapes = TestShapes::default();
        let attrs = PlayerAttributes::default();
        let mut state = PlayerState::at(pos);
        state.on_ground = false;
        let mut s = Self {
            world,
            shapes,
            attrs,
            state,
        };
        // Settle onto whatever is below (a few idle ticks).
        s.run(&InputState::default(), 3);
        s
    }
    fn step(&mut self, input: &InputState) -> DVec3 {
        let before = self.state.pos;
        tick(
            &mut self.state,
            input,
            &self.attrs,
            &self.world,
            &self.shapes,
            &self.shapes,
        );
        self.state.pos - before
    }
    fn run(&mut self, input: &InputState, n: usize) -> DVec3 {
        let mut last = DVec3::ZERO;
        for _ in 0..n {
            last = self.step(input);
        }
        last
    }
}

fn hspeed(d: DVec3) -> f64 {
    (d.x * d.x + d.z * d.z).sqrt()
}

fn forward() -> InputState {
    InputState {
        forward: true,
        ..Default::default()
    }
}

/// m/s (= blocks per second) from blocks per tick.
fn mps(bpt: f64) -> f64 {
    bpt * 20.0
}

// ---- falling and jumping -------------------------------------------------------------------

#[test]
fn free_fall_follows_documented_recurrence_and_terminal_velocity() {
    // wiki "Transportation": v(t) = 0.98 (v(t-1) - 0.08); terminal 3.92 b/t = 78.4 m/s.
    let mut s = Sim::new(TestWorld::new(), DVec3::new(0.5, 300.0, 0.5));
    s.state = PlayerState::at(DVec3::new(0.5, 300.0, 0.5));
    let mut v = 0.0f64;
    let mut y = 300.0;
    for _ in 0..40 {
        let d = s.step(&InputState::default());
        assert!((d.y - v).abs() < 1e-12, "moved {} expected {}", d.y, v);
        y += v;
        v = (v - 0.08) * 0.98f32 as f64;
        assert!((s.state.velocity.y - v).abs() < 1e-12);
    }
    assert!((s.state.pos.y - y).abs() < 1e-9);
    s.run(&InputState::default(), 1000);
    let terminal = -s.state.velocity.y;
    // wiki "Transportation" gives 78.39992374 m/s; 0.98 as f32 gives a limit of 3.9200037 b/t.
    assert!(
        (mps(terminal) - 78.4).abs() < 0.001,
        "terminal {terminal} b/t"
    );
}

#[test]
fn fall_distance_accumulates_and_landing_is_exact() {
    let mut s = Sim::new(TestWorld::flat(0, STONE), DVec3::new(0.5, 0.0, 0.5));
    s.state.pos.y = 23.0;
    s.state.on_ground = false;
    let mut max_fall = 0.0f32;
    for _ in 0..100 {
        s.step(&InputState::default());
        max_fall = max_fall.max(s.state.fall_distance);
        if s.state.on_ground {
            break;
        }
    }
    assert!(s.state.on_ground);
    assert_eq!(s.state.pos.y, 0.0);
    assert_eq!(s.state.fall_distance, 0.0);
    // Last airborne tick had fallen almost all of the 23 blocks (the landing tick resets it).
    assert!(max_fall > 20.0 && max_fall < 23.0, "{max_fall}");
}

#[test]
fn jump_height_matches_wiki() {
    // wiki "Jumping": a jump climbs 1.2522 blocks; initial speed 0.42 (wiki "Transportation").
    let mut s = Sim::new(TestWorld::flat(0, STONE), DVec3::new(0.5, 0.0, 0.5));
    assert!(s.state.on_ground);
    let jump = InputState {
        jump: true,
        ..Default::default()
    };
    let first = s.step(&jump);
    assert!((first.y - 0.42).abs() < 1e-12);
    let mut apex: f64 = s.state.pos.y;
    let mut airborne = 1;
    while !s.state.on_ground {
        s.step(&InputState::default());
        apex = apex.max(s.state.pos.y);
        airborne += 1;
        assert!(airborne < 40);
    }
    assert!((apex - 1.2522).abs() < 1e-4, "apex {apex}");
    assert_eq!(s.state.pos.y, 0.0);
    // Observation for traces: 12 ticks from take-off to landing in this model.
    assert_eq!(airborne, 12);
}

// ---- ground speeds (wiki "Walking", "Sprinting", "Sneaking", "Transportation") ------------

fn steady_speed(floor: haste_core::BlockStateId, input: InputState) -> (f64, f64) {
    let mut s = Sim::new(TestWorld::flat(0, floor), DVec3::new(0.5, 0.0, 0.5));
    let a = hspeed(s.run(&input, 99));
    let b = hspeed(s.step(&input));
    (a, b)
}

#[test]
fn walking_speed_is_constant_after_100_ticks() {
    let (a, b) = steady_speed(STONE, forward());
    assert!((a - b).abs() < 1e-9, "not steady: {a} vs {b}");
    assert!((mps(b) - 4.317).abs() < 0.001, "walk {} m/s", mps(b));
}

#[test]
fn sprint_and_sneak_speeds() {
    let (_, sprint) = steady_speed(
        STONE,
        InputState {
            forward: true,
            sprint: true,
            ..Default::default()
        },
    );
    assert!(
        (mps(sprint) - 5.612).abs() < 0.001,
        "sprint {} m/s",
        mps(sprint)
    );
    let (_, sneak) = steady_speed(
        STONE,
        InputState {
            forward: true,
            sneak: true,
            ..Default::default()
        },
    );
    assert!(
        (mps(sneak) - 1.295).abs() < 0.001,
        "sneak {} m/s",
        mps(sneak)
    ); // "about 1.3"
    let (_, diag) = steady_speed(
        STONE,
        InputState {
            forward: true,
            left: true,
            sneak: true,
            ..Default::default()
        },
    );
    assert!(
        (mps(diag) - 1.8).abs() < 0.05,
        "diagonal sneak {} m/s",
        mps(diag)
    ); // "a faster 1.8 m/s"
}

#[test]
fn speeds_on_special_blocks_match_wiki_walking_table() {
    // Slime: the wiki's 3.040 m/s ignores SlimeBlock.stepOn (0.4 + |vy| x 0.2 every tick on
    // the ground); vanilla 26.3 applies it, giving ~1.186 m/s. See haste-physics NOTES.md.
    for (block, expected) in [
        (ICE, 4.157),
        (BLUE_ICE, 4.376),
        (SLIME, 1.1857),
        (SOUL_SAND, 2.508),
    ] {
        let (a, b) = steady_speed(block, forward());
        assert!((a - b).abs() < 1e-6, "{block:?} not steady");
        assert!(
            (mps(b) - expected).abs() < 0.001,
            "{block:?}: {} m/s, wiki {expected}",
            mps(b)
        );
    }
}

#[test]
fn sprint_jump_average_speed_matches_wiki() {
    // wiki "Sprinting"/"Transportation": sprint-jumping averages 7.127 m/s.
    let input = InputState {
        forward: true,
        sprint: true,
        jump: true,
        ..Default::default()
    };
    // Average over a long window: 100 jump cycles of 12 ticks after a warm-up.
    let mut s = Sim::new(TestWorld::flat(0, STONE), DVec3::new(0.5, 0.0, 0.5));
    s.run(&input, 200);
    let a = s.state.pos;
    s.run(&input, 1200);
    let avg = mps(hspeed(s.state.pos - a) / 1200.0);
    assert!((avg - 7.127).abs() < 0.005, "sprint-jump average {avg} m/s");
}

fn sprint_jump_gap(gap: i32, start_x: f64) -> bool {
    // Platform x < 0 and x >= gap (top y = 0), void in between down to y = -11.
    let mut w = TestWorld::new();
    w.fill((-30, -1, -2), (-1, -1, 2), STONE);
    w.fill((gap, -1, -2), (gap + 10, -1, 2), STONE);
    w.fill((-30, -12, -2), (gap + 10, -12, 2), STONE);
    let mut s = Sim::new(w, DVec3::new(start_x, 0.0, 0.5));
    let run = InputState {
        forward: true,
        sprint: true,
        yaw: -90.0,
        ..Default::default()
    }; // +x
    let jump = InputState { jump: true, ..run };
    for _ in 0..400 {
        // Jump on the last tick from which another ground tick would leave the platform.
        let next_move = s.state.velocity.x.abs() + 0.13;
        let input = if s.state.on_ground && s.state.pos.x - 0.3 + next_move >= 0.0 {
            jump
        } else {
            run
        };
        s.step(&input);
        if s.state.on_ground && s.state.pos.x + 0.3 > gap as f64 {
            return s.state.pos.y == 0.0;
        }
        if s.state.pos.y < -1.0 {
            return false;
        }
    }
    false
}

fn best_of_phases(gap: i32) -> bool {
    (0..40).any(|i| sprint_jump_gap(gap, -25.5 + i as f64 * 0.01))
}

#[test]
fn sprint_jump_crosses_three_air_blocks() {
    // wiki "Sprinting": "jump across up to four blocks horizontally" (a jump onto the 4th block,
    // i.e. a 3-block gap, without 45 degree strafing). Whether a 4-air-block gap is possible is
    // not asserted: the wiki's wording does not settle it; the traces will.
    assert!(best_of_phases(3));
}

#[test]
fn walking_up_slabs_steps() {
    let mut w = TestWorld::flat(0, STONE);
    w.fill((2, 0, -2), (2, 0, 2), BOTTOM_SLAB);
    w.fill((3, 0, -2), (8, 0, 2), STONE);
    let mut s = Sim::new(w, DVec3::new(0.5, 0.0, 0.5));
    let input = InputState {
        forward: true,
        yaw: -90.0,
        ..Default::default()
    };
    s.run(&input, 40);
    assert!(s.state.pos.x > 4.0, "{:?}", s.state.pos);
    assert_eq!(s.state.pos.y, 1.0);
    assert!(s.state.on_ground);
}

#[test]
fn walking_into_a_full_block_stops() {
    let mut w = TestWorld::flat(0, STONE);
    w.fill((2, 0, -2), (2, 0, 2), STONE);
    let mut s = Sim::new(w, DVec3::new(0.5, 0.0, 0.5));
    let input = InputState {
        forward: true,
        sprint: true,
        yaw: -90.0,
        ..Default::default()
    };
    s.run(&input, 40);
    // The box half-width is 0.6f32 / 2 widened to f64 (entity dimensions are floats), so the
    // player stops at 2 - 0.30000001192..., not at 1.7.
    let half = attrs_half_width();
    assert_eq!(s.state.pos.x, 2.0 - half, "{:?}", s.state.pos);
    assert!(s.state.horizontal_collision);
    assert!(!s.state.sprinting, "collision ends sprinting");
}

fn attrs_half_width() -> f64 {
    PlayerAttributes::default().width as f64 / 2.0
}

// ---- sneaking --------------------------------------------------------------------------------

#[test]
fn sneaking_at_an_edge_does_not_fall() {
    for (yaw, left) in [
        (-90.0f32, false),
        (-90.0, true),
        (0.0, false),
        (-45.0, false),
        (135.0, true),
    ] {
        let mut w = TestWorld::new();
        w.set(0, -1, 0, STONE); // a single block, top at y = 0
        w.fill((-5, -6, -5), (5, -6, 5), STONE);
        let mut s = Sim::new(w, DVec3::new(0.5, 0.0, 0.5));
        assert!(s.state.on_ground);
        let input = InputState {
            forward: true,
            left,
            sneak: true,
            yaw,
            ..Default::default()
        };
        s.run(&input, 200);
        assert_eq!(s.state.pos.y, 0.0, "fell off at yaw {yaw}");
        assert_eq!(s.state.pose, Pose::Crouching);
        // The box may overhang but must still touch the block.
        assert!(
            s.state.pos.x > -0.3
                && s.state.pos.x < 1.3
                && s.state.pos.z > -0.3
                && s.state.pos.z < 1.3
        );
        // Releasing sneak walks off.
        s.run(
            &InputState {
                sneak: false,
                ..input
            },
            40,
        );
        assert!(
            s.state.pos.y < -1.0,
            "did not fall without sneak at yaw {yaw}"
        );
    }
}

#[test]
fn crouch_kept_under_low_ceiling() {
    let mut w = TestWorld::flat(0, STONE);
    w.fill((-2, 1, -2), (2, 1, 2), TOP_SLAB); // ceiling at y = 1.5
    let mut s = Sim::new(w, DVec3::new(0.5, 0.0, 0.5));
    s.run(
        &InputState {
            sneak: true,
            ..Default::default()
        },
        2,
    );
    assert_eq!(s.state.pose, Pose::Crouching);
    s.run(&InputState::default(), 2);
    assert_eq!(s.state.pose, Pose::Crouching);
}

/// Per-tick crouch speed from the tick shift is pressed (vanilla `LocalPlayer.modifyInput`:
/// the input vector is scaled by 0.98 and the sneaking_speed attribute, 0.3 by default
/// (vanilla `Attributes.SNEAKING_SPEED`), so the first tick's impulse is
/// 0.98 x 0.3 = 0.294 forward; each tick adds `impulse x movement_speed` (0.1 on stone) and
/// keeps `0.6 friction x 0.91 drag` = 0.546 of it: move(n) = move(n-1) x 0.546 + 0.0294.
/// The slowdown applies on the very first tick (vanilla computes its `crouching` field before
/// the movement), while the pose (and its smaller box) only changes after it.
#[test]
fn crouch_speed_per_tick_from_pressing_shift() {
    let mut s = Sim::new(TestWorld::flat(0, STONE), DVec3::new(0.5, 0.0, 0.5));
    let input = InputState {
        forward: true,
        sneak: true,
        ..Default::default()
    };
    let mut expected = 0.0f64; // blocks per tick, forwards (+z at yaw 0)
    for t in 0..5 {
        let d = s.step(&input);
        // The impulse and speed are computed in f32 (input 0.98 x 0.3, speed 0.1) and added in
        // f64 by `accelerate`, so each term is the f32 product cast up; the kept velocity is
        // dragged by the f32 product 0.6 x 0.91 in f64.
        expected = expected * f64::from(0.6f32 * 0.91f32) + f64::from(0.98f32 * 0.3f32) * 0.1;
        assert!(
            (d.z - expected).abs() < 1e-9,
            "tick {t}: moved {} expected {expected}",
            d.z
        );
        assert_eq!(d.x, 0.0);
        assert!(s.state.crouching, "slow from the first tick");
        // The pose is updated at the end of the tick (vanilla `Player.updatePlayerPose`
        // runs after the movement).
        assert_eq!(s.state.pose, Pose::Crouching);
    }
    // Steady state: with the f32 constants above, 0.029400001 / (1 - 0.546) blocks per tick
    // = 1.295 m/s, the documented sneaking speed (wiki "Sneaking").
    s.run(&input, 100);
    let d = s.step(&input);
    assert!((d.z - 0.0647577).abs() < 1e-6, "{}", d.z);
    assert!((d.z * 20.0 - 1.295).abs() < 0.001, "{} m/s", d.z * 20.0);
    // Releasing shift: the input stops being scaled (one tick of crouching pose remains).
    let stand = InputState {
        forward: true,
        ..Default::default()
    };
    s.step(&stand);
    assert!(!s.state.crouching);
    let d = s.step(&stand);
    assert!(
        d.z > 0.0647577 + 1e-6,
        "walks faster once shift is released: {}",
        d.z
    );
}

/// Vanilla `Player.updatePlayerPose` only leaves the pose alone when even the swimming box does
/// not fit; otherwise an unfit desired pose falls back to crouching, then swimming.
#[test]
fn pose_fallback_chain_matches_update_player_pose() {
    // Shift held in the open: crouch.
    let mut s = Sim::new(TestWorld::flat(0, STONE), DVec3::new(0.5, 0.0, 0.5));
    s.run(
        &InputState {
            sneak: true,
            ..Default::default()
        },
        2,
    );
    assert_eq!(s.state.pose, Pose::Crouching);
    // Shift held while flying: standing (vanilla `Player.getDesiredPose`); flight is kept
    // because the player hovers (vertical drag, no ground contact).
    let mut s = Sim::new(TestWorld::new(), DVec3::new(0.5, 80.0, 0.5));
    s.state.flying = true;
    s.state.on_ground = false;
    s.run(
        &InputState {
            sneak: true,
            ..Default::default()
        },
        2,
    );
    assert_eq!(s.state.pose, Pose::Standing);
    // A 1.0-high gap: crouching (1.5) does not fit, so the pose falls back to swimming.
    let mut w = TestWorld::flat(0, STONE);
    w.fill((-2, 1, -2), (2, 1, 2), STONE); // ceiling at y = 1
    let mut s = Sim::new(w, DVec3::new(0.5, 0.0, 0.5));
    s.run(
        &InputState {
            sneak: true,
            ..Default::default()
        },
        2,
    );
    assert_eq!(s.state.pose, Pose::Swimming);
}

// ---- fluids ----------------------------------------------------------------------------------

fn pool(floor: haste_core::BlockStateId, fluid: haste_core::BlockStateId, depth: i32) -> TestWorld {
    let mut w = TestWorld::flat(0, floor);
    w.fill((-50, 0, -50), (50, depth - 1, 50), fluid);
    w
}

#[test]
fn water_and_lava_walking_speeds_match_wiki() {
    // wiki "Walking" table, head-level block water/lava.
    for (floor, fluid, expected) in [
        (STONE, WATER_SOURCE, 1.960),
        (STONE, LAVA_SOURCE, 0.784),
        (SOUL_SAND, WATER_SOURCE, 0.576),
        (SOUL_SAND, LAVA_SOURCE, 0.490),
    ] {
        let mut s = Sim::new(pool(floor, fluid, 4), DVec3::new(0.5, 0.0, 0.5));
        s.run(&forward(), 100);
        let v = hspeed(s.step(&forward()));
        assert!(
            (mps(v) - expected).abs() < 0.001,
            "{floor:?}/{fluid:?}: {} m/s, wiki {expected}",
            mps(v)
        );
    }
}

#[test]
fn sinking_in_water_is_slow_and_jump_swims_up() {
    let mut s = Sim::new(pool(STONE, WATER_SOURCE, 20), DVec3::new(0.5, 10.0, 0.5));
    s.state.pos.y = 10.0;
    s.state.velocity = DVec3::ZERO;
    s.state.on_ground = false;
    s.run(&InputState::default(), 40);
    assert!(s.state.in_water);
    assert!(s.state.pos.y < 10.0, "sinks");
    assert!(
        s.state.velocity.y < 0.0 && s.state.velocity.y > -0.1,
        "sinks slowly: {}",
        s.state.velocity.y
    );
    let y = s.state.pos.y;
    s.run(
        &InputState {
            jump: true,
            ..Default::default()
        },
        20,
    );
    assert!(
        s.state.pos.y > y + 0.5,
        "swims up: {} -> {}",
        y,
        s.state.pos.y
    );
    // Sneaking sinks faster than idling.
    let mut a = Sim::new(pool(STONE, WATER_SOURCE, 20), DVec3::new(0.5, 10.0, 0.5));
    a.state.pos.y = 10.0;
    let mut b = Sim::new(pool(STONE, WATER_SOURCE, 20), DVec3::new(0.5, 10.0, 0.5));
    b.state.pos.y = 10.0;
    a.run(&InputState::default(), 10);
    b.run(
        &InputState {
            sneak: true,
            ..Default::default()
        },
        10,
    );
    assert!(b.state.pos.y < a.state.pos.y);
    // No fall damage accumulates in water.
    assert_eq!(s.state.fall_distance, 0.0);
}

#[test]
fn sprint_swimming_speed_and_pose() {
    // wiki "Swimming": sprint-swimming ~3.918 m/s; uses the swimming pose.
    let mut s = Sim::new(pool(STONE, WATER_SOURCE, 20), DVec3::new(0.5, 5.0, 0.5));
    s.state.pos.y = 5.0;
    let input = InputState {
        forward: true,
        sprint: true,
        ..Default::default()
    };
    s.run(&input, 100);
    assert!(s.state.swimming);
    assert_eq!(s.state.pose, Pose::Swimming);
    let v = hspeed(s.step(&input));
    assert!((mps(v) - 3.918).abs() < 0.01, "sprint-swim {} m/s", mps(v));
}

// ---- flying ----------------------------------------------------------------------------------

#[test]
fn creative_flying_speed() {
    // wiki "Flying": about 10.92 m/s, sprint-flying 21.6 m/s ("Transportation": 11.0 / 22.0).
    let mut s = Sim::new(TestWorld::new(), DVec3::new(0.5, 100.0, 0.5));
    s.state = PlayerState::at(DVec3::new(0.5, 100.0, 0.5));
    s.state.flying = true;
    s.run(&forward(), 100);
    let d = s.step(&forward());
    assert!(
        (mps(hspeed(d)) - 10.92).abs() < 0.1,
        "fly {} m/s",
        mps(hspeed(d))
    );
    assert_eq!(d.y, 0.0, "no gravity while flying");
    let sprint = InputState {
        forward: true,
        sprint: true,
        ..Default::default()
    };
    s.run(&sprint, 100);
    let d = s.step(&sprint);
    assert!(
        (mps(hspeed(d)) - 21.6).abs() < 0.4,
        "sprint-fly {} m/s",
        mps(hspeed(d))
    );
    let y = s.state.pos.y;
    s.run(
        &InputState {
            jump: true,
            ..Default::default()
        },
        10,
    );
    assert!(s.state.pos.y > y + 1.0);
}

#[test]
fn spectator_no_physics_walks_through_a_wall() {
    // Vanilla `Player.tick` sets `noPhysics = isSpectator()`; `Entity.move` then adds the
    // motion without collision. A wall that stops a normal flying player is passed through.
    let mut wall = TestWorld::new();
    wall.fill((-1, 99, 2), (1, 101, 2), STONE);
    let mut normal = Sim::new(wall, DVec3::new(0.5, 100.0, 0.5));
    normal.state = PlayerState::at(DVec3::new(0.5, 100.0, 0.5));
    normal.state.flying = true;
    normal.run(&forward(), 40);
    assert!(
        normal.state.pos.z < 2.0,
        "the wall should stop a colliding player: z={}",
        normal.state.pos.z
    );
    assert!(normal.state.horizontal_collision);

    let mut ghost = TestWorld::new();
    ghost.fill((-1, 99, 2), (1, 101, 2), STONE);
    let mut spectator = Sim::new(ghost, DVec3::new(0.5, 100.0, 0.5));
    spectator.state = PlayerState::at(DVec3::new(0.5, 100.0, 0.5));
    spectator.state.flying = true;
    spectator.state.no_physics = true;
    spectator.run(&forward(), 40);
    assert!(
        spectator.state.pos.z > 2.0,
        "spectator should pass through the wall: z={}",
        spectator.state.pos.z
    );
    assert!(!spectator.state.horizontal_collision);
}

/// Vanilla's input chain (`KeyboardInput.tick` normalises the key vector, `LocalPlayer.modifyInput`
/// scales it by 0.98 and the 0.3 sneak factor, `modifyInputSpeedForSquareMovement` stretches it by
/// the direction's distance to the unit square): a sneaking diagonal (0.707, 0.707) becomes
/// length 0.294, stretched by sqrt(2) to 0.4158, i.e. 0.294 on EACH axis: the same per-axis speed
/// as sneaking straight (1.295 m/s), so the diagonal is sqrt(2) times faster (1.83 m/s). A
/// full-speed diagonal is capped at length 1 (0.707 per axis, against 0.98 straight).
#[test]
fn square_movement_diagonal_matches_vanilla() {
    let straight = steady_speed(
        STONE,
        InputState {
            forward: true,
            sneak: true,
            ..Default::default()
        },
    )
    .1;
    let mut sim = Sim::new(TestWorld::flat(-1, STONE), DVec3::new(0.5, 0.0, 0.5));
    let diag = InputState {
        forward: true,
        left: true,
        sneak: true,
        ..Default::default()
    };
    sim.run(&diag, 100);
    let d = sim.step(&diag);
    assert!(
        (d.x.abs() - straight).abs() < 1e-6 && (d.z.abs() - straight).abs() < 1e-6,
        "diagonal sneak per axis ({}, {}) should equal the straight sneak speed {}",
        d.x.abs(),
        d.z.abs(),
        straight
    );
    assert!(
        (mps(hspeed(d)) - 1.295 * std::f64::consts::SQRT_2).abs() < 0.002,
        "diagonal sneak {} m/s",
        mps(hspeed(d))
    );

    let walk = steady_speed(STONE, forward()).1;
    let mut sim = Sim::new(TestWorld::flat(-1, STONE), DVec3::new(0.5, 0.0, 0.5));
    let diag = InputState {
        forward: true,
        left: true,
        ..Default::default()
    };
    sim.run(&diag, 100);
    let d = sim.step(&diag);
    let per_axis = walk / 0.98 * std::f64::consts::FRAC_1_SQRT_2;
    assert!(
        (d.x.abs() - per_axis).abs() < 1e-5 && (d.z.abs() - per_axis).abs() < 1e-5,
        "full-speed diagonal per axis ({}, {}) should be {}",
        d.x.abs(),
        d.z.abs(),
        per_axis
    );
}
