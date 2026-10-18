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
    let s = bench_summary(&samples);
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
