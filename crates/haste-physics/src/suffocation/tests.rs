use super::*;
use crate::testing::{states, TestShapes, TestWorld};

/// Player box (0.6 wide, 1.8 tall) with feet at `pos`, like `PlayerState::aabb`.
fn bb(pos: DVec3) -> Aabb {
    Aabb::entity(pos, 0.6, 1.8)
}

fn sand(world: &mut TestWorld, x: i32, z: i32) {
    world.set(x, 0, z, states::SAND).set(x, 1, z, states::SAND);
}

#[test]
fn east_edge_pushes_x_positive() {
    let mut world = TestWorld::new();
    // The player's column and every horizontal neighbour but the east one are sand; the
    // whole full-height column suffocates, so the nearest free neighbour is EAST.
    sand(&mut world, 0, 0);
    sand(&mut world, -1, 0);
    sand(&mut world, 0, -1);
    sand(&mut world, 0, 1);
    let shapes = TestShapes::default();
    let mut state = PlayerState::at(DVec3::new(0.5, 0.0, 0.5));
    let bb = bb(state.pos);
    move_towards_closest_space(&mut state, &bb, &world, &shapes, &shapes);
    assert_eq!(state.velocity, DVec3::new(0.1, 0.0, 0.0));
}

#[test]
fn south_edge_pushes_z_positive() {
    let mut world = TestWorld::new();
    sand(&mut world, 0, 0);
    sand(&mut world, -1, 0);
    sand(&mut world, 1, 0);
    sand(&mut world, 0, -1);
    let shapes = TestShapes::default();
    let mut state = PlayerState::at(DVec3::new(0.5, 0.0, 0.5));
    let bb = bb(state.pos);
    move_towards_closest_space(&mut state, &bb, &world, &shapes, &shapes);
    assert_eq!(state.velocity, DVec3::new(0.0, 0.0, 0.1));
}

#[test]
fn free_air_is_untouched() {
    let world = TestWorld::new();
    let shapes = TestShapes::default();
    let mut state = PlayerState::at(DVec3::new(0.5, 0.0, 0.5));
    state.velocity = DVec3::new(0.3, -0.1, -0.2);
    let bb = bb(state.pos);
    move_towards_closest_space(&mut state, &bb, &world, &shapes, &shapes);
    assert_eq!(state.velocity, DVec3::new(0.3, -0.1, -0.2));
}

#[test]
fn enclosed_without_a_free_neighbour_is_untouched() {
    let mut world = TestWorld::new();
    for (x, z) in [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)] {
        sand(&mut world, x, z);
    }
    let shapes = TestShapes::default();
    let mut state = PlayerState::at(DVec3::new(0.5, 0.0, 0.5));
    state.velocity = DVec3::new(0.0, -0.5, 0.0);
    let bb = bb(state.pos);
    move_towards_closest_space(&mut state, &bb, &world, &shapes, &shapes);
    assert_eq!(state.velocity, DVec3::new(0.0, -0.5, 0.0));
}
