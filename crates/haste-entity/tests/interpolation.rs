//! Remote-entity interpolation against the public `EntityStore` API: position syncs, head yaw,
//! spawn body yaw, baby walk cadence, death, and the yaw/pitch lerp (the vanilla classes are
//! named at each test).
use haste_core::entity::{EntitySpawn, MetadataEntry, MetadataValue};
use haste_core::ids::{EntityId, RegistryIndex};
use haste_core::DVec3;
use haste_entity::{EntityDimensions, EntityStore};
use uuid::Uuid;

fn store() -> EntityStore {
    EntityStore::new(EntityDimensions::default())
}

fn spawn(id: i32, pos: DVec3) -> EntitySpawn {
    EntitySpawn {
        id: EntityId(id),
        uuid: Uuid::new_v4(),
        kind: RegistryIndex(0),
        pos,
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 0.0,
        data: 0,
        velocity: DVec3::ZERO,
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn near_f(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

// ---------------------------------------------------------------------------
// Item 2: entity_position_sync interpolates (ClientPacketListener.handleEntityPositionSync).
// ---------------------------------------------------------------------------

/// A sync within 64 blocks goes through `Entity.moveOrInterpolateTo`: three 1/remaining steps
/// (`LinearInterpolationHandler.doInterpolate`), landing exactly on the third tick.
#[test]
fn position_sync_within_64_blocks_interpolates_over_three_ticks() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.teleport(EntityId(1), DVec3::new(9.0, 0.0, 0.0), 0.0, 0.0, true);
    // Nothing moves until the tick: the renderer must not see a snap.
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(e.pos, DVec3::ZERO);
    assert_eq!(e.prev_pos, DVec3::ZERO);
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert!(near(e.pos.x, 3.0), "tick 1: {}", e.pos.x);
    assert_eq!(e.prev_pos, DVec3::ZERO);
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert!(near(e.pos.x, 6.0), "tick 2: {}", e.pos.x);
    assert!(near(e.prev_pos.x, 3.0));
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(e.pos.x, 9.0);
    s.tick();
    assert_eq!(s.get(EntityId(1)).unwrap().pos.x, 9.0);
}

/// `distanceToSqr > 4096` snaps (`Entity.snapTo`): both positions jump at once and no step
/// follows.
#[test]
fn position_sync_beyond_64_blocks_snaps() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.teleport(EntityId(1), DVec3::new(65.0, 0.0, 0.0), 0.0, 0.0, true);
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(e.pos.x, 65.0);
    assert_eq!(e.prev_pos.x, 65.0);
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(e.pos.x, 65.0);
    assert_eq!(e.prev_pos.x, 65.0);
}

/// Exactly 64 blocks is not "too big" (the test is strictly greater than 4096).
#[test]
fn position_sync_at_exactly_64_blocks_still_interpolates() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.teleport(EntityId(1), DVec3::new(64.0, 0.0, 0.0), 0.0, 0.0, true);
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert!(near(e.pos.x, 64.0 / 3.0), "{}", e.pos.x);
}

/// A sync that lands on the target already in flight does not restart the window
/// (`AbstractInterpolationHandler.interpolateTo` skips an identical target).
#[test]
fn position_sync_to_the_current_target_does_not_restart_the_window() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.move_delta(EntityId(1), DVec3::new(6.0, 0.0, 0.0), true);
    s.tick(); // 2.0
    s.teleport(EntityId(1), DVec3::new(6.0, 0.0, 0.0), 0.0, 0.0, true);
    s.tick(); // still step 2 of 3: half of the remaining 4 -> 4.0
    assert!(near(s.get(EntityId(1)).unwrap().pos.x, 4.0));
    s.tick();
    assert_eq!(s.get(EntityId(1)).unwrap().pos.x, 6.0);
}

/// A delta after a sync builds on the synced position (the position codec's base is the sync).
#[test]
fn delta_after_position_sync_builds_on_the_synced_target() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.teleport(EntityId(1), DVec3::new(10.0, 0.0, 0.0), 0.0, 0.0, true);
    s.move_delta(EntityId(1), DVec3::new(2.0, 0.0, 0.0), true);
    for _ in 0..3 {
        s.tick();
    }
    assert_eq!(s.get(EntityId(1)).unwrap().pos.x, 12.0);
}

#[allow(dead_code)]
fn metadata(index: u8, value: MetadataValue) -> MetadataEntry {
    MetadataEntry { index, value }
}
