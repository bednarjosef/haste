//! The per-tick particle cost of firework bursts (the brief's measurement): 50 rockets' sparks,
//! trails and flashes ticked and snapshotted, against an empty pool. This is CPU work only (the
//! particle tick and the snapshot build), not the GPU draw. Release only.

use std::sync::Arc;
use std::time::{Duration, Instant};

use haste_core::view::BlockView;
use haste_core::BlockPos;

use super::*;
use crate::particles::{FireworkExplosion, FireworkShape};

/// A world of open air under full sky light, enough for the snapshot's light sampling.
struct Open;
impl BlockView for Open {
    fn block_state(&self, _p: BlockPos) -> haste_core::BlockStateId {
        haste_core::BlockStateId::AIR
    }
    fn is_chunk_loaded(&self, _p: haste_core::ChunkPos) -> bool {
        true
    }
    fn sky_light(&self, _p: BlockPos) -> u8 {
        15
    }
    fn block_light(&self, _p: BlockPos) -> u8 {
        0
    }
    fn min_y(&self) -> i32 {
        0
    }
    fn height(&self) -> i32 {
        256
    }
}

fn firework_types() -> Arc<ParticleTypes> {
    types_for_tests(&[
        ("firework", &[[0, 0, 4, 4]]),
        ("flash", &[[0, 0, 4, 4]]),
        ("poof", &[[0, 0, 4, 4]]),
    ])
}

/// A two-colour star with trails and twinkle: the heaviest common shape per spark.
fn star_show() -> Vec<FireworkExplosion> {
    vec![FireworkExplosion {
        shape: FireworkShape::Star,
        colors: vec![0xFF_00_00, 0xFF_FF_00],
        fade_colors: vec![0x00_00_FF],
        trail: true,
        twinkle: true,
    }]
}

/// Runs `TICKS` ticks, and when `bursts` is set fires 50 rockets' shows every `EVERY` ticks
/// (the bursts are queued outside the timed span). Returns the mean and worst time of one tick
/// plus its snapshot, and the peak number of live particles.
fn measure(bursts: bool) -> (Duration, Duration, usize) {
    const TICKS: u32 = 400;
    const EVERY: u32 = 40;
    let mut particles = Particles::new(firework_types());
    let show = star_show();
    let camera = DVec3::new(0.0, 64.0, 0.0);
    let mut views = Vec::new();
    let mut total = Duration::ZERO;
    let mut worst = Duration::ZERO;
    let mut peak = 0;
    for t in 0..TICKS {
        if bursts && t % EVERY == 0 {
            for i in 0..50 {
                let pos = DVec3::new(
                    -12.0 + f64::from(i % 10) * 2.6,
                    70.0,
                    f64::from(i / 10) * 2.6 - 5.0,
                );
                particles.firework_burst(pos, DVec3::new(0.0, 0.1, 0.0), &show);
            }
        }
        let start = Instant::now();
        particles.tick(camera);
        particles.build_snapshot(&Open, &mut views);
        let d = start.elapsed();
        worst = worst.max(d);
        total += d;
        peak = peak.max(particles.alive());
    }
    (total / TICKS, worst, peak)
}

#[test]
#[ignore = "timed test, run in release: cargo test --release -p haste-render firework_perf -- --ignored --nocapture"]
fn fifty_firework_bursts_tick_time() {
    let (quiet_avg, quiet_worst, _) = measure(false);
    let (avg, worst, peak) = measure(true);
    eprintln!("empty pool: average {quiet_avg:?}, worst {quiet_worst:?} per tick");
    eprintln!("50 rockets: average {avg:?}, worst {worst:?} per tick, peak {peak} particles");
    assert!(peak > 0, "the bursts put particles in the pool");
}
