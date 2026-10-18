//! Pickup tests: the `take_item_entity` sound, the stack shrink / removal and the particle
//! easing, against the real `EntityStore` (no network, no fakes for the store).

use std::sync::Arc;

use glam::DVec3;
use haste_core::entity::{EntitySpawn, ItemStackData, MetadataEntry, MetadataValue};
use haste_core::{EntityId, Identifier, RegistryIndex};
use haste_protocol::haste_core::game::{DimensionInfo, GameMode};
use haste_protocol::packets::play::clientbound as cb;

use super::*;
use crate::net::{new_entity_store, Session};
use crate::sound::{SoundRef, SoundRequest};

/// The local player's entity id in these tests (the collector).
const OWN: i32 = 7;

fn session(types: &[&str]) -> Session {
    let mut s = Session::new(
        EntityId(OWN),
        GameMode::Creative,
        DimensionInfo::overworld(),
        Arc::new(Vec::new()),
    );
    s.set_entity_type_names(types.iter().map(|n| Identifier::minecraft(n)).collect());
    s
}

fn spawn_item(store: &mut haste_entity::EntityStore, kind: u32, pos: DVec3) -> EntityId {
    let id = EntityId(5);
    store.spawn(&EntitySpawn {
        id,
        uuid: Default::default(),
        kind: RegistryIndex(kind),
        pos,
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 0.0,
        data: 0,
        velocity: DVec3::ZERO,
    });
    id
}

fn stack(count: i32, item: u32) -> MetadataValue {
    MetadataValue::ItemStack(ItemStackData {
        count,
        item: Some(RegistryIndex(item)),
        components_raw: Default::default(),
    })
}

fn take(collected: i32, count: i32) -> cb::TakeItemEntity {
    cb::TakeItemEntity {
        collected,
        collector: OWN,
        count,
    }
}

/// The single play request the handler recorded.
fn play(s: &Session) -> (&str, i32, f32, f32) {
    assert_eq!(s.sound_requests.len(), 1, "exactly one pickup sound");
    match &s.sound_requests[0] {
        SoundRequest::Play(p) => match &p.sound {
            SoundRef::Inline(name) => (name.as_str(), p.category, p.volume, p.pitch),
            other => panic!("expected an inline event, got {other:?}"),
        },
        other => panic!("expected a play, got {other:?}"),
    }
}

#[test]
fn item_pickup_plays_the_item_sound_and_shrinks_the_stack() {
    let mut s = session(&["item"]);
    let mut store = new_entity_store(8);
    let id = spawn_item(&mut store, 0, DVec3::new(1.0, 2.0, 3.0));
    store.set_metadata(
        id,
        &[MetadataEntry {
            index: 8,
            value: stack(5, 1),
        }],
    );

    s.take_item_entity(take(5, 2), &mut store);

    let (name, category, volume, pitch) = play(&s);
    assert_eq!(name, "minecraft:entity.item.pickup");
    assert_eq!(category, 7, "PLAYERS");
    assert_eq!(volume, 0.2);
    assert!((0.6..=3.4).contains(&pitch), "pitch {pitch} out of range");
    // The stack shrank by the amount, the entity stays.
    let e = store.get(id).expect("still present");
    assert_eq!(e.item.as_ref().unwrap().count, 3);
    // One particle, flying from the item to the player.
    assert_eq!(s.pickup_particles.len(), 1);
    let p = &s.pickup_particles[0];
    assert_eq!(p.id, id);
    assert_eq!(
        p.visual,
        PickupVisual::Item {
            item: RegistryIndex(1),
            count: 5
        }
    );
    assert_eq!(p.start, DVec3::new(1.0, 2.0, 3.0));
}

#[test]
fn item_pickup_removes_an_emptied_stack() {
    let mut s = session(&["item"]);
    let mut store = new_entity_store(8);
    let id = spawn_item(&mut store, 0, DVec3::new(1.0, 2.0, 3.0));
    store.set_metadata(
        id,
        &[MetadataEntry {
            index: 8,
            value: stack(2, 1),
        }],
    );

    s.take_item_entity(take(5, 2), &mut store);

    assert!(store.get(id).is_none(), "an emptied stack is removed");
}

#[test]
fn orb_pickup_plays_the_orb_sound_and_leaves_the_entity() {
    let mut s = session(&["experience_orb"]);
    let mut store = new_entity_store(8);
    let id = spawn_item(&mut store, 0, DVec3::new(1.0, 2.0, 3.0));
    store.set_metadata(
        id,
        &[MetadataEntry {
            index: 8,
            value: MetadataValue::VarInt(7),
        }],
    );

    s.take_item_entity(take(5, 1), &mut store);

    let (name, category, volume, pitch) = play(&s);
    assert_eq!(name, "minecraft:entity.experience_orb.pickup");
    assert_eq!(category, 7, "PLAYERS");
    assert_eq!(volume, 0.1);
    assert!((0.55..=1.25).contains(&pitch), "pitch {pitch} out of range");
    assert!(store.get(id).is_some(), "an orb is left for the server");
    assert_eq!(s.pickup_particles[0].visual, PickupVisual::Orb { value: 7 });
}

#[test]
fn a_silent_entity_plays_no_sound() {
    let mut s = session(&["item"]);
    let mut store = new_entity_store(8);
    let id = spawn_item(&mut store, 0, DVec3::new(1.0, 2.0, 3.0));
    store.set_metadata(
        id,
        &[
            MetadataEntry {
                index: 8,
                value: stack(2, 1),
            },
            MetadataEntry {
                index: 4,
                value: MetadataValue::Boolean(true),
            },
        ],
    );

    s.take_item_entity(take(5, 1), &mut store);

    assert!(s.sound_requests.is_empty(), "silent: no pickup sound");
    assert_eq!(s.pickup_particles.len(), 1, "the particle still shows");
}

#[test]
fn another_type_is_removed_without_a_stack() {
    let mut s = session(&["arrow"]);
    let mut store = new_entity_store(8);
    let id = spawn_item(&mut store, 0, DVec3::new(1.0, 2.0, 3.0));

    s.take_item_entity(take(5, 1), &mut store);

    assert!(
        store.get(id).is_none(),
        "a non-item, non-orb entity is removed"
    );
    assert!(s.pickup_particles.is_empty(), "no item/orb visual to draw");
}

#[test]
fn pickup_position_matches_the_vanilla_easing() {
    let p = PickupParticle {
        id: EntityId(5),
        visual: PickupVisual::Item {
            item: RegistryIndex(1),
            count: 1,
        },
        age: 0.0,
        start: DVec3::ZERO,
        target_old: DVec3::splat(3.0),
        target: DVec3::splat(6.0),
        collector: EntityId(OWN),
        life: 0,
    };
    // life 0, partial 1: t = (1/3)^2 = 1/9, target = 6, pos = 6/9 = 2/3.
    assert!((pickup_position(&p, 1.0) - DVec3::splat(2.0 / 3.0)).length() < 1e-6);
    // life 1, partial 0.5: t = (1.5/3)^2 = 0.25, target = 4.5, pos = 1.125.
    let mut p = p;
    p.life = 1;
    assert!((pickup_position(&p, 0.5) - DVec3::splat(1.125)).length() < 1e-6);
}

#[test]
fn particles_tick_toward_the_collector_and_expire_after_three_ticks() {
    let mut s = session(&["item"]);
    let mut store = new_entity_store(8);
    let id = spawn_item(&mut store, 0, DVec3::new(1.0, 2.0, 3.0));
    store.set_metadata(
        id,
        &[MetadataEntry {
            index: 8,
            value: stack(1, 1),
        }],
    );
    s.take_item_entity(take(5, 1), &mut store);
    assert_eq!(s.pickup_particles[0].life, 0);

    s.tick_pickup_particles(&store);
    assert_eq!(s.pickup_particles[0].life, 1);
    s.tick_pickup_particles(&store);
    assert_eq!(s.pickup_particles[0].life, 2);
    s.tick_pickup_particles(&store);
    assert!(s.pickup_particles.is_empty(), "removed at life 3");
}
