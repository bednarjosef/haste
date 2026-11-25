use super::*;
use crate::testing::{states::*, TestShapes, TestWorld};
use haste_core::DVec3;

fn keys(forward: bool, sprint: bool, sneak: bool) -> InputState {
    InputState {
        forward,
        sprint,
        sneak,
        ..InputState::default()
    }
}

fn walker() -> PlayerState {
    let mut s = PlayerState::at(DVec3::new(0.5, 1.0, 0.5));
    s.on_ground = true;
    s
}

#[test]
fn shallow_water_ends_a_run_and_keeps_it_from_starting() {
    // Vanilla `isSprintingPossible(abilities.flying)`: wading (in water, eyes out) is not
    // allowed unless flying.
    let mut s = walker();
    s.in_water = true;
    s.sprinting = true;
    update_sprinting(&mut s, &keys(true, true, false), false);
    assert!(!s.sprinting, "wading into water ends the sprint");
    update_sprinting(&mut s, &keys(true, true, false), false);
    assert!(!s.sprinting, "and it cannot start while wading");
    update_sprinting(&mut s, &keys(true, true, false), true);
    assert!(s.sprinting, "with the eyes under it starts");
    let mut f = walker();
    f.in_water = true;
    f.flying = true;
    update_sprinting(&mut f, &keys(true, true, false), false);
    assert!(f.sprinting, "flying ignores shallow water");
}

#[test]
fn item_use_gliding_and_blindness_keep_a_sprint_from_starting() {
    // Vanilla `canStartSprinting`: not slowed by the used item, not fall flying unless under
    // water, and sprinting possible (not blind, fed).
    let mut s = walker();
    s.slow_using_item = true;
    update_sprinting(&mut s, &keys(true, true, false), false);
    assert!(!s.sprinting, "eating");
    let mut s = walker();
    s.fall_flying = true;
    update_sprinting(&mut s, &keys(true, true, false), false);
    assert!(!s.sprinting, "gliding");
    let mut s = walker();
    s.fall_flying = true;
    s.in_water = true;
    update_sprinting(&mut s, &keys(true, true, false), true);
    assert!(s.sprinting, "gliding under water");
    let mut s = walker();
    s.sprint_blocked = true;
    update_sprinting(&mut s, &keys(true, true, false), false);
    assert!(!s.sprinting, "blind");
    let mut s = walker();
    s.sprinting = true;
    s.sprint_blocked = true;
    update_sprinting(&mut s, &keys(true, false, false), false);
    assert!(!s.sprinting, "going blind ends a running sprint");
}

#[test]
fn swimming_into_a_wall_keeps_the_sprint() {
    // `shouldStopSwimSprinting` has no collision term, unlike `shouldStopRunSprinting`.
    let mut s = walker();
    s.on_ground = false;
    s.in_water = true;
    s.swimming = true;
    s.sprinting = true;
    s.horizontal_collision = true;
    update_sprinting(&mut s, &keys(true, false, false), true);
    assert!(s.sprinting);
    s.swimming = false;
    update_sprinting(&mut s, &keys(true, false, false), true);
    assert!(!s.sprinting, "running into the wall ends a run");
}

#[test]
fn a_swim_stops_without_forward_only_off_the_ground_and_without_shift() {
    let swimmer = || {
        let mut s = walker();
        s.on_ground = false;
        s.in_water = true;
        s.swimming = true;
        s.sprinting = true;
        s
    };
    let mut s = swimmer();
    update_sprinting(&mut s, &keys(false, false, false), true);
    assert!(!s.sprinting, "letting go of forward mid-water");
    let mut s = swimmer();
    update_sprinting(&mut s, &keys(false, false, true), true);
    assert!(s.sprinting, "shift held keeps it");
    let mut s = swimmer();
    s.on_ground = true;
    update_sprinting(&mut s, &keys(false, false, false), true);
    assert!(s.sprinting, "on the ground keeps it");
    let mut s = swimmer();
    s.in_water = false;
    update_sprinting(&mut s, &keys(true, false, false), false);
    assert!(!s.sprinting, "out of the water");
}

#[test]
fn the_swim_starts_from_last_ticks_sprint_with_water_at_the_feet() {
    // `Entity.updateSwimming` runs in `baseTick`, before the sprint rule: a fresh sprint
    // swims a tick later, and only with water in the feet block and the eyes under.
    let mut w = TestWorld::flat(0, STONE);
    w.fill((-2, 1, -2), (2, 3, 2), WATER_SOURCE);
    let shapes = TestShapes::default();
    let mut s = walker();
    s.in_water = true;
    update_swimming(&mut s, true, &w, &shapes);
    assert!(!s.swimming, "not sprinting yet");
    s.sprinting = true;
    update_swimming(&mut s, false, &w, &shapes);
    assert!(!s.swimming, "eyes out of the water");
    update_swimming(&mut s, true, &w, &shapes);
    assert!(s.swimming);
    s.flying = true;
    update_swimming(&mut s, true, &w, &shapes);
    assert!(!s.swimming, "flying never swims");
}
