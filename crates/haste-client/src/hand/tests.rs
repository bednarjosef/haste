use super::*;

fn stone() -> Option<RegistryIndex> {
    Some(RegistryIndex(1))
}

/// One vanilla tick: no item held (the bare arm), standing on the ground, no swing, no
/// hands busy.
fn tick(h: &mut Hand, moved: f32) {
    h.tick(
        moved,
        (0.0, 0.0),
        true,
        false,
        HeldStack::of(None),
        4.0,
        false,
    );
}

/// Ticks until the equip progress is back up (vanilla raises the hand in over the first
/// ticks after joining, since `FirstPersonHandsAndItems` starts at 0).
fn settle(h: &mut Hand, item: Option<RegistryIndex>) {
    for _ in 0..12 {
        h.tick(
            0.0,
            (0.0, 0.0),
            true,
            false,
            HeldStack::of(item),
            4.0,
            false,
        );
    }
    assert_eq!(h.equip().1, 1.0);
}

#[test]
fn swing_progress_runs_over_six_ticks() {
    // `LivingEntity.SwingState` with `SwingAnimation.DEFAULT` (duration 6): the swing starts
    // on the tick of the attack with progress 0, and `tick` then advances ticks/duration, so
    // the visible frames are the six ticks after it.
    let mut h = Hand::new();
    h.swing();
    let mut seen = Vec::new();
    for _ in 0..9 {
        tick(&mut h, 0.0);
        seen.push(h.attack());
        let (start, end) = *seen.last().unwrap();
        assert_eq!(start, h.swing.prev_animation, "start is the previous tick");
        // The end is this tick, wrapped past 1 when it fell (`getAnimation`).
        assert_eq!(
            end,
            if h.swing.animation < h.swing.prev_animation {
                h.swing.animation + 1.0
            } else {
                h.swing.animation
            },
            "end is this tick"
        );
    }
    // progress = ticks / 6 at the end of each tick; the tick after the last clears it, and
    // the pair wraps the falling end past 1 (`SwingState.getAnimation`'s `animation + 1.0`)
    // so that tick lerps 1 -> 1, the rest pose, and nothing jumps.
    for (got, want) in seen.iter().zip([
        (0.0, 0.0),
        (0.0, 1.0 / 6.0),
        (1.0 / 6.0, 2.0 / 6.0),
        (2.0 / 6.0, 3.0 / 6.0),
        (3.0 / 6.0, 4.0 / 6.0),
        (4.0 / 6.0, 5.0 / 6.0),
        (5.0 / 6.0, 1.0),
        (1.0, 1.0),
        (0.0, 0.0),
    ]) {
        assert!(
            (got.0 - want.0).abs() < 1e-7 && (got.1 - want.1).abs() < 1e-7,
            "{got:?} != {want:?}"
        );
    }
    // Mid-swing the renderer lerps the pair: half way between the 0 and 1/6 ticks.
    let mut h = Hand::new();
    h.swing();
    tick(&mut h, 0.0);
    tick(&mut h, 0.0);
    let (start, end) = h.attack();
    assert!(start.abs() < 1e-7);
    assert!((end - 1.0 / 6.0).abs() < 1e-7);
    assert!((h.animation(0.5) - 1.0 / 12.0).abs() < 1e-7);
}

#[test]
fn the_interpolated_swing_never_jumps_back_up_after_it_ends() {
    // Every tick of one swing plus the tick after it ends, at the partials a frame can
    // sample. The swing rises over ticks 2..7, the tick it ends (8) holds the rest pose 1
    // (`SwingState.getAnimation` wraps the falling end to `animation + 1.0`, so the pair is
    // (1, 1) instead of the raw (1, 0)), and the tick after rests at 0. The raw pair would
    // lerp 1 -> 0 over the end tick and put the hand back at mid-swing (0.5) for a frame.
    let mut h = Hand::new();
    h.swing();
    let mut frames = Vec::new();
    for t in 1..=9 {
        tick(&mut h, 0.0);
        for p in [0.0, 0.5, 0.99] {
            frames.push((t, p, h.animation(p)));
        }
    }
    let at = |t: i32, p: f32| {
        frames
            .iter()
            .find(|(ft, fp, _)| *ft == t && *fp == p)
            .unwrap()
            .2
    };
    // Rising phase: within each tick, and from one tick's end into the next's start, the
    // interpolated value never decreases (tick 2 rises 0 -> 1/6, ..., tick 7 5/6 -> 1).
    for t in 2..=7 {
        assert!(
            at(t, 0.0) <= at(t, 0.5) + 1e-7 && at(t, 0.5) <= at(t, 0.99) + 1e-7,
            "tick {t} falls within itself"
        );
        assert!(
            at(t, 0.99) <= at(t + 1, 0.0) + 1e-7,
            "tick {t} ends above tick {} starts",
            t + 1
        );
    }
    // The tick the swing ends holds the rest pose 1 at every partial: the wrap.
    for p in [0.0, 0.5, 0.99] {
        let got = at(8, p);
        assert!((got - 1.0).abs() < 1e-6, "end tick partial {p}: {got} != 1");
    }
    // After the swing has ended nothing jumps back up towards mid-swing: the tick after
    // rests at 0 at every partial.
    for p in [0.0, 0.5, 0.99] {
        let got = at(9, p);
        assert!(got.abs() < 1e-6, "tick after partial {p}: {got} != 0");
    }
    // And the pair the renderer draws carries the same wrap.
    let mut h = Hand::new();
    h.swing();
    for _ in 0..8 {
        tick(&mut h, 0.0);
    }
    assert_eq!(h.attack(), (1.0, 1.0));
}

#[test]
fn a_swing_less_than_half_way_is_not_restarted() {
    let mut h = Hand::new();
    h.swing();
    for _ in 0..3 {
        tick(&mut h, 0.0);
    }
    // Half way (ticks 3 of 6): `startIfAble` refuses the restart, the swing carries on.
    h.swing();
    assert_eq!(h.attack().1, 2.0 / 6.0);
    tick(&mut h, 0.0);
    assert_eq!(h.attack().1, 3.0 / 6.0);
    // Past the half way a new swing starts over. The same hand and duration keep the
    // progress (`SwingState.start` resets only on a changed description), so the restart
    // tick's pair falls from 3/6 to 0 and `getAnimation`'s wrap carries it past 1.
    h.swing();
    tick(&mut h, 0.0);
    assert_eq!(h.attack(), (3.0 / 6.0, 1.0));
}

#[test]
fn no_swing_means_no_progress() {
    let mut h = Hand::new();
    for _ in 0..10 {
        tick(&mut h, 0.0);
    }
    assert_eq!(h.attack(), (0.0, 0.0));
    assert!(!h.is_swinging());
    // The bare arm (no item) is drawn.
    assert_eq!(h.shown_item(), None);
}

#[test]
fn equip_dips_on_an_item_change_and_comes_back() {
    // `FirstPersonHandsAndItems.tick`: with the same item held the progress falls 0.4 per
    // tick while the shown item differs from the held one, the new item appears once it is
    // below 0.1, and it then rises towards `getItemSwapScale(1.0)^3` =
    // ((itemSwapTicker + 1) / (20 / attack_speed))^3 with attack_speed 4 (a 5 tick delay).
    let mut h = Hand::new();
    settle(&mut h, None);
    let mut seen = Vec::new();
    for _ in 0..6 {
        h.tick(
            0.0,
            (0.0, 0.0),
            true,
            false,
            HeldStack::of(stone()),
            4.0,
            false,
        );
        seen.push((h.equip(), h.shown_item()));
    }
    // 1.0 -> 0.6 -> 0.2 -> 0.0 (the item switches) -> 0.4 -> 0.8.
    for (got, want) in seen
        .iter()
        .map(|(p, _)| p.1)
        .zip([0.6, 0.2, 0.0, 0.4, 0.8, 1.0])
    {
        assert!((got - want).abs() < 1e-6, "{got} != {want}");
    }
    // The bare arm stays until the progress drops below 0.1.
    assert_eq!(seen[0].1, None);
    assert_eq!(seen[1].1, None);
    assert_eq!(seen[2].1, stone());
    // A slow attack speed brings the item back slowly: with attack_speed 1 the swap delay
    // is 20 ticks, so the first tick after the switch only reaches ((3 + 1) / 20)^3 = 0.008.
    let mut slow = Hand::new();
    settle(&mut slow, None);
    for _ in 0..4 {
        slow.tick(
            0.0,
            (0.0, 0.0),
            true,
            false,
            HeldStack::of(stone()),
            1.0,
            false,
        );
    }
    assert!(
        (slow.equip().1 - 0.2f32.powi(3)).abs() < 1e-6,
        "{}",
        slow.equip().1
    );
    assert_eq!(slow.shown_item(), stone());
}

#[test]
fn a_count_or_component_change_dips_too() {
    // `shouldInstantlyReplaceVisibleItem` compares the whole stack minus the ignored
    // components (`ItemStack.matchesIgnoringComponents`): more of the same item in the
    // slot is a different stack and plays the swap, while the tickers (type-based,
    // `Player.tick`) do not reset.
    let mut h = Hand::new();
    settle(&mut h, stone());
    h.reset_attack_strength_ticker();
    let more = HeldStack {
        item: stone(),
        swap_key: 7,
    };
    h.tick(0.0, (0.0, 0.0), true, false, more, 4.0, false);
    assert_eq!(h.equip().1, 0.6, "the hand dips 0.4 on the count change");
    assert_eq!(h.shown_item(), stone(), "the old stack is still shown");
    assert_eq!(
        h.equip.attack_strength_ticker, 1,
        "no type change: no ticker reset"
    );
    for _ in 0..12 {
        h.tick(0.0, (0.0, 0.0), true, false, more, 4.0, false);
    }
    assert_eq!(
        h.equip().1,
        1.0,
        "then comes back up with the same type shown"
    );
    assert_eq!(h.shown_item(), stone());
}

#[test]
fn the_same_item_never_dips() {
    let mut h = Hand::new();
    settle(&mut h, stone());
    for _ in 0..5 {
        h.tick(
            0.0,
            (0.0, 0.0),
            true,
            false,
            HeldStack::of(stone()),
            4.0,
            false,
        );
        assert_eq!(h.equip().1, 1.0, "the held item stays put");
    }
}

#[test]
fn walk_bob_follows_the_walk_cycle() {
    // `ClientAvatarState` (walkDist = 0.6 * horizontal distance) and
    // `AbstractClientPlayer.updateBob` (the bob moves 0.4 per tick towards
    // min(0.1, the *previous* tick's horizontal speed)). Walking 0.1 blocks per tick.
    let mut h = Hand::new();
    let mut seen = Vec::new();
    for _ in 0..10 {
        tick(&mut h, 0.1);
        seen.push((h.walk_dist(), h.bob()));
    }
    // walkDist after n ticks of 0.1 blocks: 0.06 * n.
    for (i, (w, _)) in seen.iter().enumerate() {
        let n = (i + 1) as f32;
        assert!((w.1 - 0.06 * n).abs() < 1e-6, "walkDist {}", w.1);
        assert!(
            (w.0 - 0.06 * (n - 1.0)).abs() < 1e-6,
            "prev walkDist {}",
            w.0
        );
    }
    // The bob converges to the 0.1 cap by 0.4 per tick, one tick behind the walk.
    for (got, want) in seen.iter().map(|(_, b)| b.1).zip([
        0.0, 0.04, 0.064, 0.0784, 0.08704, 0.092224, 0.0953344, 0.09720064, 0.09832038, 0.09899223,
    ]) {
        assert!((got - want).abs() < 1e-6, "{got} != {want}");
    }
    // Leaving the ground: the first airborne tick still reads the ground tick's speed, the
    // one after it multiplies the bob by 0.6.
    h.tick(
        0.0,
        (0.0, 0.0),
        false,
        false,
        HeldStack::of(None),
        4.0,
        false,
    );
    let in_air = h.bob().1;
    assert!((in_air - (0.09899223 + (0.1 - 0.09899223) * 0.4)).abs() < 1e-6);
    h.tick(
        0.0,
        (0.0, 0.0),
        false,
        false,
        HeldStack::of(None),
        4.0,
        false,
    );
    assert!((h.bob().1 - in_air * 0.6).abs() < 1e-6, "{}", h.bob().1);
}

#[test]
fn the_bob_phase_is_the_walked_distance() {
    let mut h = Hand::new();
    for _ in 0..5 {
        tick(&mut h, 0.25);
    }
    // Backwards interpolated: -(walkDist + (walkDist - prevWalkDist) * partial).
    let (prev, end) = h.walk_dist();
    assert!((end - 0.75).abs() < 1e-6, "{end}");
    for partial in [0.0, 0.25, 0.5, 1.0] {
        let want = -(prev + (end - prev) * partial);
        assert!((h.bob.backwards_walk_dist(partial) - want).abs() < 1e-6);
    }
}

#[test]
fn the_hand_lags_behind_the_look() {
    // `LocalPlayer.applyInput`: xBob/yBob move half of the remaining angle towards the
    // player's new rotation each tick (yaw 10, 20, ... 80 over eight ticks).
    let mut h = Hand::new();
    let mut yaw = 0.0;
    let (mut want_yaw, mut want_pitch) = (0.0, 0.0);
    for _ in 0..8 {
        yaw += 10.0;
        h.tick(
            0.0,
            (yaw, yaw / 2.0),
            true,
            false,
            HeldStack::of(None),
            4.0,
            false,
        );
        want_yaw += (yaw - want_yaw) * 0.5;
        want_pitch += (yaw / 2.0 - want_pitch) * 0.5;
    }
    // yBob ends at 70.0390625 and xBob at 35.01953125 (exact binary fractions).
    assert_eq!(h.y_bob().1, 70.039_06, "{}", h.y_bob().1);
    assert!(
        (h.y_bob().1 - want_yaw).abs() < 1e-4,
        "{} != {want_yaw}",
        h.y_bob().1
    );
    assert_eq!(h.x_bob().1, 35.019_53, "{}", h.x_bob().1);
    assert!((h.x_bob().1 - want_pitch).abs() < 1e-4);
    // The pair is the previous tick's value and this one's.
    assert!((h.y_bob().0 - 60.078_125).abs() < 1e-4, "{}", h.y_bob().0);
}

#[test]
fn attack_strength_charges_over_the_item_delay_and_resets() {
    // `Player.attackStrengthTicker` advances each tick and `getAttackStrengthScale` is
    // (ticker + a) / (20 / attack_speed), clamped. The bare hand's attack speed 4 gives a
    // 5 tick delay; a diamond sword's 1.6 gives 12.5.
    let mut h = Hand::new();
    let mut scale = Vec::new();
    for _ in 0..6 {
        tick(&mut h, 0.0);
        scale.push(h.attack_strength_scale(0.0));
    }
    // None -> None on the first tick (no item type change), so the ticker runs 1..6.
    for (got, want) in scale.iter().zip([0.2, 0.4, 0.6, 0.8, 1.0, 1.0]) {
        assert!((got - want).abs() < 1e-6, "{got} != {want}");
    }
    // `Player.onAttack` resets only the attack ticker.
    h.reset_only_attack_strength_ticker();
    tick(&mut h, 0.0);
    assert!((h.attack_strength_scale(0.0) - 0.2).abs() < 1e-6);
    // An item type change resets it (`Player.tick`).
    h.tick(
        0.0,
        (0.0, 0.0),
        true,
        false,
        HeldStack::of(stone()),
        1.6,
        false,
    );
    assert_eq!(h.attack_strength_scale(0.0), 0.0);
    // The sword's slower attack speed: 12 ticks -> 12 / 12.5 = 0.96.
    for _ in 0..12 {
        h.tick(
            0.0,
            (0.0, 0.0),
            true,
            false,
            HeldStack::of(stone()),
            1.6,
            false,
        );
    }
    assert!((h.attack_strength_scale(0.0) - 0.96).abs() < 1e-6);
}

#[test]
fn attack_strength_charge_mirrors_the_ticker_and_delay() {
    // What the HUD mirror carries: the raw ticker and delay, from which Java's
    // `getAttackStrengthScale` computes (ticker + partial) / delay.
    let mut h = Hand::new();
    tick(&mut h, 0.0);
    assert_eq!(h.attack_strength_charge(), (1.0, 5.0));
    h.reset_only_attack_strength_ticker();
    assert_eq!(h.attack_strength_charge(), (0.0, 5.0));
    // A slower item changes the delay only (`Player.getCurrentItemAttackStrengthDelay`).
    h.tick(
        0.0,
        (0.0, 0.0),
        true,
        false,
        HeldStack::of(stone()),
        1.6,
        false,
    );
    assert_eq!(h.attack_strength_charge(), (0.0, 12.5));
    for _ in 0..3 {
        h.tick(
            0.0,
            (0.0, 0.0),
            true,
            false,
            HeldStack::of(stone()),
            1.6,
            false,
        );
    }
    assert_eq!(h.attack_strength_charge(), (3.0, 12.5));
}

#[test]
fn a_used_item_puts_the_hand_down() {
    // `FirstPersonHandsAndItems.itemUsed` sets the progress to 0 at once, so the tick that
    // follows it starts at 0 and rises back.
    let mut h = Hand::new();
    settle(&mut h, None);
    h.item_used();
    tick(&mut h, 0.0);
    assert_eq!(h.equip(), (0.0, 0.4));
    // With a real item held the shown item does not change (the type is the same).
    let mut h = Hand::new();
    settle(&mut h, stone());
    h.item_used();
    h.tick(
        0.0,
        (0.0, 0.0),
        true,
        false,
        HeldStack::of(stone()),
        4.0,
        false,
    );
    assert_eq!(h.equip(), (0.0, 0.4));
    assert_eq!(h.shown_item(), stone());
}

#[test]
fn steering_a_boat_lowers_the_hand_and_the_keys_released_raise_it() {
    // vanilla `LocalPlayer.rideTick`'s `handsBusy` (the boat we steer, a movement key held)
    // and `FirstPersonHandsAndItems.tick`'s busy branch: while busy the height falls by the
    // 0.4 step each tick (clamped at 0) instead of chasing the equip target, and once the
    // keys are released it rises back the same way.
    let mut h = Hand::new();
    settle(&mut h, stone());
    for _ in 0..3 {
        h.tick(
            0.0,
            (0.0, 0.0),
            true,
            false,
            HeldStack::of(stone()),
            4.0,
            true,
        );
    }
    // 1.0 -> 0.6 -> ~0.2 -> 0.0: three busy ticks of rowing.
    assert!((h.equip().1 - 0.0).abs() < 1e-6, "{}", h.equip().1);
    // The swap threshold still applies while busy: the same item stays shown.
    assert_eq!(h.shown_item(), stone());
    // A fourth busy tick stays at 0 (the clamp), not below.
    h.tick(
        0.0,
        (0.0, 0.0),
        true,
        false,
        HeldStack::of(stone()),
        4.0,
        true,
    );
    assert_eq!(h.equip().1, 0.0);
    // The keys released: the hand chases the equip target up again, 0.4 per tick.
    for want in [0.4, 0.8, 1.0] {
        h.tick(
            0.0,
            (0.0, 0.0),
            true,
            false,
            HeldStack::of(stone()),
            4.0,
            false,
        );
        assert!(
            (h.equip().1 - want).abs() < 1e-6,
            "{} != {want}",
            h.equip().1
        );
    }
    // The bare arm dips and rises the same way.
    let mut bare = Hand::new();
    settle(&mut bare, None);
    bare.tick(0.0, (0.0, 0.0), true, false, HeldStack::of(None), 4.0, true);
    assert!((bare.equip().1 - 0.6).abs() < 1e-6, "{}", bare.equip().1);
    bare.tick(
        0.0,
        (0.0, 0.0),
        true,
        false,
        HeldStack::of(None),
        4.0,
        false,
    );
    assert_eq!(
        bare.equip().1,
        1.0,
        "released: straight back up from 0.6 to the 1.0 target"
    );
}
