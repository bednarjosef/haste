//! `ItemActivation` countdown and activation (vanilla `ItemActivation.tick`, `reset`, `activate`).

use super::*;
use haste_core::RegistryIndex;

fn stack(item: u32) -> ItemStackData {
    ItemStackData {
        count: 1,
        item: Some(RegistryIndex(item)),
        ..Default::default()
    }
}

#[test]
fn activate_shows_a_copy_for_forty_ticks() {
    let mut a = ItemActivation::default();
    a.activate(&stack(5), &mut LegacyRandom::new(1));
    assert!(a.is_active());
    assert_eq!(a.ticks(), 40);
    assert_eq!(a.item(), Some(&stack(5)));
}

#[test]
fn countdown_clears_the_item_on_the_fortieth_tick() {
    let mut a = ItemActivation::default();
    a.activate(&stack(5), &mut LegacyRandom::new(1));
    for _ in 0..39 {
        a.tick();
    }
    assert_eq!(a.ticks(), 1);
    assert!(a.is_active());
    assert!(a.item().is_some());
    a.tick();
    assert_eq!(a.ticks(), 0);
    assert!(!a.is_active());
    assert!(a.item().is_none());
}

#[test]
fn tick_does_nothing_when_idle() {
    let mut a = ItemActivation::default();
    a.tick();
    assert_eq!(a.ticks(), 0);
    assert!(!a.is_active());
}

#[test]
fn reset_drops_the_item_and_the_countdown() {
    let mut a = ItemActivation::default();
    a.activate(&stack(5), &mut LegacyRandom::new(1));
    a.tick();
    a.reset();
    assert!(!a.is_active());
    assert_eq!(a.ticks(), 0);
    assert!(a.item().is_none());
}

#[test]
fn offsets_are_in_minus_one_to_one_and_follow_the_random_stream() {
    let mut a = ItemActivation::default();
    let mut r = LegacyRandom::new(99);
    a.activate(&stack(5), &mut r);
    let mut expect = LegacyRandom::new(99);
    let x = expect.next_float() * 2.0 - 1.0;
    let y = expect.next_float() * 2.0 - 1.0;
    assert_eq!(a.off_x(), x);
    assert_eq!(a.off_y(), y);
    assert!((-1.0..1.0).contains(&a.off_x()));
    assert!((-1.0..1.0).contains(&a.off_y()));
}

#[test]
fn the_renderer_sees_the_item_only_while_the_countdown_runs() {
    let mut a = ItemActivation::default();
    assert_eq!(a.render_state(), None);
    a.activate(&stack(5), &mut LegacyRandom::new(3));
    a.tick();
    let s = a.render_state().expect("active");
    assert_eq!((s.item, s.ticks), (RegistryIndex(5), 39));
    assert_eq!((s.off_x, s.off_y), (a.off_x(), a.off_y()));
    for _ in 0..39 {
        a.tick();
    }
    assert_eq!(a.render_state(), None);
}
