//! `ItemEntity.tick`: a dropped item must move from the moment it appears, driven by the velocity
//! the spawn packet carried, without waiting for the first position update; and it must follow
//! vanilla's gravity / move / drag / ground-bounce order once it is on the ground.
use haste_core::entity::{ItemStackData, MetadataEntry, MetadataValue};
use haste_core::ids::{EntityId, RegistryIndex};
use haste_core::view::FluidKind;
use haste_core::DVec3;

use crate::item_entity::{ItemWorld, AIR_DRAG};

use super::{spawn, store};

/// `ItemEntity.DATA_ITEM` (index 8): the item stack. Only `ItemEntity` puts an item stack here,
/// so this also confirms the entity's type when the registry name is unavailable.
fn item_stack() -> MetadataEntry {
    MetadataEntry {
        index: 8,
        value: MetadataValue::ItemStack(ItemStackData {
            count: 1,
            item: Some(RegistryIndex(1)),
            components_raw: Default::default(),
        }),
    }
}

/// Spawn an item and mark it as one, returning its id.
fn item(s: &mut crate::store::EntityStore, id: i32, pos: DVec3, velocity: DVec3) -> EntityId {
    let mut sp = spawn(id, pos);
    sp.velocity = velocity;
    s.spawn(&sp);
    s.set_metadata(EntityId(id), &[item_stack()]);
    EntityId(id)
}

/// A world with a fixed block friction and no fluid, clipping the downward move while on the
/// ground (the stand-in for `move`'s floor collision that `NoWorld` uses).
struct TestWorld {
    friction: f32,
}

impl ItemWorld for TestWorld {
    fn friction_below(&self, _pos: DVec3) -> f32 {
        self.friction
    }

    fn fluid_height(&self, _pos: DVec3, _kind: FluidKind) -> f32 {
        0.0
    }

    fn move_with_collision(&self, _pos: DVec3, delta: DVec3, on_ground: &mut bool) -> DVec3 {
        if *on_ground && delta.y < 0.0 {
            DVec3::new(delta.x, 0.0, delta.z)
        } else {
            delta
        }
    }
}

/// Vanilla's position after `ticks` for an item in the air: gravity 0.04, move by the velocity,
/// then the air drag 0.98 (`ItemEntity.tick`).
fn air_pos(start: DVec3, velocity: DVec3, ticks: usize) -> DVec3 {
    let drag = f64::from(AIR_DRAG);
    let mut v = velocity;
    let mut p = start;
    for _ in 0..ticks {
        v.y -= 0.04;
        p += v;
        v *= drag;
    }
    p
}

/// Vanilla's position after 1, 2 and 5 ticks for an item spawned at `start` with velocity
/// (0, 0.2, 0.1), with no further packets.
#[test]
fn item_moves_from_its_spawn_velocity() {
    let mut s = store();
    let start = DVec3::new(0.0, 100.0, 0.0);
    let id = item(&mut s, 1, start, DVec3::new(0.0, 0.2, 0.1));

    // A plain entity with the same velocity must not move (no client-side simulation).
    s.spawn(&spawn(2, DVec3::ZERO));

    for ticks in 1..=5 {
        s.tick();
        let got = s.get(id).unwrap().pos;
        let want = air_pos(start, DVec3::new(0.0, 0.2, 0.1), ticks);
        assert!(
            (got.x - want.x).abs() < 1e-9
                && (got.y - want.y).abs() < 1e-9
                && (got.z - want.z).abs() < 1e-9,
            "item after {ticks} ticks: got {got:?}, want {want:?}"
        );
        assert_eq!(
            s.get(EntityId(2)).unwrap().pos,
            DVec3::ZERO,
            "a non-item entity must not simulate"
        );
    }
}

/// A resting item must not sink through the floor: the server reports `onGround` and vanilla's
/// `move` stops the downward motion at the floor (the server does not resend a resting item's
/// position).
#[test]
fn resting_item_does_not_sink() {
    let mut s = store();
    let floor = DVec3::new(0.0, 100.0, 0.0);
    let id = item(&mut s, 1, floor, DVec3::ZERO);
    // The item lands: the server's position packet reports it on the ground.
    s.teleport(id, floor, 0.0, 0.0, true);
    for _ in 0..10 {
        s.tick();
    }
    assert_eq!(s.get(id).unwrap().pos.y, floor.y);
}

/// The on-ground skip: a resting item (`onGround`, no horizontal speed) only moves on every 4th
/// tick, `(tickCount + id) % 4 == 0`; the skipped ticks apply gravity but no drag or bounce. With
/// id 0 the first move is tick 4, so ticks 1-3 only accumulate gravity.
#[test]
fn resting_item_skips_three_of_four_ticks() {
    let mut s = store();
    let floor = DVec3::new(0.0, 100.0, 0.0);
    let id = item(&mut s, 0, floor, DVec3::ZERO);
    s.teleport(id, floor, 0.0, 0.0, true);

    for expected in [-0.04, -0.08, -0.12] {
        s.tick();
        let e = s.get(id).unwrap();
        assert!(
            (e.velocity.y - expected).abs() < 1e-12,
            "skipped tick: velocity.y = {}, want {expected}",
            e.velocity.y
        );
        assert_eq!(e.pos.y, floor.y, "a skipped tick must not move");
    }

    // The 4th tick moves: gravity, then drag, then the ground bounce.
    s.tick();
    let e = s.get(id).unwrap();
    let want = -((-0.16) * f64::from(AIR_DRAG)) * 0.5;
    assert!(
        (e.velocity.y - want).abs() < 1e-12,
        "4th tick: velocity.y = {}, want {want}",
        e.velocity.y
    );
    assert_eq!(e.pos.y, floor.y);
}

/// The ground bounce: on the ground with a downward velocity, vanilla multiplies y by -0.5 after
/// the drag (`ItemEntity.tick`). A horizontal speed above 1e-5 keeps the item out of the skip.
#[test]
fn ground_bounce_halves_downward_velocity() {
    let mut s = store();
    let floor = DVec3::new(0.0, 100.0, 0.0);
    let id = item(&mut s, 1, floor, DVec3::ZERO);
    s.teleport(id, floor, 0.0, 0.0, true);
    s.set_velocity(id, DVec3::new(0.1, -0.4, 0.0));

    s.tick();
    let e = s.get(id).unwrap();
    // gravity -> -0.44; move clips y at the floor; drag 0.98; bounce -0.5.
    let want_y = -((-0.44) * f64::from(AIR_DRAG)) * 0.5;
    assert!(
        (e.velocity.y - want_y).abs() < 1e-12,
        "bounce: velocity.y = {}, want {want_y}",
        e.velocity.y
    );
    // The horizontal drag on the ground is airDrag x friction (0.98 x 0.6), both floats.
    let want_x = 0.1 * f64::from(AIR_DRAG * 0.6_f32);
    assert!(
        (e.velocity.x - want_x).abs() < 1e-12,
        "ground drag: velocity.x = {}, want {want_x}",
        e.velocity.x
    );
    assert_eq!(e.pos.y, floor.y);
}

/// Ice friction: on the ground the horizontal drag is `airDrag * Block.getFriction` (0.98 x 0.98
/// on ice), not the air drag.
#[test]
fn ice_friction_slows_the_item_less() {
    let mut s = store();
    let floor = DVec3::new(0.0, 100.0, 0.0);
    let id = item(&mut s, 1, floor, DVec3::ZERO);
    s.teleport(id, floor, 0.0, 0.0, true);
    s.set_velocity(id, DVec3::new(0.1, 0.0, 0.0));

    s.tick_in_world(&TestWorld { friction: 0.98 });
    let e = s.get(id).unwrap();
    let want_x = 0.1 * f64::from(AIR_DRAG * 0.98_f32);
    assert!(
        (e.velocity.x - want_x).abs() < 1e-12,
        "ice drag: velocity.x = {}, want {want_x}",
        e.velocity.x
    );
}
