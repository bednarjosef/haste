//! The client-side ambient block particles, vanilla `ClientLevel.animateTick`
//! (`Minecraft.tick` runs it right before the particle engine's tick): 667 random
//! blocks around the player per tick, each sampled at radius 16 and 32
//! (`xt + nextInt(r) - nextInt(r)`, the difference of two uniforms), and every
//! sampled block's `animateTick` runs plus the fluid drip rule. The per-block
//! behaviours are the ones vanilla implements in the block classes; blocks
//! without one (the vast majority, and every mod block we do not know) spawn
//! nothing, like vanilla's empty `Block.animateTick`.
//!
//! Ported here (vanilla class in brackets):
//! - torch, soul_torch [TorchBlock]: smoke + flame at (0.5, 0.7, 0.5) of the block;
//! - wall_torch, soul_wall_torch [WallTorchBlock]: the same 0.27 out of the wall
//!   the torch faces away from, 0.22 up;
//! - fire, soul_fire [BaseFireBlock]: 3 large smoke in the upper half when the
//!   block below is sturdy (the flammable-neighbour side smoke needs the ignite
//!   odds table, which haste does not load; treated as not flammable);
//! - campfire [CampfireBlock.animateTick + CampfireBlockEntity.particleTick]: the
//!   lava sparks (1 in 5) plus the cosy/signal smoke puffs (11 %, 2-3 of them).
//!   soul_campfire passes `spawnParticles=false` and gets none;
//! - lava [LavaFluid.animateTick]: a lava particle on the surface, 1 in 100, when
//!   the block above is air; the pop and ambient sounds are not ours to play here;
//! - nether_portal [NetherPortalBlock]: 4 portal particles, pushed off the plane;
//! - leaves [LeavesBlock]: rain-gated dripping water (1 in 15) under the block —
//!   dormant until weather is decoded (`raining` is always false today);
//! - any fluid state [ClientLevel.doAnimateTick's fluid branch]: the drip rule,
//!   1 in 10 — vanilla `trySpawnDripParticles` (the `IMPERMEABLE` tag check is not
//!   modelled: fluid over glass drips here where vanilla's does not).
//!
//! The sounds vanilla plays in some of these (fire crackle, lava pop, campfire
//! crackle, portal hum) are skipped: haste plays sounds from packets and its own
//! player rules only so far.

use haste_core::view::{BlockView, CollisionShapes};
use haste_core::{BlockPos, BlockStateId, Identifier};
use haste_data::GameData;
use haste_render::ParticleSpawn;

/// Blocks sampled per tick (vanilla `ClientLevel.animateTick`).
const SAMPLES: u32 = 667;
/// The two sampling radii of each sample.
const RADII: [i32; 2] = [16, 32];

/// xorshift64* — deterministic, so the odds are testable.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform f64 in [0, 1) (`RandomSource.nextDouble`'s 53-bit form is not needed:
    /// vanilla's `nextDouble` is 53-bit, ours 24 — positions differ in the last bits).
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 40) as f64 / 16_777_216.0
    }
    /// Vanilla `RandomSource.nextInt(bound)`.
    fn next_int(&mut self, bound: i32) -> i32 {
        ((self.next_f64() * bound as f64) as i32).min(bound - 1)
    }
    /// Vanilla `RandomSource.nextFloat`.
    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / 16_777_216.0
    }
}

/// One `animateTick` step: samples the blocks and queues `addParticle` spawns
/// (`ParticleSpawn` count-0 semantics; the pool applies vanilla's 32-block limiter
/// against the camera). `raining` gates the leaf drip (weather is not decoded yet).
/// The buffer is replaced, not appended: only this tick's samples are in it when the
/// call returns, so a block that is gone (a broken torch) stops spawning at once.
pub fn animate_tick(
    world: &dyn BlockView,
    data: &GameData,
    player: BlockPos,
    raining: bool,
    out: &mut Vec<ParticleSpawn>,
) {
    out.clear();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..SAMPLES {
        for r in RADII {
            // `xt + nextInt(r) - nextInt(r)`: triangular around the player.
            let pos = BlockPos::new(
                player.x + rng.next_int(r) - rng.next_int(r),
                player.y + rng.next_int(r) - rng.next_int(r),
                player.z + rng.next_int(r) - rng.next_int(r),
            );
            if !world.is_chunk_loaded(pos.chunk()) {
                continue;
            }
            let state = world.block_state(pos);
            if state == BlockStateId::AIR {
                continue;
            }
            block_animate_tick(world, data, pos, state, raining, &mut rng, out);
        }
    }
}

/// `state.getBlock().animateTick(state, level, pos, animateRandom)`: dispatch by the
/// block's registry id (vanilla by class; unknown blocks do nothing).
fn block_animate_tick(
    world: &dyn BlockView,
    data: &GameData,
    pos: BlockPos,
    state: BlockStateId,
    raining: bool,
    rng: &mut Rng,
    out: &mut Vec<ParticleSpawn>,
) {
    let block = data.blocks.block_of(state);
    let (ns, path) = (block.namespace(), block.path());
    if ns != "minecraft" {
        return;
    }
    let add = |out: &mut Vec<ParticleSpawn>,
               kind: &str,
               x: f64,
               y: f64,
               z: f64,
               vx: f32,
               vy: f32,
               vz: f32| {
        out.push(ParticleSpawn {
            kind: Identifier::minecraft(kind),
            pos: glam::DVec3::new(x, y, z),
            offset: [vx, vy, vz],
            // count 0: one particle at the position, velocity `offset * max_speed`.
            max_speed: [1.0, 1.0, 1.0],
            count: 0,
            randomization: 0,
            override_limiter: false,
            dust: None,
            block_state: None,
        });
    };
    match path {
        // TorchBlock: smoke + the torch's flame, 0.7 up in the middle.
        "torch" | "soul_torch" => {
            let (x, y, z) = (pos.x as f64 + 0.5, pos.y as f64 + 0.7, pos.z as f64 + 0.5);
            add(out, "smoke", x, y, z, 0.0, 0.0, 0.0);
            add(
                out,
                if path == "torch" {
                    "flame"
                } else {
                    "soul_fire_flame"
                },
                x,
                y,
                z,
                0.0,
                0.0,
                0.0,
            );
        }
        // WallTorchBlock: 0.27 out of the wall opposite the facing, 0.22 up.
        "wall_torch" | "soul_wall_torch" => {
            let facing = facing_of(data, state).unwrap_or(haste_core::Direction::North);
            let away = facing.opposite().offset().to_array();
            let (ox, oz) = (away[0], away[2]);
            let (x, y, z) = (
                pos.x as f64 + 0.5 + 0.27 * ox as f64,
                pos.y as f64 + 0.7 + 0.22,
                pos.z as f64 + 0.5 + 0.27 * oz as f64,
            );
            add(out, "smoke", x, y, z, 0.0, 0.0, 0.0);
            add(
                out,
                if path == "wall_torch" {
                    "flame"
                } else {
                    "soul_fire_flame"
                },
                x,
                y,
                z,
                0.0,
                0.0,
                0.0,
            );
        }
        // BaseFireBlock: 3 large smoke in the upper half when the block below is
        // sturdy (the flammable-neighbour smoke is not modelled).
        "fire" | "soul_fire" => {
            let below = BlockPos::new(pos.x, pos.y - 1, pos.z);
            if is_sturdy_top(world, data, below) {
                for _ in 0..3 {
                    add(
                        out,
                        "large_smoke",
                        pos.x as f64 + rng.next_f64(),
                        pos.y as f64 + rng.next_f64() * 0.5 + 0.5,
                        pos.z as f64 + rng.next_f64(),
                        0.0,
                        0.0,
                        0.0,
                    );
                }
            }
        }
        // CampfireBlock: the sparks; CampfireBlockEntity.particleTick's smoke.
        "campfire" => {
            let lit = data
                .blocks
                .property_value(state, "lit")
                .is_some_and(|v| v == "true");
            if lit {
                if rng.next_int(5) == 0 {
                    add(
                        out,
                        "lava",
                        pos.x as f64 + 0.5,
                        pos.y as f64 + 0.5,
                        pos.z as f64 + 0.5,
                        rng.next_f32() / 2.0,
                        5.0e-5,
                        rng.next_f32() / 2.0,
                    );
                }
                if rng.next_f64() < 0.11 {
                    let signal = data
                        .blocks
                        .property_value(state, "signal_fire")
                        .is_some_and(|v| v == "true");
                    let kind = if signal {
                        "campfire_signal_smoke"
                    } else {
                        "campfire_cosy_smoke"
                    };
                    let puffs = rng.next_int(2) + 2;
                    for _ in 0..puffs {
                        add(
                            out,
                            kind,
                            pos.x as f64
                                + rng.next_f64() / 3.0
                                    * if rng.next_f64() < 0.5 { 1.0 } else { -1.0 },
                            pos.y as f64 + rng.next_f64() + rng.next_f64(),
                            pos.z as f64
                                + rng.next_f64() / 3.0
                                    * if rng.next_f64() < 0.5 { 1.0 } else { -1.0 },
                            0.0,
                            0.07,
                            0.0,
                        );
                    }
                }
            }
        }
        // NetherPortalBlock: 4 particles pushed off the portal plane.
        "nether_portal" => {
            let (west, east) = (
                world.block_state(BlockPos::new(pos.x - 1, pos.y, pos.z)),
                world.block_state(BlockPos::new(pos.x + 1, pos.y, pos.z)),
            );
            for _ in 0..4 {
                let mut x = pos.x as f64 + rng.next_f64();
                let y = pos.y as f64 + rng.next_f64();
                let mut z = pos.z as f64 + rng.next_f64();
                let mut xa = (rng.next_f32() - 0.5) * 0.5;
                let ya = (rng.next_f32() - 0.5) * 0.5;
                let mut za = (rng.next_f32() - 0.5) * 0.5;
                let flip = if rng.next_int(2) == 0 { -1.0 } else { 1.0 };
                if west != state && east != state {
                    x = pos.x as f64 + 0.5 + 0.25 * flip;
                    xa = rng.next_f32() * 2.0 * flip as f32;
                } else {
                    z = pos.z as f64 + 0.5 + 0.25 * flip;
                    za = rng.next_f32() * 2.0 * flip as f32;
                }
                add(out, "portal", x, y, z, xa, ya, za);
            }
        }
        _ => {}
    }
    // LeavesBlock.makeDrippingWaterParticles: 1 in 15 under rain, when the block
    // below does not block the drip (weather is not decoded: the `raining` flag
    // stays false and this never fires yet).
    if is_leaves(path)
        && raining
        && rng.next_int(15) == 1
        && !is_sturdy_top(world, data, BlockPos::new(pos.x, pos.y - 1, pos.z))
    {
        add(
            out,
            "dripping_water",
            pos.x as f64 + rng.next_f64(),
            pos.y as f64 - 0.05,
            pos.z as f64 + rng.next_f64(),
            0.0,
            0.0,
            0.0,
        );
    }
    // The fluid branch of `doAnimateTick`: any non-empty fluid state drips.
    if let Some(fluid) = haste_physics::BlockPhysics::fluid(data, state) {
        if rng.next_int(10) == 0 {
            let kind = match fluid.kind {
                haste_core::view::FluidKind::Water => "dripping_water",
                haste_core::view::FluidKind::Lava => "dripping_lava",
            };
            drip_below(world, data, pos, kind, rng, out);
        }
        // LavaFluid.animateTick: a lava particle on the surface, 1 in 100, when the
        // block above is air (the pop/ambient sounds stay vanilla's server-side or
        // are not ours).
        if fluid.kind == haste_core::view::FluidKind::Lava {
            let above = world.block_state(BlockPos::new(pos.x, pos.y + 1, pos.z));
            if above == BlockStateId::AIR && rng.next_int(100) == 0 {
                add(
                    out,
                    "lava",
                    pos.x as f64 + rng.next_f64(),
                    pos.y as f64 + 1.0,
                    pos.z as f64 + rng.next_f64(),
                    0.0,
                    0.0,
                    0.0,
                );
            }
        }
    }
}

/// Vanilla `trySpawnDripParticles`: the fluid at `pos` drips below the block under
/// it when that block's shape lets a particle through — the common cases are a
/// short block under the fluid (drip at its top) and a full block with room below
/// (drip at its bottom, over the block's own column).
fn drip_below(
    world: &dyn BlockView,
    data: &GameData,
    pos: BlockPos,
    kind: &str,
    rng: &mut Rng,
    out: &mut Vec<ParticleSpawn>,
) {
    let below = BlockPos::new(pos.x, pos.y - 1, pos.z);
    let below_state = world.block_state(below);
    if haste_physics::BlockPhysics::fluid(data, below_state).is_some() {
        return;
    }
    let boxes = data.shapes.collision_boxes(below_state);
    let (top, bottom) = bounds_y(boxes);
    let mut add = |out: &mut Vec<ParticleSpawn>, y: f64, x0: f64, x1: f64, z0: f64, z1: f64| {
        let t = rng.next_f64();
        let u = rng.next_f64();
        out.push(ParticleSpawn {
            kind: Identifier::minecraft(kind),
            pos: glam::DVec3::new(x0 + t * (x1 - x0), y, z0 + u * (z1 - z0)),
            offset: [0.0, 0.0, 0.0],
            max_speed: [0.0, 0.0, 0.0],
            count: 0,
            randomization: 0,
            override_limiter: false,
            dust: None,
            block_state: None,
        });
    };
    if top < 1.0 {
        // A short block under the fluid: the drip is at the fluid's bottom.
        add(
            out,
            below.y as f64 + 1.0 - 0.05,
            below.x as f64,
            below.x as f64 + 1.0,
            below.z as f64,
            below.z as f64 + 1.0,
        );
    } else if bottom > 0.0 {
        // A block held up off the floor: the drip is at its underside.
        add(
            out,
            below.y as f64 + bottom - 0.05,
            below.x as f64,
            below.x as f64 + 1.0,
            below.z as f64,
            below.z as f64 + 1.0,
        );
    } else {
        // A full block: the drip is under it, when the block under that is short.
        let below2 = BlockPos::new(pos.x, pos.y - 2, pos.z);
        let below2_state = world.block_state(below2);
        if haste_physics::BlockPhysics::fluid(data, below2_state).is_none() {
            let (top2, _) = bounds_y(data.shapes.collision_boxes(below2_state));
            if top2 < 1.0 {
                add(
                    out,
                    below.y as f64 - 0.05,
                    below.x as f64,
                    below.x as f64 + 1.0,
                    below.z as f64,
                    below.z as f64 + 1.0,
                );
            }
        }
    }
}

/// The top and bottom of a block's collision boxes (empty: none).
fn bounds_y(boxes: &[haste_core::Aabb]) -> (f64, f64) {
    let mut top = f64::NEG_INFINITY;
    let mut bottom = f64::INFINITY;
    for b in boxes {
        top = top.max(b.max.y);
        bottom = bottom.min(b.min.y);
    }
    (top, bottom)
}

/// `below.isFaceSturdy(level, below, Direction.UP)` approximated with the collision
/// top reaching 1.0 (the blocks that matter here: full cubes are sturdy).
fn is_sturdy_top(world: &dyn BlockView, data: &GameData, below: BlockPos) -> bool {
    let state = world.block_state(below);
    bounds_y(data.shapes.collision_boxes(state)).0 >= 1.0
}

/// The `facing` property of a state, if it holds a direction name.
fn facing_of(data: &GameData, state: BlockStateId) -> Option<haste_core::Direction> {
    let v = data.blocks.property_value(state, "facing")?;
    match v {
        "north" => Some(haste_core::Direction::North),
        "south" => Some(haste_core::Direction::South),
        "west" => Some(haste_core::Direction::West),
        "east" => Some(haste_core::Direction::East),
        _ => None,
    }
}

/// The block ids vanilla's `LeavesBlock` hierarchy covers (blocks whose name ends in
/// `_leaves`; the class is what vanilla dispatches on and every leaves block
/// extends it).
fn is_leaves(path: &str) -> bool {
    path.ends_with("_leaves")
}

#[cfg(test)]
mod tests {
    use super::*;
    use haste_core::view::BlockView;

    /// A flat world of one block type repeated at y=64.
    struct Flat(BlockStateId);
    impl BlockView for Flat {
        fn block_state(&self, p: BlockPos) -> BlockStateId {
            if p.y == 64 {
                self.0
            } else {
                BlockStateId::AIR
            }
        }
        fn is_chunk_loaded(&self, _: haste_core::ChunkPos) -> bool {
            true
        }
        fn sky_light(&self, _: BlockPos) -> u8 {
            15
        }
        fn block_light(&self, _: BlockPos) -> u8 {
            0
        }
        fn min_y(&self) -> i32 {
            -64
        }
        fn height(&self) -> i32 {
            384
        }
    }

    /// Needs the datagen reports in the cache (block states).
    fn game_data() -> Option<GameData> {
        let data = GameData::load().ok()?;
        // The shapes fallback is fine; states must resolve names.
        (data.blocks.len() > 1000).then_some(data)
    }

    fn state_of(data: &GameData, name: &str) -> BlockStateId {
        data.blocks
            .default_state(&Identifier::minecraft(name))
            .unwrap()
    }

    /// Torch odds: with the player standing in a sea of torches, every animateTick
    /// step queues smoke+flame pairs — the fraction of samples that land on a
    /// torch is the triangular distribution's share, and each hit queues exactly
    /// one smoke and one flame at (0.5, 0.7, 0.5) within the block.
    #[test]
    #[ignore = "needs the datagen reports in the cache"]
    fn torch_animate_tick_spawns_smoke_and_flame() {
        let Some(data) = game_data() else {
            eprintln!("ignored: needs the data generator reports in the cache");
            return;
        };
        let torch = state_of(&data, "torch");
        let world = Flat(torch);
        let mut out = Vec::new();
        animate_tick(&world, &data, BlockPos::new(0, 64, 0), false, &mut out);
        let smoke = out
            .iter()
            .filter(|s| s.kind.path() == "smoke")
            .collect::<Vec<_>>();
        let flame = out
            .iter()
            .filter(|s| s.kind.path() == "flame")
            .collect::<Vec<_>>();
        // 667 samples * 2 radii around the player land on torches (y=64 plane):
        // every sampled position with y==64 is a torch.
        assert!(!smoke.is_empty(), "no smoke spawned");
        assert_eq!(smoke.len(), flame.len(), "smoke and flame pair up");
        for s in &smoke {
            // Every torch is at the same offsets in its block.
            let rel = s.pos - s.pos.floor();
            assert!((rel.x - 0.5).abs() < 1e-9, "x {}", rel.x);
            assert!((rel.y - 0.7).abs() < 1e-9, "y {}", rel.y);
            assert!((rel.z - 0.5).abs() < 1e-9, "z {}", rel.z);
            // addParticle semantics: no speed.
            assert_eq!(s.count, 0);
            assert_eq!(s.max_speed, [1.0, 1.0, 1.0]);
            assert_eq!(s.offset, [0.0, 0.0, 0.0]);
        }
        // The samples are triangular around the player: none farther than the radii.
        for s in &out {
            assert!(s.pos.y.floor() == 64.0, "only the y=64 plane has torches");
        }
    }

    /// A reused buffer holds only the current tick's samples: the spawns of a
    /// previous tick are dropped, not appended to (vanilla's `animateTick` starts
    /// from nothing each tick; a broken torch stops spawning immediately because
    /// nothing replays the old positions).
    #[test]
    #[ignore = "needs the datagen reports in the cache"]
    fn animate_tick_replaces_the_buffer_not_appends() {
        let Some(data) = game_data() else {
            eprintln!("ignored: needs the data generator reports in the cache");
            return;
        };
        let torch = state_of(&data, "torch");
        let world = Flat(torch);
        let mut out = Vec::new();
        animate_tick(&world, &data, BlockPos::new(0, 64, 0), false, &mut out);
        let first = out.len();
        assert!(first > 0, "no spawns with torches around");
        animate_tick(&world, &data, BlockPos::new(0, 64, 0), false, &mut out);
        assert_eq!(out.len(), first, "the second tick appended to the first");
        // The torch is gone (broken): the same world without it spawns nothing, and
        // the stale spawns from when it stood are not replayed.
        let air = Flat(BlockStateId::AIR);
        animate_tick(&air, &data, BlockPos::new(0, 64, 0), false, &mut out);
        assert!(out.is_empty(), "a broken torch still spawns: {}", out.len());
    }

    /// The torch odds over the whole sample: 667 * 2 draws, the y coordinate is
    /// `64 + nextInt(r) - nextInt(r)` — the chance y lands on 64 is roughly
    /// 1 in ~sqrt(2*r) for the triangular difference; assert the observed fraction
    /// stays in a wide band (the exact rate is vanilla's, the band catches
    /// regressions like a wrong radius or count).
    #[test]
    #[ignore = "needs the datagen reports in the cache"]
    fn torch_animate_tick_rate_is_in_the_vanilla_band() {
        let Some(data) = game_data() else {
            eprintln!("ignored: needs the data generator reports in the cache");
            return;
        };
        let torch = state_of(&data, "torch");
        let world = Flat(torch);
        let mut out = Vec::new();
        animate_tick(&world, &data, BlockPos::new(0, 64, 0), false, &mut out);
        // P(nextInt(16) - nextInt(16) == 0) = (2*16-1)/256 ~ 0.121, same for 32
        // (63/1024 ~ 0.062); averaged over both radii ~0.091 of samples hit y=64.
        let samples = SAMPLES as usize * RADII.len();
        let hits = out.len() / 2;
        let rate = hits as f64 / samples as f64;
        assert!(
            (0.05..0.14).contains(&rate),
            "torch hit rate {rate} outside the vanilla band"
        );
    }

    /// Lava: the surface pop is 1 in 100 per sample, and only with air above.
    #[test]
    #[ignore = "needs the datagen reports in the cache"]
    fn lava_animate_tick_pops_only_under_air() {
        let Some(data) = game_data() else {
            eprintln!("ignored: needs the data generator reports in the cache");
            return;
        };
        let lava = state_of(&data, "lava");
        let world = Flat(lava);
        let mut out = Vec::new();
        animate_tick(&world, &data, BlockPos::new(0, 64, 0), false, &mut out);
        let pops = out.iter().filter(|s| s.kind.path() == "lava").count();
        // ~1% of the y=64 hits (~9% of samples) fire a pop: a handful.
        assert!(pops > 0, "no lava pops in a sea of lava");
        // The drip rule also queues dripping_lava from the fluid branch.
        assert!(out.iter().any(|s| s.kind.path() == "dripping_lava"));
    }

    /// Sprint-particle rules live in the renderer's pool; here the queue contract:
    /// a nether portal block spawns 4 portal particles per hit.
    #[test]
    #[ignore = "needs the datagen reports in the cache"]
    fn portal_animate_tick_spawns_four() {
        let Some(data) = game_data() else {
            eprintln!("ignored: needs the data generator reports in the cache");
            return;
        };
        let portal = state_of(&data, "nether_portal");
        let world = Flat(portal);
        let mut out = Vec::new();
        animate_tick(&world, &data, BlockPos::new(0, 64, 0), false, &mut out);
        let portals = out.iter().filter(|s| s.kind.path() == "portal").count();
        assert!(portals > 0, "no portal particles");
        // In a sea of portals the west/east neighbours are portals, so the else
        // branch runs: z is pinned to 0.5 ± 0.25 (x stays free).
        for s in out.iter().filter(|s| s.kind.path() == "portal") {
            let rel = s.pos - s.pos.floor();
            assert!(
                (rel.z - 0.75).abs() < 1e-9 || (rel.z - 0.25).abs() < 1e-9,
                "z {} not pinned",
                rel.z
            );
        }
    }
}
