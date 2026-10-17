//! Climbing scenarios (vanilla `LivingEntity.onClimbable` / `handleOnClimbable` and the climb
//! boost in `handleRelativeFrictionAndCalculateMovement`): the numbers are vanilla's constants
//! run through vanilla's order of steps (the 0.2 boost, then gravity 0.08, then the 0.98 drag).
use haste_core::input::InputState;
use haste_core::DVec3;
use haste_physics::testing::{states::*, TestShapes, TestWorld};
use haste_physics::{on_climbable, tick_with, PlayerAttributes, PlayerState};

struct Sim {
    world: TestWorld,
    shapes: TestShapes,
    attrs: PlayerAttributes,
    state: PlayerState,
}

impl Sim {
    fn new(world: TestWorld, pos: DVec3) -> Self {
        Self {
            world,
            shapes: TestShapes::default(),
            attrs: PlayerAttributes::default(),
            state: PlayerState::at(pos),
        }
    }
    fn step(&mut self, input: &InputState) -> DVec3 {
        let before = self.state.pos;
        tick_with(
            &mut self.state,
            input,
            &self.attrs,
            &self.world,
            &self.shapes,
            &self.shapes,
            &self.shapes,
        );
        self.state.pos - before
    }
}

/// A stone floor whose surface is y = 0, a ladder column at (0, 0..=6, 0) on a stone wall at z = 1.
fn ladder_world() -> TestWorld {
    let mut w = TestWorld::flat(0, STONE);
    w.fill((0, 0, 0), (0, 6, 0), LADDER);
    w.fill((-1, 0, 1), (1, 7, 1), STONE);
    w
}

/// The rise per tick while climbing: the boost 0.2, minus gravity, times the vertical drag.
fn climb_rise(attrs: &PlayerAttributes) -> f64 {
    (haste_physics::climbing::CLIMB_UP_SPEED - attrs.gravity)
        * haste_physics::movement::VERTICAL_DRAG as f64
}

#[test]
fn pushing_into_a_ladder_climbs_at_vanilla_speed() {
    // Facing +z (yaw 0) into the ladder's slab: the horizontal collision on a climbable sets
    // y = 0.2 after the move, so every following tick rises (0.2 - 0.08) x 0.98 = 0.1176 blocks
    // (2.35 m/s, the known ladder speed).
    let mut sim = Sim::new(ladder_world(), DVec3::new(0.5, 0.0, 0.5));
    sim.state.on_ground = true;
    let forward = InputState {
        forward: true,
        ..Default::default()
    };
    let mut climbed = 0;
    for _ in 0..12 {
        let d = sim.step(&forward);
        if sim.state.horizontal_collision {
            assert!(
                (sim.state.velocity.y - climb_rise(&sim.attrs)).abs() < 1e-12,
                "velocity after a climbing tick {}",
                sim.state.velocity.y
            );
            if climbed > 0 {
                assert!(
                    (d.y - climb_rise(&sim.attrs)).abs() < 1e-12,
                    "rise per tick {}",
                    d.y
                );
            }
            climbed += 1;
        }
    }
    assert!(climbed >= 6, "only {climbed} climbing ticks");
    assert_eq!(sim.state.fall_distance, 0.0);
}

#[test]
fn holding_jump_on_a_ladder_climbs_without_jumping() {
    // Standing inside the ladder cell without touching its slab: the jump key alone (vanilla
    // `jumping`) gives the 0.2 boost, which replaces the 0.42 jump velocity.
    let mut sim = Sim::new(ladder_world(), DVec3::new(0.5, 0.0, 0.3));
    sim.state.on_ground = true;
    let jump = InputState {
        jump: true,
        ..Default::default()
    };
    sim.step(&jump);
    assert!((sim.state.velocity.y - climb_rise(&sim.attrs)).abs() < 1e-12);
    for _ in 0..5 {
        let d = sim.step(&jump);
        assert!(!sim.state.horizontal_collision);
        assert!((d.y - climb_rise(&sim.attrs)).abs() < 1e-12, "rise {}", d.y);
    }
}

#[test]
fn falling_down_a_ladder_is_capped_at_0_15() {
    let mut sim = Sim::new(ladder_world(), DVec3::new(0.5, 5.0, 0.3));
    let idle = InputState::default();
    let mut capped_ticks = 0;
    for _ in 0..60 {
        let d = sim.step(&idle);
        assert!(
            d.y >= -haste_physics::climbing::CLIMB_SPEED_LIMIT - 1e-12,
            "fell {} in one tick at y={} (on climbable: {})",
            -d.y,
            sim.state.pos.y,
            on_climbable(&sim.state, &sim.world, &sim.shapes)
        );
        assert!(sim.state.fall_distance <= 0.15 + 1e-6);
        if d.y == -haste_physics::climbing::CLIMB_SPEED_LIMIT {
            capped_ticks += 1;
        }
        if sim.state.on_ground {
            break;
        }
    }
    assert!(
        capped_ticks >= 10,
        "{capped_ticks} ticks at the floor speed"
    );
    assert!(sim.state.on_ground, "should reach the floor");
    assert_eq!(sim.state.fall_distance, 0.0);
}

#[test]
fn sneaking_holds_the_player_on_a_ladder_but_not_on_scaffolding() {
    let sneak = InputState {
        sneak: true,
        ..Default::default()
    };
    let mut sim = Sim::new(ladder_world(), DVec3::new(0.5, 5.0, 0.3));
    for _ in 0..20 {
        sim.step(&sneak);
    }
    // Each tick vanilla zeroes the (negative) fall speed before the move, moves nothing, then
    // adds gravity again; the position never changes.
    assert_eq!(sim.state.pos.y, 5.0, "a sneaking player stays put");

    let mut w = TestWorld::flat(0, STONE);
    w.fill((0, 0, 0), (0, 6, 0), SCAFFOLDING);
    let mut sim = Sim::new(w, DVec3::new(0.5, 5.0, 0.5));
    for _ in 0..5 {
        sim.step(&sneak);
    }
    let d = sim.step(&sneak);
    assert_eq!(
        d.y,
        -haste_physics::climbing::CLIMB_SPEED_LIMIT,
        "sneaking slides down scaffolding at the floor speed"
    );
}

#[test]
fn open_trapdoor_over_a_matching_ladder_is_climbable() {
    let shapes = TestShapes::default();
    let mut w = TestWorld::flat(0, STONE);
    w.set(0, 0, 0, LADDER).set(0, 1, 0, OPEN_TRAPDOOR);
    let on_trapdoor = PlayerState::at(DVec3::new(0.5, 1.0, 0.3));
    assert!(on_climbable(&on_trapdoor, &w, &shapes));

    let mut w = TestWorld::flat(0, STONE);
    w.set(0, 0, 0, STONE).set(0, 1, 0, OPEN_TRAPDOOR);
    assert!(!on_climbable(&on_trapdoor, &w, &shapes));

    let mut spectator = on_trapdoor;
    spectator.spectating = true;
    let mut w = TestWorld::flat(0, STONE);
    w.set(0, 0, 0, LADDER).set(0, 1, 0, OPEN_TRAPDOOR);
    assert!(!on_climbable(&spectator, &w, &shapes));
}
