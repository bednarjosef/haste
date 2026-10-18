use super::*;
use std::collections::HashMap;

#[test]
fn empty_section_connects_all_faces() {
    let v = compute_visibility(|_, _, _| false);
    for a in Face::ALL {
        for b in Face::ALL {
            assert!(v.visible_between(a, b));
        }
    }
}

#[test]
fn solid_section_connects_nothing() {
    let v = compute_visibility(|_, _, _| true);
    for a in Face::ALL {
        for b in Face::ALL {
            assert!(!v.visible_between(a, b));
        }
    }
}

#[test]
fn wall_splits_west_from_east() {
    // Full YZ plane at x=8 (256 cells, so the flood path runs).
    let v = compute_visibility(|x, _, _| x == 8);
    assert!(!v.visible_between(Face::West, Face::East));
    assert!(v.visible_between(Face::West, Face::Up));
    assert!(v.visible_between(Face::East, Face::Up));
}

#[test]
fn open_grid_reaches_every_section() {
    // No visibility data => every section is see-through; the walk must
    // reach every section within render distance.
    let section_vis: HashMap<(ChunkPos, i32), VisibilitySet> = HashMap::new();
    let sc = 8;
    let rd = 6;
    let eye = DVec3::new(8.0, 72.0, 8.0);
    let mask = compute_visible_mask(
        |c, si| section_vis.get(&(c, si)).copied(),
        ChunkPos::new(0, 0),
        4,
        eye,
        0,
        sc,
        rd,
    );
    let full = (1u32 << sc) - 1;
    for x in -rd..=rd {
        for z in -rd..=rd {
            assert_eq!(
                mask.column(ChunkPos::new(x, z)),
                full,
                "column ({x}, {z}) not fully reached"
            );
        }
    }
}

#[test]
fn solid_wall_hides_sections_behind_it() {
    let mut section_vis = HashMap::new();
    section_vis.insert((ChunkPos::new(0, -1), 4), VisibilitySet::none());
    let eye = DVec3::new(8.0, 72.0, 8.0);
    let mask = compute_visible_mask(
        |c, si| section_vis.get(&(c, si)).copied(),
        ChunkPos::new(0, 0),
        4,
        eye,
        0,
        8,
        4,
    );
    // The wall section itself is reached (visible face)...
    assert!(mask.column(ChunkPos::new(0, -1)) & (1 << 4) != 0);
    // ...but the section directly behind it on the same row is not.
    assert!(mask.column(ChunkPos::new(0, -2)) & (1 << 4) == 0);
}

/// The walk before the culling optimizations: per-face ray march, the
/// 6-iteration `any` sightline check and `/16.0` section indexing. The
/// optimized walk must produce the identical mask; this reference is the
/// oracle the differential test compares against.
fn reference_visible_mask(
    section_vis: &HashMap<(ChunkPos, i32), VisibilitySet>,
    cam_col: ChunkPos,
    cam_si: i32,
    eye: DVec3,
    min_y: i32,
    section_count: i32,
    render_distance: i32,
) -> VisibleMask {
    let r = render_distance.max(0);
    let w = 2 * r + 1;
    let h = section_count.max(1);
    let n = (w * w * h) as usize;
    let mut source = vec![0u8; n];
    let mut cone = vec![0u8; n];
    let mut columns = vec![0u32; (w * w) as usize];
    let mut queue: VecDeque<(i32, i32, i32)> = VecDeque::new();
    let mut order: Vec<u32> = Vec::new();
    let idx = |x: i32, si: i32, z: i32| -> Option<usize> {
        let (dx, dz) = (x - cam_col.x + r, z - cam_col.z + r);
        if dx < 0 || dz < 0 || dx >= w || dz >= w || si < 0 || si >= h {
            return None;
        }
        Some(((dx * w + dz) * h + si) as usize)
    };
    let cam_center = DVec3::new(
        (cam_col.x * 16 + 8) as f64,
        (min_y + cam_si * 16 + 8) as f64,
        (cam_col.z * 16 + 8) as f64,
    );
    let start = (cam_col.x, cam_si.clamp(0, h - 1), cam_col.z);
    cone[idx(start.0, start.1, start.2).unwrap()] = VISITED;
    columns[(r * w + r) as usize] |= 1u32 << start.1;
    queue.push_back(start);
    while let Some((x, si, z)) = queue.pop_front() {
        let i = idx(x, si, z).unwrap();
        order.push(((x - cam_col.x + r) * w + (z - cam_col.z + r)) as u32 * h as u32 + si as u32);
        let src = source[i];
        let c = cone[i] & !VISITED;
        let vis = section_vis
            .get(&(ChunkPos::new(x, z), si))
            .copied()
            .unwrap_or(VisibilitySet::all());
        let distant = (x - cam_col.x).abs() > ADV_CULL_SECTION_DIST
            || (si - cam_si).abs() > ADV_CULL_SECTION_DIST
            || (z - cam_col.z).abs() > ADV_CULL_SECTION_DIST;
        for face in Face::ALL {
            if c & face.opposite().bit() != 0 {
                continue;
            }
            if src != 0
                && !Face::ALL
                    .into_iter()
                    .any(|sf| src & sf.bit() != 0 && vis.visible_between(sf.opposite(), face))
            {
                continue;
            }
            let (dx, dy, dz) = face.offset();
            let (nx, nsi, nz) = (x + dx, si + dy, z + dz);
            if distant
                && reference_ray_occluded(
                    &cone,
                    &idx,
                    eye,
                    cam_center,
                    (x, si, z),
                    min_y,
                    section_count,
                )
            {
                continue;
            }
            let Some(ni) = idx(nx, nsi, nz) else {
                continue;
            };
            if cone[ni] & VISITED != 0 {
                source[ni] |= face.bit();
                continue;
            }
            source[ni] = face.bit();
            cone[ni] = VISITED | c | face.bit();
            let col = ((nx - cam_col.x + r) * w + (nz - cam_col.z + r)) as usize;
            columns[col] |= 1u32 << nsi;
            queue.push_back((nx, nsi, nz));
        }
    }
    VisibleMask {
        cx: cam_col.x,
        cz: cam_col.z,
        r,
        h,
        columns,
        order,
    }
}

fn reference_ray_occluded(
    cone: &[u8],
    idx: &impl Fn(i32, i32, i32) -> Option<usize>,
    eye: DVec3,
    cam_center: DVec3,
    (sx, ssi, sz): (i32, i32, i32),
    min_y: i32,
    section_count: i32,
) -> bool {
    let ox = (sx * 16) as f64;
    let oy = (min_y + ssi * 16) as f64;
    let oz = (sz * 16) as f64;
    let corner = |c: f64, o: f64| o + if c > o { 16.0 } else { 0.0 };
    let mut check = DVec3::new(
        corner(cam_center.x, ox),
        corner(cam_center.y, oy),
        corner(cam_center.z, oz),
    );
    let step = (eye - check).normalize() * CEILED_SECTION_DIAGONAL;
    let world_min_y = min_y as f64;
    let world_max_y = (min_y + section_count * 16) as f64;
    while check.distance_squared(eye) > ADV_CULL_MIN_DIST_SQ {
        check += step;
        if check.y > world_max_y || check.y < world_min_y {
            return false;
        }
        let cx = (check.x / 16.0).floor() as i32;
        let csi = ((check.y - min_y as f64) / 16.0).floor() as i32;
        let cz = (check.z / 16.0).floor() as i32;
        let blocked = idx(cx, csi, cz).is_none_or(|i| cone[i] & VISITED == 0);
        if blocked {
            return true;
        }
    }
    false
}

/// Deterministic LCG so the differential test needs no dependencies.
fn lcg(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *state >> 33
}

#[test]
fn optimized_walk_matches_reference() {
    for seed in 1..=40u64 {
        let mut rng = seed;
        let sc = 4 + (lcg(&mut rng) % 5) as i32; // 4..=8 sections tall
        let rd = 2 + (lcg(&mut rng) % 4) as i32; // render distance 2..=5
        let w = 2 * rd + 1;
        let mut section_vis = HashMap::new();
        for x in -rd..=rd {
            for z in -rd..=rd {
                for si in 0..sc {
                    let roll = lcg(&mut rng) % 10;
                    let vis = match roll {
                        0 => VisibilitySet::none(),
                        1 => VisibilitySet::all(),
                        _ => {
                            // Symmetric like every set `add` builds.
                            let bits = lcg(&mut rng) & ((1u64 << 36) - 1);
                            let mut mirrored = 0u64;
                            for a in 0..6u32 {
                                for b in 0..6u32 {
                                    if bits >> (a * 6 + b) & 1 != 0 {
                                        mirrored |= 1u64 << (b * 6 + a);
                                    }
                                }
                            }
                            VisibilitySet(bits | mirrored)
                        }
                    };
                    section_vis.insert((ChunkPos::new(x, z), si), vis);
                }
            }
        }
        let cam_x = lcg(&mut rng) as i32 % w - rd;
        let cam_z = lcg(&mut rng) as i32 % w - rd;
        let cam_si = lcg(&mut rng) as i32 % sc;
        let eye = DVec3::new(
            (lcg(&mut rng) % 400) as f64 - 100.0,
            (lcg(&mut rng) % 200) as f64,
            (lcg(&mut rng) % 400) as f64 - 100.0,
        );
        let min_y = -8 + (lcg(&mut rng) % 16) as i32;
        let fast = compute_visible_mask(
            |c, si| section_vis.get(&(c, si)).copied(),
            ChunkPos::new(cam_x, cam_z),
            cam_si,
            eye,
            min_y,
            sc,
            rd,
        );
        let slow = reference_visible_mask(
            &section_vis,
            ChunkPos::new(cam_x, cam_z),
            cam_si,
            eye,
            min_y,
            sc,
            rd,
        );
        for x in -rd..=rd {
            for z in -rd..=rd {
                assert_eq!(
                    fast.column(ChunkPos::new(x, z)),
                    slow.column(ChunkPos::new(x, z)),
                    "seed {seed}: column ({x}, {z}) differs"
                );
            }
        }
    }
}

/// The worker's mask must equal the synchronous walk over the same data:
/// the async path changes only where the walk runs, not what it computes.
#[test]
fn async_walk_matches_sync() {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    let mut map = AHashMap::new();
    let mut state = 0x1234_5678_9abc_def0u64;
    let sc = 8i32;
    let vd = 6i32;
    for x in -vd..=vd {
        for z in -vd..=vd {
            for si in 0..sc {
                let v = match lcg(&mut state) % 4 {
                    0 => VisibilitySet::none(),
                    1 => VisibilitySet::all(),
                    _ => VisibilitySet(lcg(&mut state) | (lcg(&mut state) << 32)),
                };
                map.insert((ChunkPos::new(x, z), si), v);
            }
        }
    }
    let vis = Arc::new(Mutex::new(map));
    let walker = AsyncOcclusionWalker::new(vis.clone()).unwrap();
    let cam = ChunkPos::new(1, -1);
    let eye = DVec3::new(20.0, 70.0, -10.0);
    walker.schedule(SectionPos::new(cam.x, 4, cam.z), vd, sc, 0, eye);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mask = loop {
        if let Some(m) = walker.poll() {
            break m;
        }
        assert!(Instant::now() < deadline, "worker never finished the walk");
        std::thread::sleep(Duration::from_millis(1));
    };
    let sync = compute_visible_mask(
        |c, si| vis.lock().get(&(c, si)).copied(),
        cam,
        4,
        eye,
        0,
        sc,
        vd + 1,
    );
    assert_eq!(mask, sync);
}

/// The render thread must never run the walk: `schedule` (a channel send)
/// and `poll` (a channel drain) cost microseconds even while large walks are
/// in flight on the worker. A single call can be delayed arbitrarily on a
/// loaded machine (CI neighbours, preemption), so this measures many calls
/// against a worker kept busy with queued walks and asserts on the median —
/// if either call ran the ~ms walk itself, the median would be ~ms too.
#[test]
fn render_thread_never_walks() {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    let mut map = AHashMap::new();
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let sc = 24i32;
    let vd = 32i32;
    for x in -vd..=vd {
        for z in -vd..=vd {
            for si in 0..sc {
                let v = match lcg(&mut state) % 3 {
                    0 => VisibilitySet::none(),
                    1 => VisibilitySet::all(),
                    _ => VisibilitySet(lcg(&mut state) | (lcg(&mut state) << 32)),
                };
                map.insert((ChunkPos::new(x, z), si), v);
            }
        }
    }
    let vis = Arc::new(Mutex::new(map));
    let walker = AsyncOcclusionWalker::new(vis).unwrap();
    let eye = DVec3::new(8.0, 72.0, 8.0);

    // Queue enough full walks (each ~ms) that the worker stays busy for the
    // whole measurement; the walker joins the remaining queue on drop, so the
    // count stays bounded and the test finishes quickly.
    const WALKS: usize = 32;
    const SAMPLES: usize = 512;

    let mut schedule_us: Vec<f64> = Vec::with_capacity(WALKS);
    let mut poll_us: Vec<f64> = Vec::with_capacity(SAMPLES);
    let mut mask = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while poll_us.len() < SAMPLES && Instant::now() < deadline {
        if schedule_us.len() < WALKS {
            let t = Instant::now();
            walker.schedule(SectionPos::new(0, 4, 0), vd, sc, -4, eye);
            schedule_us.push(t.elapsed().as_secs_f64() * 1e6);
        }
        let t = Instant::now();
        if let Some(m) = walker.poll() {
            mask = Some(m);
        }
        poll_us.push(t.elapsed().as_secs_f64() * 1e6);
        std::thread::sleep(Duration::from_micros(200));
    }
    let median = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.total_cmp(b));
        v[v.len() / 2]
    };
    let med_schedule = median(&mut schedule_us);
    let med_poll = median(&mut poll_us);
    eprintln!(
        "schedule: median {med_schedule:.2} us ({} calls), \
         poll: median {med_poll:.2} us ({} calls)",
        schedule_us.len(),
        poll_us.len()
    );
    assert!(
        med_schedule < 500.0,
        "schedule blocked the render thread (median {med_schedule:.2} us)"
    );
    assert!(
        med_poll < 500.0,
        "poll blocked the render thread (median {med_poll:.2} us)"
    );
    let mask = mask.unwrap_or_else(|| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(m) = walker.poll() {
                break m;
            }
            assert!(Instant::now() < deadline, "worker never finished");
            std::thread::sleep(Duration::from_micros(200));
        }
    });
    assert!(
        mask.column(ChunkPos::new(0, 0)) != 0,
        "worker walked an empty mask"
    );
}
