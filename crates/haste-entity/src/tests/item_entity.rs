//! `ItemEntity.tick`: a dropped item must move from the moment it appears, driven by the velocity
//! the spawn packet carried, without waiting for the first position update.
use haste_core::entity::{ItemStackData, MetadataEntry, MetadataValue};
use haste_core::ids::{EntityId, RegistryIndex};
use haste_core::DVec3;

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

/// Vanilla's position after 1, 2 and 5 ticks for an item spawned at `start` with velocity
/// (0, 0.2, 0.1), with no further packets: gravity 0.04, then move by the velocity, then drag
/// 0.98 (`ItemEntity.tick`).
#[test]
fn item_moves_from_its_spawn_velocity() {
    let mut s = store();
    let start = DVec3::new(0.0, 100.0, 0.0);
    let mut sp = spawn(1, start);
    sp.velocity = DVec3::new(0.0, 0.2, 0.1);
    s.spawn(&sp);
    s.set_metadata(EntityId(1), &[item_stack()]);

    // A plain entity with the same velocity must not move (no client-side simulation).
    s.spawn(&spawn(2, DVec3::ZERO));

    let expected = |ticks: usize| {
        let mut v = DVec3::new(0.0, 0.2, 0.1);
        let mut p = start;
        for _ in 0..ticks {
            v.y -= 0.04;
            p += v;
            v *= 0.98;
        }
        p
    };

    for ticks in 1..=5 {
        s.tick();
        let got = s.get(EntityId(1)).unwrap().pos;
        let want = expected(ticks);
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
    let mut sp = spawn(1, floor);
    sp.velocity = DVec3::ZERO;
    s.spawn(&sp);
    s.set_metadata(EntityId(1), &[item_stack()]);
    // The item lands: the server's position packet reports it on the ground.
    s.teleport(EntityId(1), floor, 0.0, 0.0, true);
    for _ in 0..10 {
        s.tick();
    }
    assert_eq!(s.get(EntityId(1)).unwrap().pos.y, floor.y);
}
