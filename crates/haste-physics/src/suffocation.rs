//! Vanilla `LocalPlayer.moveTowardsClosestSpace` / `LocalPlayer.suffocatesAt`: frees a player
//! who ends a tick inside a suffocating block (sand falling on them, a piston or a door closing)
//! by nudging one horizontal velocity component to +-0.1 towards the nearest free neighbour.
//!
//! `LocalPlayer.aiStep` calls [`move_towards_closest_space`] for the four points at
//! +-0.35 * bbWidth around the feet, in vanilla's WEST, EAST, NORTH, SOUTH order, when not
//! `noPhysics` and not a passenger.
use haste_core::view::{BlockPhysics, BlockView, CollisionShapes};
use haste_core::{Aabb, BlockPos, DVec3, Direction};

use crate::PlayerState;

/// The deflation vanilla applies to the tested column (`AABB.deflate(1.0E-7)`).
const SUFFOCATE_EPSILON: f64 = 1.0e-7;

/// Vanilla `LocalPlayer.aiStep`'s four push-out probes: `moveTowardsClosestSpace` at
/// +-0.35 * bbWidth around the feet, in the vanilla call order.
pub fn move_towards_closest_space(
    state: &mut PlayerState,
    bb: &Aabb,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    blocks: &dyn BlockPhysics,
) {
    let width = bb.max.x - bb.min.x;
    let (x, z) = (state.pos.x, state.pos.z);
    let d = width * 0.35;
    move_one(state, x - d, z + d, bb, world, shapes, blocks);
    move_one(state, x - d, z - d, bb, world, shapes, blocks);
    move_one(state, x + d, z - d, bb, world, shapes, blocks);
    move_one(state, x + d, z + d, bb, world, shapes, blocks);
}

/// Vanilla `LocalPlayer.moveTowardsClosestSpace(x, z)`.
fn move_one(
    state: &mut PlayerState,
    x: f64,
    z: f64,
    bb: &Aabb,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    blocks: &dyn BlockPhysics,
) {
    let pos = BlockPos::new(
        x.floor() as i32,
        state.pos.y.floor() as i32,
        z.floor() as i32,
    );
    if !suffocates_at(pos, bb, world, shapes, blocks) {
        return;
    }
    let xd = x - pos.x as f64;
    let zd = z - pos.z as f64;
    // WEST/EAST lie on the X axis (distance xd, 1 - xd), NORTH/SOUTH on Z (zd, 1 - zd);
    // EAST and SOUTH are the positive axis directions. Strict `<` keeps the first on ties.
    let candidates = [
        (Direction::West, xd),
        (Direction::East, 1.0 - xd),
        (Direction::North, zd),
        (Direction::South, 1.0 - zd),
    ];
    let mut closest = f64::MAX;
    let mut chosen = None;
    for (dir, distance_to_edge) in candidates {
        if distance_to_edge < closest && !suffocates_at(pos.offset(dir), bb, world, shapes, blocks)
        {
            closest = distance_to_edge;
            chosen = Some(dir);
        }
    }
    if let Some(dir) = chosen {
        match dir {
            Direction::West | Direction::East => state.velocity.x = 0.1 * dir.step_x() as f64,
            Direction::North | Direction::South => state.velocity.z = 0.1 * dir.step_z() as f64,
            _ => {}
        }
    }
}

/// Vanilla `LocalPlayer.suffocatesAt(pos)`: the full-height column of `bb` at `pos`, deflated,
/// against every block whose state is suffocating and whose collision shape intersects it.
fn suffocates_at(
    pos: BlockPos,
    bb: &Aabb,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    blocks: &dyn BlockPhysics,
) -> bool {
    let test = Aabb {
        min: DVec3::new(
            pos.x as f64 + SUFFOCATE_EPSILON,
            bb.min.y + SUFFOCATE_EPSILON,
            pos.z as f64 + SUFFOCATE_EPSILON,
        ),
        max: DVec3::new(
            pos.x as f64 + 1.0 - SUFFOCATE_EPSILON,
            bb.max.y - SUFFOCATE_EPSILON,
            pos.z as f64 + 1.0 - SUFFOCATE_EPSILON,
        ),
    };
    let world_min = world.min_y();
    let world_max = world_min + world.height() - 1;
    let y0 = (test.min.y.floor() as i32).max(world_min);
    let y1 = (test.max.y.floor() as i32).min(world_max);
    for y in y0..=y1 {
        let p = BlockPos::new(pos.x, y, pos.z);
        let state = world.block_state(p);
        if !blocks.is_suffocating(state) {
            continue;
        }
        let origin = DVec3::new(pos.x as f64, y as f64, pos.z as f64);
        for local in shapes.collision_boxes(state) {
            if local.offset(origin).intersects(&test) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests;
