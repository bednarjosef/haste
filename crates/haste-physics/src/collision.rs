//! Axis-separated AABB sweep against block collision shapes, plus step-up.
//!
//! Pure functions: they read the world through `BlockView` + `CollisionShapes` and never allocate.
//! A call first gathers the world-space boxes in the whole swept region into a fixed-size stack
//! buffer (`BOX_CAPACITY`), then clips each axis against that list. If the region holds more boxes
//! than fit (very long movements), each axis pass scans the world directly instead; both paths
//! give bit-identical results.
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
enum Axis {
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

/// Boxes gathered on the stack per call. 0.6 x 1.8 player moving a tick at sprint/terminal speed
/// touches well under this; longer movements fall back to scanning the world per axis.
pub const BOX_CAPACITY: usize = 96;

/// Where the boxes come from: a gathered list, or the world itself (fallback).
#[derive(Clone, Copy)]
enum Source<'a> {
    List(&'a [Aabb]),
    World(&'a dyn BlockView, &'a dyn CollisionShapes),
}

struct BoxBuf {
    boxes: Vec<Aabb>,
}

thread_local! {
    /// Reusable per-thread scratch buffer: allocated once, never shrunk.
    static SCRATCH: std::cell::RefCell<BoxBuf> = std::cell::RefCell::new(BoxBuf { boxes: Vec::with_capacity(BOX_CAPACITY) });
}

/// Gathers world-space collision boxes intersecting `region` (or touching it within the epsilon).
/// Returns `false` when they do not fit in the buffer.
fn gather(region: &Aabb, world: &dyn BlockView, shapes: &dyn CollisionShapes, buf: &mut BoxBuf) -> bool {
    let grown = region.expand(DVec3::splat(COLLISION_EPSILON));
    let mut ok = true;
    buf.boxes.clear();
    for_each_shape(region, world, shapes, |origin, boxes| {
        for local in boxes {
            let b = local.offset(origin);
            if !b.intersects(&grown) {
                continue;
            }
            if buf.boxes.len() == BOX_CAPACITY {
                ok = false;
                return false;
            }
            buf.boxes.push(b);
        }
        true
    });
    ok
}

/// Clips `d` against one world-space box.
#[inline]
fn clip_one(mover: &Aabb, b: &Aabb, a: Axis, d: f64) -> f64 {
    if !overlaps_other_axes(mover, b, a) {
        return d;
    }
    if d > 0.0 {
        let face = get(b.min, a);
        let hi = get(mover.max, a);
        if hi <= face + COLLISION_EPSILON {
            let gap = face - hi;
            if gap < d {
                return gap;
            }
        }
    } else {
        let face = get(b.max, a);
        let lo = get(mover.min, a);
        if lo >= face - COLLISION_EPSILON {
            let gap = face - lo;
            if gap > d {
                return gap;
            }
        }
    }
    d
}

fn swept_region(mover: &Aabb, a: Axis, d: f64) -> Aabb {
    let mut region = *mover;
    match (a, d > 0.0) {
        (Axis::X, true) => region.max.x += d,
        (Axis::X, false) => region.min.x += d,
        (Axis::Y, true) => region.max.y += d,
        (Axis::Y, false) => region.min.y += d,
        (Axis::Z, true) => region.max.z += d,
        (Axis::Z, false) => region.min.z += d,
    }
    region
}

/// Clips movement `d` of `mover` along axis `a`. Returns the allowed movement.
fn clip(src: Source, mover: &Aabb, a: Axis, d: f64) -> f64 {
    if d == 0.0 {
        return 0.0;
    }
    let mut d = d;
    match src {
        Source::List(boxes) => {
            for b in boxes {
                d = clip_one(mover, b, a, d);
            }
        }
        Source::World(world, shapes) => {
            for_each_shape(&swept_region(mover, a, d), world, shapes, |origin, boxes| {
                for local in boxes {
                    d = clip_one(mover, &local.offset(origin), a, d);
                }
                true
            });
        }
    }
    if d.abs() < COLLISION_EPSILON {
        0.0
    } else {
        d
    }
}

/// Region covered by moving `aabb` by `m` (the box stretched towards the movement).
fn stretch(aabb: &Aabb, m: DVec3) -> Aabb {
    Aabb { min: aabb.min + m.min(DVec3::ZERO), max: aabb.max + m.max(DVec3::ZERO) }
}

fn with_boxes<R>(region: &Aabb, world: &dyn BlockView, shapes: &dyn CollisionShapes, f: impl FnOnce(Source) -> R) -> R {
    SCRATCH.with(|cell| match cell.try_borrow_mut() {
        Ok(mut buf) => {
            if gather(region, world, shapes, &mut buf) {
                f(Source::List(&buf.boxes))
            } else {
                f(Source::World(world, shapes))
            }
        }
        // Re-entrant call (a BlockView calling back into physics): scan the world directly.
        Err(_) => f(Source::World(world, shapes)),
    })
}

/// Pure axis-separated sweep: Y first, then X/Z ordered by larger magnitude (X first on ties).
/// Returns the movement actually possible. No step-up; see [`collide_with_step`].
pub fn collide(aabb: Aabb, movement: DVec3, world: &dyn BlockView, shapes: &dyn CollisionShapes) -> DVec3 {
    if movement == DVec3::ZERO {
        return movement;
    }
    with_boxes(&stretch(&aabb, movement), world, shapes, |src| sweep(src, aabb, movement))
}

fn sweep(src: Source, aabb: Aabb, movement: DVec3) -> DVec3 {
    let mut b = aabb;
    let y = clip(src, &b, Axis::Y, movement.y);
    b = b.offset(DVec3::new(0.0, y, 0.0));
    let (x, z) = if movement.x.abs() < movement.z.abs() {
        let z = clip(src, &b, Axis::Z, movement.z);
        b = b.offset(DVec3::new(0.0, 0.0, z));
        let x = clip(src, &b, Axis::X, movement.x);
        (x, z)
    } else {
        let x = clip(src, &b, Axis::X, movement.x);
        b = b.offset(DVec3::new(x, 0.0, 0.0));
        let z = clip(src, &b, Axis::Z, movement.z);
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
    if movement == DVec3::ZERO {
        return movement;
    }
    let mut region = stretch(&aabb, movement);
    if step_height > 0.0 {
        region.max.y = region.max.y.max(aabb.max.y + step_height);
    }
    with_boxes(&region, world, shapes, |src| sweep_with_step(src, aabb, movement, step_height, on_ground))
}

fn sweep_with_step(src: Source, aabb: Aabb, movement: DVec3, step_height: f64, on_ground: bool) -> DVec3 {
    let plain = sweep(src, aabb, movement);
    let blocked_h = plain.x != movement.x || plain.z != movement.z;
    let landed = movement.y < 0.0 && plain.y != movement.y;
    if step_height <= 0.0 || !blocked_h || !(on_ground || landed) {
        return plain;
    }
    let up = clip(src, &aabb, Axis::Y, step_height);
    if up <= 0.0 {
        return plain;
    }
    let raised = aabb.offset(DVec3::new(0.0, up, 0.0));
    let h = sweep(src, raised, DVec3::new(movement.x, 0.0, movement.z));
    let moved = raised.offset(DVec3::new(h.x, 0.0, h.z));
    let down = clip(src, &moved, Axis::Y, -up + movement.y.min(0.0));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{states::*, TestShapes, TestWorld};

    /// The gathered-list path and the direct world scan must agree bit for bit.
    #[test]
    fn list_and_world_paths_agree() {
        let mut w = TestWorld::flat(0, STONE);
        for i in -6..6 {
            w.set(i, 0, (i * 3) % 5, BOTTOM_SLAB).set(2, 0, i, STAIR_NORTH).set(i, 1, -3, FENCE_POST).set(-3, 0, i, STONE);
        }
        let s = TestShapes::default();
        let mut seed = 0x2545_f491_u64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 10_000) as f64 / 10_000.0
        };
        for _ in 0..5000 {
            let p = DVec3::new(rnd() * 8.0 - 4.0, rnd() * 1.5, rnd() * 8.0 - 4.0);
            let b = Aabb::entity(p, 0.6, 1.8);
            let m = DVec3::new(rnd() - 0.5, rnd() - 0.6, rnd() - 0.5);
            let region = stretch(&b, m);
            let mut buf = BoxBuf { boxes: Vec::new() };
            let mut big = region;
            big.max.y = big.max.y.max(b.max.y + 0.6);
            if !gather(&big, &w, &s, &mut buf) {
                continue;
            }
            let world = Source::World(&w, &s);
            let list = Source::List(&buf.boxes);
            assert_eq!(sweep(world, b, m), sweep(list, b, m), "{p:?} {m:?}");
            assert_eq!(sweep_with_step(world, b, m, 0.6, true), sweep_with_step(list, b, m, 0.6, true), "{p:?} {m:?}");
        }
    }
}
