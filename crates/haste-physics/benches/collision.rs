//! Collision hot path benchmark (std timing, no extra dependencies).
//! Run: `cargo bench -p haste-physics` (release profile).
use std::hint::black_box;
use std::time::Instant;

use haste_core::input::InputState;
use haste_core::view::BlockView;
use haste_core::{Aabb, BlockPos, BlockStateId, ChunkPos, DVec3};
use haste_physics::testing::{states::*, TestShapes};
use haste_physics::{collide, collide_with_step, tick, PlayerAttributes, PlayerState};

/// Dense 64^3 world (array lookups, like a section snapshot) so the sweep itself is measured.
struct Dense {
    blocks: Vec<BlockStateId>,
}
const N: i32 = 64;
impl Dense {
    fn new() -> Self {
        let mut d = Self { blocks: vec![BlockStateId::AIR; (N * N * N) as usize] };
        for x in 0..N {
            for z in 0..N {
                d.set(x, 9, z, STONE);
                if (x + z) % 7 == 0 {
                    d.set(x, 10, z, BOTTOM_SLAB);
                }
                if x % 16 == 0 {
                    d.set(x, 10, z, STONE);
                    d.set(x, 11, z, STONE);
                }
            }
        }
        d
    }
    fn set(&mut self, x: i32, y: i32, z: i32, s: BlockStateId) {
        self.blocks[((y * N + z) * N + x) as usize] = s;
    }
}
impl BlockView for Dense {
    fn block_state(&self, p: BlockPos) -> BlockStateId {
        if p.x < 0 || p.y < 0 || p.z < 0 || p.x >= N || p.y >= N || p.z >= N {
            return BlockStateId::AIR;
        }
        self.blocks[((p.y * N + p.z) * N + p.x) as usize]
    }
    fn is_chunk_loaded(&self, _: ChunkPos) -> bool {
        true
    }
    fn sky_light(&self, _: BlockPos) -> u8 {
        15
    }
    fn block_light(&self, _: BlockPos) -> u8 {
        0
    }
    fn min_y(&self) -> i32 {
        0
    }
    fn height(&self) -> i32 {
        N
    }
}

/// Best of 7 batches (the machine is shared; the minimum is the least noisy estimate).
fn bench(name: &str, iters: u32, mut f: impl FnMut(u32)) {
    for i in 0..iters / 10 {
        f(i);
    }
    let mut best = f64::MAX;
    for _ in 0..7 {
        let t = Instant::now();
        for i in 0..iters {
            f(i);
        }
        best = best.min(t.elapsed().as_nanos() as f64 / iters as f64);
    }
    println!("{name:<40} {best:>9.1} ns/iter (best of 7 x {iters})");
}

fn main() {
    let w = Dense::new();
    let s = TestShapes::default();
    let player = |i: u32| Aabb::entity(DVec3::new(20.5 + (i % 13) as f64 * 0.37, 10.0, 20.5 + (i % 7) as f64 * 0.41), 0.6, 1.8);
    bench("collide: walk on ground (gravity)", 200_000, |i| {
        black_box(collide(black_box(player(i)), DVec3::new(0.2, -0.0784, 0.1), &w, &s));
    });
    bench("collide: fall at terminal velocity", 200_000, |i| {
        let b = player(i).offset(DVec3::new(0.0, 20.0, 0.0));
        black_box(collide(black_box(b), DVec3::new(0.05, -3.92, 0.02), &w, &s));
    });
    bench("collide_with_step: walk on ground", 200_000, |i| {
        black_box(collide_with_step(black_box(player(i)), DVec3::new(0.28, -0.0784, 0.1), 0.6, true, &w, &s));
    });
    bench("collide_with_step: into a wall", 200_000, |_| {
        let b = Aabb::entity(DVec3::new(31.7, 10.0, 20.5), 0.6, 1.8);
        black_box(collide_with_step(black_box(b), DVec3::new(0.28, -0.0784, 0.1), 0.6, true, &w, &s));
    });
    let attrs = PlayerAttributes::default();
    let input = InputState { forward: true, sprint: true, jump: true, yaw: 30.0, ..Default::default() };
    let mut st = PlayerState::at(DVec3::new(20.5, 10.0, 20.5));
    bench("tick: sprint-jumping over terrain", 200_000, |i| {
        if i % 100 == 0 {
            st = PlayerState::at(DVec3::new(20.5, 10.0, 20.5));
        }
        black_box(tick(&mut st, &input, &attrs, &w, &s, &s));
    });
}
