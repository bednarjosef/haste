//! Axis-separated AABB sweep against block collision shapes, plus step-up.
//!
//! Pure functions: they read the world through `BlockView` + `CollisionShapes` and never allocate.
//! Each axis pass walks the blocks in the region swept along that axis and clips the movement
//! against every collision box it finds there (no box list is collected).
//!
//! Order (brief + INTERFACES.md): Y first, then X and Z, the axis with the larger magnitude first.
//! Constants and their status are listed in `NOTES.md`.
use haste_core::view::{BlockView, CollisionShapes};
use haste_core::{Aabb, BlockPos, BlockStateId, DVec3};

/// Tolerance used when deciding whether a box lies "ahead" of the mover on an axis, and below which
/// a clipped movement is snapped to zero. Assumption (not from a public page): 1e-7, the usual
/// value for f64 voxel sweeps. To be settled by traces.
pub const COLLISION_EPSILON: f64 = 1e-7;

/// Some block shapes reach above their cell (fences and walls are 1.5 tall), so every Y range is
/// extended this many blocks downwards when collecting boxes.
const TALL_SHAPE_MARGIN: i32 = 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Axis {
    X,
    Y,
    Z,
}

#[inline]
fn get(v: DVec3, a: Axis) -> f64 {
    match a {
        Axis::X => v.x,
        Axis::Y => v.y,
        Axis::Z => v.z,
    }
}

/// Overlap test on the two axes other than `a` (strict, so touching faces do not overlap).
#[inline]
fn overlaps_other_axes(m: &Aabb, b: &Aabb, a: Axis) -> bool {
    let x = m.min.x < b.max.x && m.max.x > b.min.x;
    let y = m.min.y < b.max.y && m.max.y > b.min.y;
    let z = m.min.z < b.max.z && m.max.z > b.min.z;
    match a {
        Axis::X => y && z,
        Axis::Y => x && z,
        Axis::Z => x && y,
    }
}

/// Calls `f(block_min, state, boxes)` for every block with collision boxes whose cell can intersect
/// `region` (extended down by `TALL_SHAPE_MARGIN`). Stops early when `f` returns `false`.
#[inline]
fn for_each_shape(
    region: &Aabb,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    mut f: impl FnMut(DVec3, &[Aabb]) -> bool,
) {
    let x0 = (region.min.x - COLLISION_EPSILON).floor() as i32;
    let x1 = (region.max.x + COLLISION_EPSILON).floor() as i32;
    let y0 = (region.min.y - COLLISION_EPSILON).floor() as i32 - TALL_SHAPE_MARGIN;
    let y1 = (region.max.y + COLLISION_EPSILON).floor() as i32;
    let z0 = (region.min.z - COLLISION_EPSILON).floor() as i32;
    let z1 = (region.max.z + COLLISION_EPSILON).floor() as i32;
    let world_min = world.min_y();
    let world_max = world_min + world.height() - 1;
    let (y0, y1) = (y0.max(world_min), y1.min(world_max));
    for x in x0..=x1 {
        for z in z0..=z1 {
            for y in y0..=y1 {
                let state = world.block_state(BlockPos::new(x, y, z));
                if state == BlockStateId::AIR {
                    continue;
                }
                let boxes = shapes.collision_boxes(state);
                if boxes.is_empty() {
                    continue;
                }
                if !f(DVec3::new(x as f64, y as f64, z as f64), boxes) {
                    return;
                }
            }
        }
    }
}

/// Clips movement `d` of `mover` along axis `a` against the world. Returns the allowed movement.
pub(crate) fn clip_axis(mover: &Aabb, a: Axis, d: f64, world: &dyn BlockView, shapes: &dyn CollisionShapes) -> f64 {
    if d == 0.0 {
        return 0.0;
    }
    let mut region = *mover;
    match (a, d > 0.0) {
        (Axis::X, true) => region.max.x += d,
        (Axis::X, false) => region.min.x += d,
        (Axis::Y, true) => region.max.y += d,
        (Axis::Y, false) => region.min.y += d,
        (Axis::Z, true) => region.max.z += d,
        (Axis::Z, false) => region.min.z += d,
    }
    let mut d = d;
    let lo = get(mover.min, a);
    let hi = get(mover.max, a);
    for_each_shape(&region, world, shapes, |origin, boxes| {
        for local in boxes {
            let b = local.offset(origin);
            if !overlaps_other_axes(mover, &b, a) {
                continue;
            }
            if d > 0.0 {
                let face = get(b.min, a);
                if hi <= face + COLLISION_EPSILON {
                    let gap = face - hi;
                    if gap < d {
                        d = gap;
                    }
                }
            } else {
                let face = get(b.max, a);
                if lo >= face - COLLISION_EPSILON {
                    let gap = face - lo;
                    if gap > d {
                        d = gap;
                    }
                }
            }
        }
        true
    });
    if d.abs() < COLLISION_EPSILON {
        0.0
    } else {
        d
    }
}

/// Pure axis-separated sweep: Y first, then X/Z ordered by larger magnitude (X first on ties).
/// Returns the movement actually possible. No step-up; see [`collide_with_step`].
pub fn collide(aabb: Aabb, movement: DVec3, world: &dyn BlockView, shapes: &dyn CollisionShapes) -> DVec3 {
    let mut b = aabb;
    let y = clip_axis(&b, Axis::Y, movement.y, world, shapes);
    b = b.offset(DVec3::new(0.0, y, 0.0));
    let (x, z) = if movement.x.abs() < movement.z.abs() {
        let z = clip_axis(&b, Axis::Z, movement.z, world, shapes);
        b = b.offset(DVec3::new(0.0, 0.0, z));
        let x = clip_axis(&b, Axis::X, movement.x, world, shapes);
        (x, z)
    } else {
        let x = clip_axis(&b, Axis::X, movement.x, world, shapes);
        b = b.offset(DVec3::new(x, 0.0, 0.0));
        let z = clip_axis(&b, Axis::Z, movement.z, world, shapes);
        (x, z)
    };
    DVec3::new(x, y, z)
}

#[inline]
fn horizontal_len_sq(v: DVec3) -> f64 {
    v.x * v.x + v.z * v.z
}

/// Sweep with step-up: when the plain sweep is blocked horizontally and the mover is on ground
/// (or lands during this movement), try rising by `step_height`, moving horizontally, and settling
/// back down; keep that result if it covers more horizontal distance.
///
/// Approximation: one candidate height (the full `step_height`, clipped by ceilings). Whether
/// vanilla 26.3 tries several candidate heights is not determined; stairs/slab traces will tell.
pub fn collide_with_step(
    aabb: Aabb,
    movement: DVec3,
    step_height: f64,
    on_ground: bool,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
) -> DVec3 {
    let plain = collide(aabb, movement, world, shapes);
    let blocked_h = plain.x != movement.x || plain.z != movement.z;
    let landed = movement.y < 0.0 && plain.y != movement.y;
    if step_height <= 0.0 || !blocked_h || !(on_ground || landed) {
        return plain;
    }
    let up = clip_axis(&aabb, Axis::Y, step_height, world, shapes);
    if up <= 0.0 {
        return plain;
    }
    let raised = aabb.offset(DVec3::new(0.0, up, 0.0));
    let h = collide(raised, DVec3::new(movement.x, 0.0, movement.z), world, shapes);
    let moved = raised.offset(DVec3::new(h.x, 0.0, h.z));
    let down = clip_axis(&moved, Axis::Y, -up + movement.y.min(0.0), world, shapes);
    let stepped = DVec3::new(h.x, up + down, h.z);
    if horizontal_len_sq(stepped) > horizontal_len_sq(plain) {
        stepped
    } else {
        plain
    }
}

/// True when `aabb` intersects no collision box in the world.
pub fn no_collision(aabb: &Aabb, world: &dyn BlockView, shapes: &dyn CollisionShapes) -> bool {
    let mut free = true;
    for_each_shape(aabb, world, shapes, |origin, boxes| {
        for local in boxes {
            if local.offset(origin).intersects(aabb) {
                free = false;
                return false;
            }
        }
        true
    });
    free
}

/// The block the mover stands on: among blocks whose collision boxes touch the plane just below
/// `aabb`, the one whose cell centre is closest (horizontally) to the box centre. `None` if none.
/// Approximation of vanilla's "supporting block" (documented in NOTES.md).
pub fn supporting_block(aabb: &Aabb, world: &dyn BlockView, shapes: &dyn CollisionShapes) -> Option<BlockPos> {
    let probe = Aabb { min: DVec3::new(aabb.min.x, aabb.min.y - 1e-6, aabb.min.z), max: DVec3::new(aabb.max.x, aabb.min.y, aabb.max.z) };
    let c = aabb.center();
    let mut best: Option<(f64, BlockPos)> = None;
    for_each_shape(&probe, world, shapes, |origin, boxes| {
        if boxes.iter().any(|b| b.offset(origin).intersects(&probe)) {
            let dx = origin.x + 0.5 - c.x;
            let dz = origin.z + 0.5 - c.z;
            let d = dx * dx + dz * dz;
            let pos = BlockPos::new(origin.x as i32, origin.y as i32, origin.z as i32);
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, pos));
            }
        }
        true
    });
    best.map(|(_, p)| p)
}
