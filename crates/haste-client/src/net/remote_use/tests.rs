//! A remote entity's use effects: the crumbs a bread-eating entity spawns (five per emit,
//! every fourth tick past vanilla's start delay, positioned from the entity's eye and
//! rotations — `LivingEntity.spawnItemParticles`), no local sound (the server's broadcast
//! carries it), nothing for an item without a consumable.

use std::sync::{Arc, OnceLock};

use glam::DVec3;
use haste_core::entity::{EntitySpawn, ItemStackData, MetadataEntry, MetadataValue};
use haste_core::ids::{EntityId, RegistryIndex};
use haste_core::Identifier;
use haste_data::{Consumable, GameData};
use haste_protocol::haste_core::game::{DimensionInfo, GameMode};
use haste_protocol::ClientboundPlay;

use super::*;
use crate::net::new_entity_store;

/// The local player's entity id in these tests (never the eater).
const OWN: i32 = 7;
/// The bread item's registry index in the test data.
const BREAD: RegistryIndex = RegistryIndex(0);
/// Bread's use duration (`Consumable.consumeTicks`, 1.6 s).
const BREAD_TICKS: i32 = 32;

/// A small GameData (no cache needed): bread and stone items, bread a food consumable.
fn data() -> &'static Arc<GameData> {
    static D: OnceLock<Arc<GameData>> = OnceLock::new();
    D.get_or_init(|| {
        let dir =
            std::env::temp_dir().join(format!("haste-remote-use-test-{}", std::process::id()));
        let reports = dir.join("reports");
        std::fs::create_dir_all(&reports).unwrap();
        std::fs::write(
            reports.join("blocks.json"),
            r#"{"minecraft:air": {"states": [{"id": 0, "default": true}]}}"#,
        )
        .unwrap();
        std::fs::write(
            reports.join("registries.json"),
            r#"{"minecraft:block": {"entries": {"minecraft:air": {"protocol_id": 0}}},
                "minecraft:item": {"entries": {"minecraft:bread": {"protocol_id": 0},
                                               "minecraft:stone": {"protocol_id": 1}}}}"#,
        )
        .unwrap();
        let mut d = GameData::load_from(&reports, None).unwrap();
        d.item_components
            .insert_consumable(Identifier::minecraft("bread"), Consumable::default_food());
        Arc::new(d)
    })
}

fn session() -> Session {
    Session::new(
        EntityId(OWN),
        GameMode::Creative,
        DimensionInfo::overworld(),
        Arc::new(Vec::new()),
    )
}

/// A remote living entity (kind 0 is always treated as living) at `pos`.
fn spawn_remote(store: &mut haste_entity::EntityStore, id: i32, pos: DVec3) {
    store.spawn(&EntitySpawn {
        id: EntityId(id),
        uuid: Default::default(),
        kind: RegistryIndex(0),
        pos,
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 0.0,
        data: 0,
        velocity: DVec3::ZERO,
    });
}

fn equipment(item: RegistryIndex) -> ItemStackData {
    ItemStackData {
        count: 1,
        item: Some(item),
        components_raw: Default::default(),
    }
}

fn living_flags(flags: i8) -> Vec<MetadataEntry> {
    vec![MetadataEntry {
        index: 8,
        value: MetadataValue::Byte(flags),
    }]
}

/// One 20 Hz tick of a watching client: the entities' step, then the use effects. The
/// crumbs this tick queued.
fn tick(s: &mut Session, store: &mut haste_entity::EntityStore) -> usize {
    store.tick();
    s.apply_remote_use(store.use_ticks(), data());
    let crumbs = s.item_particle_requests.len();
    s.item_particle_requests.clear();
    crumbs
}

/// A remote entity eating bread emits five crumbs every fourth tick past vanilla's start
/// delay (`Consumable.shouldEmitParticlesAndSounds`: past 21.875 % of the 32-tick use and
/// every fourth remaining tick); stopping clears it, and no sound request is made (the
/// server's broadcast sound packets carry the sound, `ClientLevel.playSeededSound` plays
/// no local one for another entity).
#[test]
fn a_remote_bread_eater_emits_its_crumbs_every_fourth_tick() {
    let mut s = session();
    let mut store = new_entity_store(0);
    spawn_remote(&mut store, 5, DVec3::new(10.0, 70.0, 10.0));
    store.set_equipment(EntityId(5), 0, equipment(BREAD));
    store.set_metadata(EntityId(5), &living_flags(0x01));
    let mut emit_ticks = Vec::new();
    for t in 1..=BREAD_TICKS {
        if tick(&mut s, &mut store) > 0 {
            emit_ticks.push(t);
        }
    }
    assert_eq!(
        emit_ticks,
        vec![9, 13, 17, 21, 25, 29],
        "used 8, 12, 16, 20, 24, 28 — six emits, like the local player's own bread use"
    );
    // The flag clears when the use ends: no further emits.
    store.set_metadata(EntityId(5), &living_flags(0x00));
    for _ in 0..10 {
        assert_eq!(tick(&mut s, &mut store), 0);
    }
    // The watcher heard nothing locally: the eat sound comes from the server's packets.
    assert!(s.sound_requests.is_empty());
}

/// The crumbs a remote eater spawns sit in front of its mouth (`spawnItemParticles`: the
/// eye plus the rotated offsets — the entity looks at yaw 0, pitch 0, so the 0.6 forward
/// offset lands on +z, the +-0.15 jitter on x, the drop up to 0.9 below the eye) and
/// carry the used stack's item.
#[test]
fn the_crumbs_sit_in_front_of_the_eaters_mouth() {
    let mut s = session();
    let mut store = new_entity_store(0);
    let pos = DVec3::new(10.0, 70.0, 10.0);
    spawn_remote(&mut store, 5, pos);
    store.set_equipment(EntityId(5), 0, equipment(BREAD));
    store.set_metadata(EntityId(5), &living_flags(0x01));
    let mut crumbs = Vec::new();
    for _ in 0..BREAD_TICKS {
        store.tick();
        s.apply_remote_use(store.use_ticks(), data());
        crumbs.append(&mut s.item_particle_requests);
    }
    assert_eq!(crumbs.len(), 30, "six emits x five crumbs");
    // Each emit's five crumbs share the eye of that tick; their drop spans 0.6.
    for group in crumbs.chunks(5) {
        let ys: Vec<f64> = group.iter().map(|c| c.pos.y).collect();
        let lo = ys.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        assert!(hi - lo <= 0.6 + 1e-6, "one eye per emit: {ys:?}");
    }
    for c in &crumbs {
        assert_eq!(c.item, BREAD, "the used stack's item");
        assert!((c.pos.z - (pos.z + 0.6)).abs() < 1e-9, "z {:?}", c.pos.z);
        assert!((c.pos.x - pos.x).abs() <= 0.15 + 1e-6, "x {:?}", c.pos.x);
        // The velocity's y part is the upward draw (0.1..0.2) plus the 0.05 lift.
        assert!((0.15..=0.25).contains(&c.vel[1]), "vy {:?}", c.vel);
        assert!(c.vel[0].abs() <= 0.05 + 1e-6, "vx {:?}", c.vel);
    }
}

/// Each remote eater draws its own crumb positions (vanilla's per-entity `RandomSource`),
/// and an item without a consumable (stone) emits nothing at all.
#[test]
fn each_eater_draws_its_own_and_a_non_consumable_emits_nothing() {
    let mut s = session();
    let mut store = new_entity_store(0);
    spawn_remote(&mut store, 5, DVec3::new(10.0, 70.0, 10.0));
    spawn_remote(&mut store, 6, DVec3::new(40.0, 70.0, 10.0));
    spawn_remote(&mut store, 8, DVec3::new(70.0, 70.0, 10.0));
    for id in [5, 6] {
        store.set_equipment(EntityId(id), 0, equipment(BREAD));
        store.set_metadata(EntityId(id), &living_flags(0x01));
    }
    // Stone: no `minecraft:consumable`, so the use emits nothing (`ItemStack.onUseTick`'s
    // guard) however long it runs.
    store.set_equipment(EntityId(8), 0, equipment(RegistryIndex(1)));
    store.set_metadata(EntityId(8), &living_flags(0x01));
    let mut first_crumbs = Vec::new();
    for t in 1..=BREAD_TICKS {
        store.tick();
        s.apply_remote_use(store.use_ticks(), data());
        if t == 9 {
            first_crumbs = std::mem::take(&mut s.item_particle_requests);
        } else {
            s.item_particle_requests.clear();
        }
    }
    assert_eq!(first_crumbs.len(), 10, "two eaters x five crumbs");
    // The eaters' crumbs cluster around their own mouths: entity 5 sits at x 10, entity 6
    // at x 40, and the x jitter is +-0.15, so every crumb is unambiguous.
    let near = |c: &crate::item_particles::ItemParticleRequest, x: f64| (c.pos.x - x).abs() < 1.0;
    assert!(
        first_crumbs.iter().any(|c| near(c, 10.0)),
        "entity 5's crumbs"
    );
    assert!(
        first_crumbs.iter().any(|c| near(c, 40.0)),
        "entity 6's crumbs"
    );
    assert!(
        first_crumbs.iter().all(|c| near(c, 10.0) || near(c, 40.0)),
        "no crumb between the two eaters"
    );
    // The stone user emitted nothing over the whole use, and no sound anywhere.
    assert!(s.sound_requests.is_empty());
    assert_eq!(s.item_particle_requests.len(), 0);
}

/// The per-entity draw generators live as long as the entity: `remove_entities` drops
/// them.
#[test]
fn removing_an_entity_drops_its_draws() {
    let mut s = session();
    let mut store = new_entity_store(0);
    spawn_remote(&mut store, 5, DVec3::ZERO);
    store.set_equipment(EntityId(5), 0, equipment(BREAD));
    store.set_metadata(EntityId(5), &living_flags(0x01));
    // The draw generator appears with the first emit (tick 9, used 8).
    for _ in 0..9 {
        tick(&mut s, &mut store);
    }
    assert!(s.remote_use.contains_key(&EntityId(5)));
    let mut world =
        haste_world::World::new_with_biomes(DimensionInfo::overworld(), data().clone(), 4);
    s.apply(
        ClientboundPlay::RemoveEntities(
            haste_protocol::packets::play::clientbound::RemoveEntities {
                entity_ids: vec![5],
            },
        ),
        &mut world,
        &mut store,
        &haste_protocol::SyncedRegistries::default(),
        &mut Vec::new(),
    );
    assert!(
        !s.remote_use.contains_key(&EntityId(5)),
        "dropped with the entity"
    );
}
