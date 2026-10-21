//! The evoker fangs' client state (`EvokerFangs.tick`, `handleEntityEvent` status 4 and
//! `getAnimationProgress`), with vanilla's numbers: `lifeTicks` starts at 22, the bite
//! reads `lifeTicks - 2` and lasts `ATTACK_DURATION` (20) ticks.

use super::*;
use haste_core::ids::EntityId;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

#[test]
fn fresh_fangs_start_at_life_22_with_no_bite() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(e.fangs.life_ticks, 22);
    assert!(!e.fangs.attack_started);
    // `getAnimationProgress` is 0 before the bite, whatever the partial tick.
    assert_eq!(e.fangs.animation_progress(0.0), 0.0);
    assert_eq!(e.fangs.animation_progress(0.7), 0.0);
}

#[test]
fn life_ticks_only_count_down_once_the_attack_starts() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.tick();
    s.tick();
    assert_eq!(s.get(EntityId(1)).unwrap().fangs.life_ticks, 22);
    s.mark_fangs_attack(EntityId(1));
    s.tick();
    assert_eq!(s.get(EntityId(1)).unwrap().fangs.life_ticks, 21);
}

#[test]
fn the_fangs_attack_does_not_start_the_golem_swing() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.mark_fangs_attack(EntityId(1));
    assert_eq!(s.get(EntityId(1)).unwrap().attack_ticks, 0);
}

#[test]
fn progress_rises_linearly_over_the_20_tick_bite() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.mark_fangs_attack(EntityId(1));
    // lifeTicks 22: remaining 20, so progress = 1 - (20 - a) / 20 = a / 20.
    let e = s.get(EntityId(1)).unwrap();
    assert!(close(e.fangs.animation_progress(0.0), 0.0));
    assert!(close(e.fangs.animation_progress(0.5), 0.025));
    // Nineteen ticks later lifeTicks is 3: remaining 1, progress at partial 0 is 0.95.
    for _ in 0..19 {
        s.tick();
    }
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(e.fangs.life_ticks, 3);
    assert!(close(e.fangs.animation_progress(0.0), 0.95));
}

#[test]
fn progress_is_one_once_the_bite_is_over() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.mark_fangs_attack(EntityId(1));
    // lifeTicks 2 is remaining 0: the bite has finished, whatever the partial tick.
    for _ in 0..20 {
        s.tick();
    }
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(e.fangs.life_ticks, 2);
    assert_eq!(e.fangs.animation_progress(0.0), 1.0);
    assert_eq!(e.fangs.animation_progress(0.9), 1.0);
    // And it stays at 1 as lifeTicks runs on below 2 until the server discards the fangs.
    s.tick();
    assert_eq!(
        s.get(EntityId(1)).unwrap().fangs.animation_progress(0.5),
        1.0
    );
}

#[test]
fn the_crit_burst_falls_on_the_tick_life_reaches_14() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::new(2.0, 64.0, 3.0)));
    s.mark_fangs_attack(EntityId(1));
    let mut burst_ticks = Vec::new();
    for tick in 1..=22 {
        s.tick();
        if s.fangs_crit_bursts().next().is_some() {
            burst_ticks.push(tick);
        }
    }
    // 22 - 8 = 14: the eighth tick after the bite starts, and only that one.
    assert_eq!(burst_ticks, vec![8]);
}

#[test]
fn the_crit_burst_spreads_twelve_crits_over_the_box_a_block_up() {
    use crate::evoker_fangs::{crit_burst, CRIT_COUNT};
    let pos = DVec3::new(10.0, 64.0, -5.0);
    // Every draw at 1.0 puts each crit at the box's +x/+z corner, at the top of its height
    // range and at full speed; every draw at 0.0 at the opposite extremes.
    let mut high = Vec::new();
    crit_burst(pos, 0.5, || 1.0, |c| high.push(c));
    assert_eq!(high.len(), CRIT_COUNT);
    let c = high[0];
    assert!((c.pos - DVec3::new(10.25, 64.0 + 0.05 + 1.0 + 1.0, -4.75)).length() < 1e-9);
    assert!((c.vel - DVec3::new(0.3, 0.6, 0.3)).length() < 1e-9);
    let mut low = Vec::new();
    crit_burst(pos, 0.5, || 0.0, |c| low.push(c));
    let c = low[0];
    assert!((c.pos - DVec3::new(9.75, 64.0 + 0.05 + 1.0, -5.25)).length() < 1e-9);
    assert!((c.vel - DVec3::new(-0.3, 0.3, -0.3)).length() < 1e-9);
}
