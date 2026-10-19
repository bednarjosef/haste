//! Tests for the bench summary and camera path.

use super::*;

fn sample(frame_ms: f32) -> BenchSample {
    BenchSample {
        frame_ms,
        sections_drawn: 900,
        sections_total: 4000,
        draw_calls: 200,
        timings: haste_render::FrameTimings {
            prepare_ms: 1.0,
            cull_ms: 2.0,
            encode_ms: 3.0,
            submit_ms: 4.0,
            ..Default::default()
        },
        feed_ms: 0.5,
        sections_new: 0,
        uploads: 0,
        upload_bytes: 0,
        indices_drawn: 0,
        index: 0,
        visible_unmeshed: 0,
    }
}

#[test]
fn percentile_interpolates_and_clamps() {
    let v = [1.0, 2.0, 3.0, 4.0];
    assert_eq!(percentile(&v, 0.0), 1.0);
    assert_eq!(percentile(&v, 1.0), 4.0);
    // Halfway between 3 and 4: index (4-1)*0.75 = 2.25.
    assert!((percentile(&v, 0.75) - 3.25).abs() < 1e-6);
    assert!(percentile(&v, 2.0) <= 4.0); // out-of-range p clamps
    assert_eq!(percentile(&[], 0.5), 0.0);
}

#[test]
fn bench_summary_averages_and_percentile() {
    // 200 samples: 199 at 10 ms, one at 100 ms -> the 1% low (p99) is still a 10 ms frame,
    // the outlier sits above the 99th percentile.
    let samples: Vec<BenchSample> = (0..200)
        .map(|i| sample(if i == 0 { 100.0 } else { 10.0 }))
        .collect();
    let s = bench_summary(&samples, (0, 0, 0));
    assert_eq!(s.frames, 200);
    // f32: (199*10 + 100)/200 accumulates to 10.45.
    assert!((s.avg_ms - 10.45).abs() < 1e-2, "{}", s.avg_ms);
    assert!((s.p99_ms - 10.0).abs() < 1e-2, "{}", s.p99_ms);
    assert_eq!(s.worst_ms, 100.0);
    assert!((s.fps() - 1000.0 / 10.45).abs() < 1e-2, "{}", s.fps());
    assert!((s.prepare_ms - 1.0).abs() < 1e-6);
    assert!((s.cull_ms - 2.0).abs() < 1e-6);
    assert!((s.encode_ms - 3.0).abs() < 1e-6);
    assert!((s.submit_ms - 4.0).abs() < 1e-6);
    assert!((s.sections_drawn - 900.0).abs() < 1e-6);
    assert!((s.draw_calls - 200.0).abs() < 1e-6);
}

#[test]
fn slowest_indices_slowest_first_with_ties_by_frame() {
    let mut slow = sample(50.0);
    slow.index = 10;
    let mut samples: Vec<BenchSample> = (0..49).map(|_| sample(5.0)).collect();
    samples.push(slow);
    let order = slowest_indices(&samples, 20);
    assert_eq!(order.len(), 20);
    assert_eq!(order[0], 49, "the 50 ms frame leads");
    // All 5 ms frames tie; the earliest frames come first among them.
    assert_eq!(order[1], 0);
    assert_eq!(order[2], 1);
    // Asking for more than there are returns all, and an empty run returns none.
    assert_eq!(slowest_indices(&samples, 100).len(), 50);
    assert!(slowest_indices(&[], 20).is_empty());
    print_slowest_frames(&samples);
}

#[test]
fn bench_camera_path_turns_then_flies() {
    let base = Camera {
        pos: DVec3::new(100.0, 64.0, -200.0),
        yaw: 30.0,
        pitch: 10.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    };
    // t = 0: at the base, unturned.
    let c = bench_camera(0.0, &base, 60.0);
    assert_eq!(c.pos, base.pos);
    assert_eq!(c.yaw, 30.0);
    // t = half: a full 360 turned; flight begins here (y = eye + 20, no distance yet).
    let c = bench_camera(0.5, &base, 60.0);
    assert!((c.pos.y - 84.0).abs() < 1e-4, "{}", c.pos.y);
    assert!((c.pos.x - 100.0).abs() < 1e-4, "{}", c.pos.x);
    assert!((c.yaw - 390.0).abs() < 1e-4, "{}", c.yaw);
    // t = 1: 30 s of flight at 10 b/s along +X, 20 blocks up.
    let c = bench_camera(1.0, &base, 60.0);
    assert!((c.pos.x - 400.0).abs() < 1e-4, "{}", c.pos.x);
    assert!((c.pos.y - 84.0).abs() < 1e-4, "{}", c.pos.y);
    assert!((c.yaw - 390.0).abs() < 1e-4);
    assert_eq!(c.pos.z, base.pos.z);
    // Quarter point: 180 degrees around, no flight yet.
    let c = bench_camera(0.25, &base, 60.0);
    assert!((c.yaw - 210.0).abs() < 1e-4, "{}", c.yaw);
    assert_eq!(c.pos, base.pos);
    // Just after half: climbing already done, flight starting.
    let c = bench_camera(0.51, &base, 60.0);
    assert!((c.pos.y - 84.0).abs() < 1e-4, "{}", c.pos.y);
    // Three-quarter point: 15 s flown = 150 blocks along +X.
    let c = bench_camera(0.75, &base, 60.0);
    assert!((c.pos.x - 250.0).abs() < 1e-4, "{}", c.pos.x);
    assert!((c.pos.y - 84.0).abs() < 1e-4, "{}", c.pos.y);
}

#[test]
fn turns_path_holds_turns_180_alternately_then_flies() {
    let base = Camera {
        pos: DVec3::new(9.5, 71.0, 6.5),
        yaw: 0.0,
        pitch: 10.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    };
    // 30 s run: 2 s hold, turning until 60% (18 s: four full 4 s cycles + 2 s into the
    // fifth), then flight for the last 40% (12 s).
    // Hold: at the base, unturned, unmoved.
    let c = bench_camera_turns(0.0, &base, 30.0);
    assert_eq!((c.pos, c.yaw), (base.pos, 0.0));
    let c = bench_camera_turns(1.99, &base, 30.0);
    assert_eq!((c.pos, c.yaw), (base.pos, 0.0));
    // First turn: 180 deg/s for 1 s, so half a second in is 90 degrees around.
    let c = bench_camera_turns(2.5, &base, 30.0);
    assert!((c.yaw - 90.0).abs() < 1e-4, "{}", c.yaw);
    assert_eq!(c.pos, base.pos, "no movement during a turn");
    // Held at +180 after the first turn.
    let c = bench_camera_turns(3.5, &base, 30.0);
    assert!((c.yaw - 180.0).abs() < 1e-4, "{}", c.yaw);
    // Second turn: back the other way, 90 degrees in is +90 again.
    let c = bench_camera_turns(6.5, &base, 30.0);
    assert!((c.yaw - 90.0).abs() < 1e-4, "{}", c.yaw);
    let c = bench_camera_turns(7.5, &base, 30.0);
    assert!((c.yaw - 0.0).abs() < 1e-4, "{}", c.yaw);
    // The turning phase ends at 18 s (t = 16 into it: four full cycles, the last one
    // ending back at the base look) — the look holds wherever the last cycle left it.
    let c = bench_camera_turns(17.9, &base, 30.0);
    assert!((c.yaw - 0.0).abs() < 1e-4, "{}", c.yaw);
    let c = bench_camera_turns(18.0, &base, 30.0);
    assert!((c.yaw - 0.0).abs() < 1e-4, "{}", c.yaw);
    // Flight leg: 10 blocks/s along +X, 20 blocks up. 3 s in is 30 blocks.
    let c = bench_camera_turns(21.0, &base, 30.0);
    assert!((c.pos.x - 39.5).abs() < 1e-4, "{}", c.pos.x);
    assert!((c.pos.y - 91.0).abs() < 1e-4, "{}", c.pos.y);
    assert!((c.yaw - 0.0).abs() < 1e-4, "{}", c.yaw);
    // End of the run: 12 s flown = 120 blocks.
    let c = bench_camera_turns(30.0, &base, 30.0);
    assert!((c.pos.x - 129.5).abs() < 1e-4, "{}", c.pos.x);
}

#[test]
fn popin_summary_counts_episodes_and_section_frames() {
    // A turn: 3 unmeshed frames, one meshed, 2 more unmeshed, then quiet.
    let unmeshed = [0u32, 4, 2, 1, 0, 7, 3, 0, 0];
    let samples: Vec<BenchSample> = unmeshed
        .iter()
        .enumerate()
        .map(|(i, u)| {
            let mut s = sample(5.0);
            s.index = i as u32;
            s.visible_unmeshed = *u;
            s
        })
        .collect();
    let p = popin_summary(&samples);
    let (episodes, avg, worst) = p.stats();
    assert_eq!(episodes, 2, "two turns with pop-in");
    assert!((avg - 2.5).abs() < 1e-6, "{avg}");
    assert_eq!(worst, 3);
    assert_eq!(p.frames_with_unmeshed, 5);
    assert_eq!(p.unmeshed_section_frames, 17);
    // A run that never pops: no episode, no frames.
    let clean: Vec<BenchSample> = (0..10).map(|_| sample(5.0)).collect();
    let p = popin_summary(&clean);
    assert_eq!(p.stats(), (0, 0.0, 0));
    assert_eq!(p.unmeshed_section_frames, 0);
}

#[test]
fn interval_stats_sorts_and_picks_percentiles() {
    // 999 frames of 1 ms and one of 50 ms: the average moves, p50/p99 stay 1 ms.
    let mut v = vec![1.0_f32; 999];
    v.insert(500, 50.0);
    let [avg, p50, p90, p99, _p999, worst] = window::interval_stats(&v);
    assert!((avg - 1.049).abs() < 1e-4, "{avg}");
    assert_eq!((p50, p90, p99, worst), (1.0, 1.0, 1.0, 50.0));
    assert_eq!(window::interval_stats(&[]), [0.0; 6]);
}
