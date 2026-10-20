//! In-process A/B measurement (`HASTE_AB_BLOCK=N`): the bench flips
//! `haste_render::experiment` every N frames and reports each variant's frame and GPU
//! main-pass times, block against neighbouring block, so a GPU clock change between runs
//! cannot pass for a difference between the variants.

use super::BenchSample;

/// Frames dropped at the start of each block: the frames still in flight belong to the
/// previous variant, and the GPU timings arrive a frame or two late.
const SETTLE: u32 = 4;

/// The block length from `HASTE_AB_BLOCK`, if set (at least `2 * SETTLE` frames).
pub(super) fn block() -> Option<u32> {
    let n: u32 = std::env::var("HASTE_AB_BLOCK").ok()?.parse().ok()?;
    Some(n.max(2 * SETTLE))
}

/// Whether frame `index` runs the variant under test.
pub(super) fn variant(index: u32, block: u32) -> bool {
    (index / block) % 2 == 1
}

/// Average frame and GPU main-pass ms per variant, and the median of the per-pair ratios
/// (each on-block against the off-block before it).
pub(super) fn report(samples: &[BenchSample], block: u32) -> String {
    // Per block: (sum frame, sum gpu, frames).
    let mut blocks: Vec<(f64, f64, u32)> = Vec::new();
    for s in samples {
        let b = (s.index / block) as usize;
        if s.index % block < SETTLE {
            continue;
        }
        if blocks.len() <= b {
            blocks.resize(b + 1, (0.0, 0.0, 0));
        }
        blocks[b].0 += s.frame_ms as f64;
        blocks[b].1 += s.timings.gpu_ms[0] as f64;
        blocks[b].2 += 1;
    }
    let avg = |b: &(f64, f64, u32)| (b.0 / b.2.max(1) as f64, b.1 / b.2.max(1) as f64);
    let mut totals = [(0.0, 0.0, 0u32); 2];
    let mut ratios = Vec::new();
    for (i, b) in blocks.iter().enumerate() {
        if b.2 == 0 {
            continue;
        }
        let t = &mut totals[i % 2];
        t.0 += b.0;
        t.1 += b.1;
        t.2 += b.2;
        if i % 2 == 1 && blocks[i - 1].2 > 0 {
            let (on, off) = (avg(b), avg(&blocks[i - 1]));
            ratios.push((on.0 / off.0, on.1 / off.1));
        }
    }
    let median = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or(f64::NAN)
    };
    let (off, on) = (avg(&totals[0]), avg(&totals[1]));
    format!(
        "A/B (block {block}): off frame {:.3} ms gpu main {:.3} ms ({} frames) | on frame {:.3} ms \
         gpu main {:.3} ms ({} frames) | median on/off ratio frame {:.3} gpu {:.3} over {} pairs",
        off.0,
        off.1,
        totals[0].2,
        on.0,
        on.1,
        totals[1].2,
        median(ratios.iter().map(|r| r.0).collect()),
        median(ratios.iter().map(|r| r.1).collect()),
        ratios.len()
    )
}
