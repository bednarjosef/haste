//! collide()/collide_with_step() against a tiny in-memory world.
use haste_core::{Aabb, DVec3};
use haste_physics::collision::no_collision;
use haste_physics::testing::{states::*, TestShapes, TestWorld};
use haste_physics::{collide, collide_with_step};

const W: f64 = 0.6;
const H: f64 = 1.8;

fn player(x: f64, y: f64, z: f64) -> Aabb {
    Aabb::entity(DVec3::new(x, y, z), W, H)
}

fn close(a: DVec3, b: DVec3) {
    assert!((a - b).abs().max_element() < 1e-12, "{a:?} != {b:?}");
}

#[test]
fn free_movement_is_unchanged() {
    let world = TestWorld::new();
    let shapes = TestShapes::default();
    let m = DVec3::new(0.3, -0.7, 0.21);
    assert_eq!(collide(player(0.5, 10.0, 0.5), m, &world, &shapes), m);
}

#[test]
fn falling_stops_exactly_on_floor() {
    let world = TestWorld::flat(0, STONE);
    let shapes = TestShapes::default();
    // 0.3 above the floor, falling 1.0: stops at the surface.
    let got = collide(
        player(0.5, 0.3, 0.5),
        DVec3::new(0.0, -1.0, 0.0),
        &world,
        &shapes,
    );
    close(got, DVec3::new(0.0, -0.3, 0.0));
    // Terminal-velocity fall from high up lands on y = 0 exactly.
    let y0 = 7.123456789;
    let got = collide(
        player(0.5, y0, 0.5),
        DVec3::new(0.0, -100.0, 0.0),
        &world,
        &shapes,
    );
    assert_eq!(y0 + got.y, 0.0);
    // Resting on the floor: further downward movement is zero.
    assert_eq!(
        collide(
            player(0.5, 0.0, 0.5),
            DVec3::new(0.0, -0.0784, 0.0),
            &world,
            &shapes
        ),
        DVec3::ZERO
    );
}

#[test]
fn ceiling_stops_jump() {
    let mut world = TestWorld::flat(0, STONE);
    world.fill((-2, 2, -2), (2, 2, 2), STONE); // ceiling bottom at y = 2
    let shapes = TestShapes::default();
    let got = collide(
        player(0.5, 0.0, 0.5),
        DVec3::new(0.0, 0.42, 0.0),
        &world,
        &shapes,
    );
    close(got, DVec3::new(0.0, 0.2, 0.0));
}

#[test]
fn wall_stops_at_face() {
    let mut world = TestWorld::flat(0, STONE);
    world.fill((2, 0, -3), (2, 3, 3), STONE); // wall face at x = 2
    let shapes = TestShapes::default();
    let got = collide(
        player(1.5, 0.0, 0.5),
        DVec3::new(0.5, 0.0, 0.0),
        &world,
        &shapes,
    );
    close(got, DVec3::new(0.2, 0.0, 0.0)); // box max x 1.8 -> 2.0
                                           // Negative direction against a wall at x = -1 (face at x = 0).
    world.fill((-1, 0, -3), (-1, 3, 3), STONE);
    let got = collide(
        player(0.5, 0.0, 0.5),
        DVec3::new(-0.5, 0.0, 0.0),
        &world,
        &shapes,
    );
    close(got, DVec3::new(-0.2, 0.0, 0.0));
}

#[test]
fn sliding_along_wall_on_two_axes() {
    let mut world = TestWorld::flat(0, STONE);
    world.fill((2, 0, -5), (2, 3, 5), STONE);
    let shapes = TestShapes::default();
    let got = collide(
        player(1.5, 0.0, 0.5),
        DVec3::new(0.5, 0.0, 0.4),
        &world,
        &shapes,
    );
    close(got, DVec3::new(0.2, 0.0, 0.4));
    // Z larger than X: Z first, then X stops at the face.
    let got = collide(
        player(1.5, 0.0, 0.5),
        DVec3::new(0.3, 0.0, 0.9),
        &world,
        &shapes,
    );
    close(got, DVec3::new(0.2, 0.0, 0.9));
}

#[test]
fn corner_where_two_blocks_meet() {
    // Two blocks meeting at an edge: (1,0,0) and (0,0,1). Player at (0.5,0,0.5) touching neither.
    let mut world = TestWorld::flat(0, STONE);
    world
        .set(1, 0, 0, STONE)
        .set(1, 1, 0, STONE)
        .set(0, 0, 1, STONE)
        .set(0, 1, 1, STONE);
    let shapes = TestShapes::default();
    let got = collide(
        player(0.5, 0.0, 0.5),
        DVec3::new(0.5, 0.0, 0.5),
        &world,
        &shapes,
    );
    close(got, DVec3::new(0.2, 0.0, 0.2));
    // Only a diagonal corner block at (1,0,1): the axis order decides. |x| == |z| -> X first,
    // X is free (box does not overlap the corner block in Z yet), then Z is blocked.
    let mut world = TestWorld::flat(0, STONE);
    world.set(1, 0, 1, STONE).set(1, 1, 1, STONE);
    let got = collide(
        player(0.5, 0.0, 0.5),
        DVec3::new(0.5, 0.0, 0.5),
        &world,
        &shapes,
    );
    close(got, DVec3::new(0.5, 0.0, 0.2));
    // |z| > |x| -> Z first and free, then X is blocked.
    let got = collide(
        player(0.5, 0.0, 0.5),
        DVec3::new(0.45, 0.0, 0.5),
        &world,
        &shapes,
    );
    close(got, DVec3::new(0.2, 0.0, 0.5));
}

#[test]
fn touching_faces_do_not_block_parallel_movement() {
    // Player flush against a wall at x = 2 walks along it in Z.
    let mut world = TestWorld::flat(0, STONE);
    world.fill((2, 0, -5), (2, 3, 5), STONE);
    let shapes = TestShapes::default();
    let got = collide(
        player(1.7, 0.0, 0.5),
        DVec3::new(0.0, 0.0, 0.5),
        &world,
        &shapes,
    );
    close(got, DVec3::new(0.0, 0.0, 0.5));
    assert!(no_collision(&player(1.7, 0.0, 0.5), &world, &shapes));
}

#[test]
fn step_onto_half_slab() {
    let mut world = TestWorld::flat(0, STONE);
    world.fill((1, 0, -2), (3, 0, 2), BOTTOM_SLAB);
    let shapes = TestShapes::default();
    let m = DVec3::new(0.3, -0.0784, 0.0);
    // Plain sweep is blocked at x = 1.
    let plain = collide(player(0.6, 0.0, 0.5), m, &world, &shapes);
    close(plain, DVec3::new(0.1, 0.0, 0.0));
    let got = collide_with_step(player(0.6, 0.0, 0.5), m, 0.6, true, &world, &shapes);
    close(got, DVec3::new(0.3, 0.5, 0.0));
    // Not on ground: no step.
    let got = collide_with_step(
        player(0.6, 0.5, 0.5),
        DVec3::new(0.3, 0.0, 0.0),
        0.6,
        false,
        &world,
        &shapes,
    );
    close(got, DVec3::new(0.3, 0.0, 0.0)); // at y 0.5 it clears the slab anyway
    let got = collide_with_step(
        player(0.6, 0.2, 0.5),
        DVec3::new(0.3, 0.0, 0.0),
        0.6,
        false,
        &world,
        &shapes,
    );
    close(got, DVec3::new(0.1, 0.0, 0.0));
}

#[test]
fn step_onto_stair_bottom_then_top() {
    let mut world = TestWorld::flat(0, STONE);
    world.set(1, 0, 0, STAIR_NORTH);
    let shapes = TestShapes::default();
    // Walking +x into the stair's side at z in [0,1]: the upper quarter covers z < 0.5, so the
    // box (z 0.2..0.8) is blocked at the step height 0.6 by the upper part (top 1.0 > 0.6).
    let got = collide_with_step(
        player(0.6, 0.0, 0.5),
        DVec3::new(0.3, -0.0784, 0.0),
        0.6,
        true,
        &world,
        &shapes,
    );
    close(got, DVec3::new(0.1, 0.0, 0.0));
    // From the south (box z 1.1..1.7, walking -z): the lower half is steppable; the box ends on y 0.5.
    let got = collide_with_step(
        player(1.5, 0.0, 1.4),
        DVec3::new(0.0, -0.0784, -0.3),
        0.6,
        true,
        &world,
        &shapes,
    );
    close(got, DVec3::new(0.0, 0.5, -0.3));
}

#[test]
fn cannot_step_a_full_block() {
    let mut world = TestWorld::flat(0, STONE);
    world.fill((1, 0, -2), (3, 0, 2), STONE);
    let shapes = TestShapes::default();
    let got = collide_with_step(
        player(0.6, 0.0, 0.5),
        DVec3::new(0.3, -0.0784, 0.0),
        0.6,
        true,
        &world,
        &shapes,
    );
    close(got, DVec3::new(0.1, 0.0, 0.0));
}

#[test]
fn step_limited_by_ceiling() {
    // Slab ahead but a ceiling at y = 2: the box (top 1.8) can rise only 0.2, less than the slab.
    let mut world = TestWorld::flat(0, STONE);
    world.fill((1, 0, -2), (3, 0, 2), BOTTOM_SLAB);
    world.fill((-2, 2, -2), (3, 2, 2), STONE);
    let shapes = TestShapes::default();
    let got = collide_with_step(
        player(0.6, 0.0, 0.5),
        DVec3::new(0.3, -0.0784, 0.0),
        0.6,
        true,
        &world,
        &shapes,
    );
    close(got, DVec3::new(0.1, 0.0, 0.0));
}

#[test]
fn fence_post_is_taller_than_its_cell() {
    let mut world = TestWorld::flat(0, STONE);
    world.set(0, 0, 0, FENCE_POST);
    let shapes = TestShapes::default();
    // Falling onto the 1.5 tall post from y = 3 (block cell y = 0, so the scan must look one
    // block below the swept region).
    let got = collide(
        player(0.5, 3.0, 0.5),
        DVec3::new(0.0, -2.0, 0.0),
        &world,
        &shapes,
    );
    close(got, DVec3::new(0.0, -1.5, 0.0));
    // Can't step over it from the ground.
    let got = collide_with_step(
        player(0.0, 0.0, 0.5),
        DVec3::new(0.3, -0.0784, 0.0),
        0.6,
        true,
        &world,
        &shapes,
    );
    close(got, DVec3::new(0.075, 0.0, 0.0));
}

#[test]
fn soul_sand_is_lower() {
    let world = TestWorld::flat(0, SOUL_SAND);
    let shapes = TestShapes::default();
    let got = collide(
        player(0.5, 0.5, 0.5),
        DVec3::new(0.0, -1.0, 0.0),
        &world,
        &shapes,
    );
    close(got, DVec3::new(0.0, -0.625, 0.0));
}
