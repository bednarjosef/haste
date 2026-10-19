//! Thrown-item state: the per-type registration vanilla's `EntityRenderers` carries (scale,
//! full-bright, the default item) and the item the synched stack or the default resolves to
//! (26.3 source, `net.minecraft.client.renderer.entity.EntityRenderers`,
//! `net.minecraft.world.entity.projectile.throwableitemprojectile.ThrowableItemProjectile`).
use haste_core::entity::{ItemStackData, MetadataEntry, MetadataValue};
use haste_core::ids::EntityId;
use haste_core::model::{Decoration, EntityInstance, ThrownItemDecoration};
use haste_core::{DVec3, RegistryIndex};

use crate::metadata::Metadata;
use crate::thrown_item;

fn metadata(entries: Vec<MetadataEntry>) -> Metadata {
    let mut m = Metadata::default();
    m.apply(&entries);
    m
}

fn stack(index: u8, item: Option<u32>) -> MetadataEntry {
    MetadataEntry {
        index,
        value: MetadataValue::ItemStack(ItemStackData {
            count: 1,
            item: item.map(RegistryIndex),
            components_raw: Default::default(),
        }),
    }
}

/// The registry id of an entity type name (reverse of `registry::entity_type_name`).
fn kind_of(name: &str) -> RegistryIndex {
    *crate::registry::entity_type_names()
        .expect("registries.json")
        .iter()
        .find(|&(_, n)| *n == name)
        .map(|(k, _)| k)
        .unwrap_or_else(|| panic!("no entity type {name}"))
}

/// Every type vanilla registers with a `ThrownItemRenderer`, with its constructor arguments
/// (`EntityRenderers`); scale 1 and no full-bright is the plain constructor, the eye of
/// ender and the two fireballs pass their own.
#[test]
fn registration_matches_vanilla() {
    let cases = [
        ("minecraft:egg", 1.0, false),
        ("minecraft:ender_pearl", 1.0, false),
        ("minecraft:experience_bottle", 1.0, false),
        ("minecraft:lingering_potion", 1.0, false),
        ("minecraft:snowball", 1.0, false),
        ("minecraft:splash_potion", 1.0, false),
        ("minecraft:eye_of_ender", 1.0, true),
        ("minecraft:fireball", 3.0, true),
        ("minecraft:small_fireball", 0.75, true),
    ];
    for (name, scale, full_bright) in cases {
        let t = thrown_item::variant_for_name(name).unwrap_or_else(|| panic!("{name}"));
        assert_eq!((t.scale, t.full_bright), (scale, full_bright), "{name}");
    }
    // Everything else (including the types with their own renderers) is not a thrown item.
    assert!(thrown_item::variant_for_name("minecraft:zombie").is_none());
    assert!(thrown_item::variant_for_name("minecraft:dragon_fireball").is_none());
    assert!(thrown_item::variant_for_name("minecraft:item").is_none());
    assert!(thrown_item::variant_for_name("minecraft:arrow").is_none());
}

/// A non-empty synched stack (`DATA_ITEM_STACK`, slot 8) picks the item: a custom splash
/// potion draws its own item, not the type's default.
#[test]
fn decoration_uses_the_synced_stack() {
    let t = thrown_item::variant_for_name("minecraft:splash_potion").unwrap();
    let m = metadata(vec![stack(8, Some(900))]);
    assert_eq!(
        thrown_item::decoration(&t, &m),
        Some(ThrownItemDecoration {
            item: RegistryIndex(900),
            scale: 1.0,
            full_bright: false,
        })
    );
    // An emptied stack (which vanilla never sends: the slot is defined from the type's
    // default item) falls back to the type's default.
    let m = metadata(vec![stack(8, None)]);
    assert_eq!(
        thrown_item::decoration(&t, &m).map(|d| d.full_bright),
        Some(false)
    );
}

/// Without a stack the type's default item resolves through the item registry: the
/// snowball's `getDefaultItem` is `minecraft:snowball`, the fireballs' `FIRE_CHARGE`, the
/// eye of ender's `ENDER_EYE`.
#[test]
#[ignore = "needs cache/datagen (registries.json; HASTE_CACHE overrides)"]
fn decoration_falls_back_to_the_default_item() {
    let items = thrown_item::item_indices().expect("registries.json");
    for (name, default) in [
        ("minecraft:snowball", "minecraft:snowball"),
        ("minecraft:egg", "minecraft:egg"),
        ("minecraft:ender_pearl", "minecraft:ender_pearl"),
        ("minecraft:experience_bottle", "minecraft:experience_bottle"),
        ("minecraft:eye_of_ender", "minecraft:ender_eye"),
        ("minecraft:splash_potion", "minecraft:splash_potion"),
        ("minecraft:lingering_potion", "minecraft:lingering_potion"),
        ("minecraft:fireball", "minecraft:fire_charge"),
        ("minecraft:small_fireball", "minecraft:fire_charge"),
    ] {
        let t = thrown_item::variant_for_name(name).unwrap();
        let d = thrown_item::decoration(&t, &Metadata::default())
            .unwrap_or_else(|| panic!("{name} resolves no item"));
        assert_eq!(Some(&d.item), items.get(default), "{name}");
    }
}

/// A spawned snowball tracks as a thrown item, not a dropped item: its instance carries the
/// sprite decoration at its own interpolated position (thrown projectiles are not hanging
/// and not client-simulated like `minecraft:item`), and a synched stack at slot 8 feeds the
/// sprite instead of the drop path.
#[test]
#[ignore = "needs cache/datagen (registries.json; HASTE_CACHE overrides)"]
fn store_builds_the_thrown_item_instance() {
    let mut s = super::store();
    let mut sp = super::spawn(1, DVec3::new(1.0, 64.0, 1.0));
    sp.kind = kind_of("minecraft:snowball");
    s.spawn(&sp);
    s.set_metadata(EntityId(1), &[stack(8, Some(777))]);
    let mut out: Vec<EntityInstance> = Vec::new();
    s.render_instances(0.5, &mut out);
    assert_eq!(out.len(), 1);
    match &out[0].decoration {
        Some(Decoration::ThrownItem(ThrownItemDecoration { item, .. })) => {
            assert_eq!(*item, RegistryIndex(777))
        }
        other => panic!("expected a thrown item, got {other:?}"),
    }
    // Not a dropped item (no spin/bob, no client-side item movement).
    assert!(out[0].item.is_none());
    assert!(!s.get(EntityId(1)).unwrap().is_item);
    // The position is the entity's own, not a hanging centre.
    assert_eq!(out[0].pos, DVec3::new(1.0, 64.0, 1.0));
}
