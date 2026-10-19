// Tests for the work budget: `fixed` is exactly the historical 2 ms / 8 ms pair, and
// `adaptive` predicts from the recent frames and clamps to the floor and cap.

use super::*;

/// Feeds `n` frames of observations into `budget` at the given interval.
fn feed(
    budget: &mut WorkBudget,
    interval_ms: f32,
    render_ms: f32,
    feed_ms: f32,
    gpu_ms: f32,
    n: u32,
) {
    for _ in 0..n {
        budget.note_frame(interval_ms, render_ms, feed_ms, gpu_ms);
    }
}

#[test]
fn fixed_is_the_historical_pair() {
    let mut b = WorkBudget::new();
    b.set_mode(WorkBudgetMode::Fixed);
    assert_eq!(b.budget_ms(0), FIXED_MS);
    assert_eq!(b.budget_ms(BACKLOG_THRESHOLD), FIXED_MS);
    assert_eq!(b.budget_ms(BACKLOG_THRESHOLD + 1), BACKLOG_MS);
    assert_eq!(b.budget_ms(100_000), BACKLOG_MS);
}

#[test]
fn adaptive_never_leaves_the_floor_and_cap() {
    let mut b = WorkBudget::new();
    b.set_mode(WorkBudgetMode::Adaptive);
    // A frame predicted far over any target: the floor keeps the backlog draining.
    feed(&mut b, 1.0, 50.0, 50.0, 0.0, 200);
    assert_eq!(b.budget_ms(5000), FLOOR_MS);
    // A frame with everything idle: the cap keeps one frame from stalling.
    feed(&mut b, 100.0, 0.1, 0.1, 0.0, 200);
    assert_eq!(b.budget_ms(5000), CAP_MS);
}

#[test]
fn adaptive_gives_the_time_left_to_the_target() {
    let mut b = WorkBudget::new();
    b.set_mode(WorkBudgetMode::Adaptive);
    b.set_target(Some(Duration::from_millis(16)));
    // A GPU-bound 8 ms frame with 11 ms of CPU work: the uploads fill the target minus
    // the CPU cost (the GPU time is already the frame's floor), mid-range so the floor
    // and cap do not clamp it.
    feed(&mut b, 8.0, 9.0, 2.0, 8.0, 200);
    let budget = b.budget_ms(0);
    assert!((budget - 5.0).abs() < 1.0, "budget {budget}");
    // A heavier frame predicts less room: the budget shrinks.
    feed(&mut b, 8.0, 11.0, 2.0, 8.0, 200);
    let heavier = b.budget_ms(0);
    assert!(heavier < budget, "heavier {heavier} vs {budget}");
}

#[test]
fn adaptive_uses_the_rolling_interval_without_a_target() {
    let mut b = WorkBudget::new();
    b.set_mode(WorkBudgetMode::Adaptive);
    // Unthrottled at ~5 ms frames with 3 ms of CPU cost: the rolling interval is the
    // target, so the uploads get the slack but never make frames worse than the recent
    // average.
    feed(&mut b, 5.0, 2.0, 1.0, 0.0, 200);
    let budget = b.budget_ms(0);
    assert!((budget - 2.0).abs() < 1.0, "budget {budget}");
    // A slower machine (10 ms frames of the same CPU work) gets more room.
    feed(&mut b, 10.0, 2.0, 1.0, 0.0, 200);
    let slower = b.budget_ms(0);
    assert!(slower > budget, "slower {slower} vs {budget}");
}

#[test]
fn adaptive_lifts_the_target_to_the_gpu_time() {
    let mut b = WorkBudget::new();
    b.set_mode(WorkBudgetMode::Adaptive);
    // The GPU main pass is the frame's floor: uploads under it are free, so a 12 ms GPU
    // frame with 8 ms of CPU work budgets against max(rolling 12, GPU 12) = 12, not the
    // unthrottled interval the CPU alone would allow.
    feed(&mut b, 12.0, 6.0, 2.0, 12.0, 200);
    let budget = b.budget_ms(0);
    assert!((budget - 4.0).abs() < 1.0, "budget {budget}");
    // Without the GPU time the same CPU cost on a 12 ms frame would leave more room
    // only if the interval were the target — it is, so this is the control: a lower GPU
    // time must never shrink the budget.
    feed(&mut b, 12.0, 6.0, 2.0, 4.0, 200);
    let control = b.budget_ms(0);
    assert!(control >= budget - 0.5, "control {control} vs {budget}");
}

#[test]
fn a_spiky_frame_cost_leaves_less_room_than_a_smooth_one() {
    let mut b = WorkBudget::new();
    b.set_mode(WorkBudgetMode::Adaptive);
    b.set_target(Some(Duration::from_millis(16)));
    let mut c = WorkBudget::new();
    c.set_mode(WorkBudgetMode::Adaptive);
    c.set_target(Some(Duration::from_millis(16)));
    // Same average CPU cost, one smooth and one spiky: the variance term keeps the
    // spiky one's budget lower (both land inside the floor-cap window).
    for i in 0..400 {
        let spike = if i % 8 == 0 { 18.0 } else { 8.0 };
        b.note_frame(16.7, spike, 2.0, 5.0);
        c.note_frame(16.7, 9.0, 2.0, 5.0);
    }
    let smooth = c.budget_ms(0);
    let spiky = b.budget_ms(0);
    assert!(spiky < smooth, "spiky {spiky} vs smooth {smooth}");
}

#[test]
fn ewma_converges_and_deviates() {
    let mut e = Ewma::default();
    for _ in 0..1000 {
        e.update(10.0, 1.0 / 60.0);
    }
    assert!((e.mean - 10.0).abs() < 0.01);
    assert!(e.sigma() < 0.01);
    // An alternating series has the same mean but a nonzero deviation.
    for i in 0..1000 {
        e.update(if i % 2 == 0 { 12.0 } else { 8.0 }, 1.0 / 60.0);
    }
    assert!((e.mean - 10.0).abs() < 0.2);
    assert!((e.sigma() - 2.0).abs() < 0.5, "sigma {}", e.sigma());
}
