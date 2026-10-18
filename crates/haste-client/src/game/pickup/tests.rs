//! The pickup particle's render instance: an item draws as the item, an orb as the orb, both
//! at the eased position.

use glam::DVec3;
use haste_core::model::Decoration;
use haste_core::{EntityId, RegistryIndex};

use super::*;

fn particle(visual: PickupVisual) -> PickupParticle {
    PickupParticle {
        id: EntityId(5),
        visual,
        age: 4.0,
        start: DVec3::ZERO,
        target_old: DVec3::splat(2.0),
        target: DVec3::splat(2.0),
        collector: EntityId(7),
        life: 1,
    }
}

#[test]
fn item_particle_draws_the_item_at_the_eased_position() {
    let p = particle(PickupVisual::Item {
        item: RegistryIndex(3),
        count: 2,
    });
    let inst = instance(&p, 0.0);
    let item = inst.item.expect("an item instance");
    assert_eq!(item.item, RegistryIndex(3));
    assert_eq!(item.count, 2);
    assert_eq!(item.age, 4.0);
    assert!(inst.decoration.is_none());
    // life 1, partial 0: t = (1/3)^2 = 1/9, pos = 2/9.
    assert!((inst.pos - DVec3::splat(2.0 / 9.0)).length() < 1e-6);
}

#[test]
fn orb_particle_draws_the_orb() {
    let p = particle(PickupVisual::Orb { value: 5 });
    let inst = instance(&p, 0.0);
    assert!(inst.item.is_none());
    match inst.decoration {
        Some(Decoration::ExperienceOrb(o)) => assert_eq!(o.value, 5),
        other => panic!("expected an orb decoration, got {other:?}"),
    }
}
