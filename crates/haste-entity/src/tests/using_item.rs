//! A remote entity's item use (`LivingEntity.onSyncedDataUpdated` for the living flags,
//! `updatingUsingItem`): started by the synced flag from the held equipment, stopped by a
//! clear or a swapped hand, one recorded step per tick while it runs.
use super::*;

/// Equipment stack: `count` of item `item` in wire slot `slot`.
fn stack(slot: u8, item: u32) -> (u8, haste_core::entity::ItemStackData) {
    (
        slot,
        haste_core::entity::ItemStackData {
            count: 1,
            item: Some(RegistryIndex(item)),
            components_raw: Default::default(),
        },
    )
}

/// Metadata update that only sets the living-flags byte.
fn living_flags(flags: i8) -> Vec<MetadataEntry> {
    vec![MetadataEntry {
        index: 8,
        value: MetadataValue::Byte(flags),
    }]
}

#[test]
fn the_living_flags_start_a_use_from_the_hand_item() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::new(0.0, 64.0, 0.0)));
    // The server syncs the held stack before it sets the flag (`startUsingItem`'s
    // `updatePlayersWithNewEquipment` first).
    s.set_equipment(EntityId(1), 0, stack(0, 4).1);
    s.set_metadata(EntityId(1), &living_flags(0x01));
    // One recorded step per tick while the use runs, carrying the used item and the
    // pose (feet, eye at 1.62 for the default box, rotations).
    for expected_used in 0..3 {
        s.tick();
        let ticks = s.use_ticks();
        assert_eq!(ticks.len(), 1, "tick {expected_used}");
        let t = ticks[0];
        assert_eq!(t.entity, EntityId(1));
        assert_eq!(t.item, RegistryIndex(4));
        assert_eq!(t.used, expected_used);
        assert_eq!(t.pos, DVec3::new(0.0, 64.0, 0.0));
        assert!((t.eye_y - (64.0 + 1.62)).abs() < 1e-6);
        assert_eq!(t.look, (0.0, 0.0));
    }
}

#[test]
fn the_offhand_bit_reads_the_offhand_equipment() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.set_equipment(EntityId(1), 0, stack(0, 4).1);
    s.set_equipment(EntityId(1), 1, stack(1, 7).1);
    // Bit 2: the used hand is the off hand (`getUsedItemHand`).
    s.set_metadata(EntityId(1), &living_flags(0x01 | 0x02));
    s.tick();
    assert_eq!(s.use_ticks()[0].item, RegistryIndex(7));
}

#[test]
fn clearing_the_flag_stops_the_use() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.set_equipment(EntityId(1), 0, stack(0, 4).1);
    s.set_metadata(EntityId(1), &living_flags(0x01));
    s.tick();
    assert_eq!(s.use_ticks().len(), 1);
    // The server clears the flag when the use ends (`stopUsingItem`'s sync).
    s.set_metadata(EntityId(1), &living_flags(0x00));
    s.tick();
    s.tick();
    assert!(s.use_ticks().is_empty(), "no steps after the clear");
    // And a restart counts from zero again.
    s.set_metadata(EntityId(1), &living_flags(0x01));
    s.tick();
    assert_eq!(s.use_ticks()[0].used, 0);
}

#[test]
fn a_swapped_or_emptied_hand_stops_the_use() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.set_equipment(EntityId(1), 0, stack(0, 4).1);
    s.set_metadata(EntityId(1), &living_flags(0x01));
    s.tick();
    // `updatingUsingItem`: the hand no longer holds the item the use started with, so
    // the use stops without waiting for the flag.
    s.set_equipment(EntityId(1), 0, stack(0, 5).1);
    s.tick();
    s.tick();
    assert!(s.use_ticks().is_empty());
    // The flag still set does not restart it (vanilla's `useItem` stayed empty).
    s.tick();
    assert!(s.use_ticks().is_empty());
}

#[test]
fn a_use_without_a_hand_item_never_starts() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    // The flag arriving with an empty hand: vanilla's `useItem` stays empty and the
    // per-tick check stops the use; the equipment arriving later does not start one.
    s.set_metadata(EntityId(1), &living_flags(0x01));
    s.tick();
    s.set_equipment(EntityId(1), 0, stack(0, 4).1);
    s.tick();
    s.tick();
    assert!(s.use_ticks().is_empty());
}

#[test]
fn a_non_living_entity_tracks_no_use() {
    let mut s = store();
    let mut sp = spawn(1, DVec3::ZERO);
    // A type the entity-type table names as non-living: the index-8 byte is not the
    // living-flags byte, so no use is tracked however it reads.
    if let Some(names) = crate::registry::entity_type_names() {
        if let Some((kind, _)) = names.iter().find(|(_, n)| **n == "minecraft:arrow") {
            sp.kind = *kind;
        } else {
            return; // no registry table loaded: the living default would apply
        }
    } else {
        return; // no registry table loaded (no cache): the living default would apply
    }
    s.spawn(&sp);
    s.set_equipment(EntityId(1), 0, stack(0, 4).1);
    s.set_metadata(EntityId(1), &living_flags(0x01));
    s.tick();
    assert!(s.use_ticks().is_empty());
}
