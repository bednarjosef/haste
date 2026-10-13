//! Chunk-load benchmark: `World::load_chunk` per chunk (median, p99), chunks/s on one
//! thread, resident memory per chunk and allocations per chunk (counting global allocator).
//!
//! Chunks are synthetic but shaped like a real 26.3 overworld chunk the server sends: 24
//! sections from y=-64, single-value stone deep underground, ore belts (2-bit palettes), a
//! cave section (3-bit), the surface belt (3-bit, stone/dirt/grass/sand/water), single-value
//! air above ground, biome containers, two heightmaps, 8 block entities, full light. Fixed
//! seeds, so runs (and code versions) are comparable.
//!
//! Run: `cargo test -q -p haste-world --release -- --ignored --nocapture load_bench`
//! (`benchmark_chunks_decode_stably` runs in every plain `cargo test`).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use haste_core::chunk::{BlockEntityData, ChunkData, LightData};
use haste_core::game::DimensionInfo;
use haste_core::view::DirtyTracker as _;
use haste_core::{BlockPos, BlockStateId, ChunkPos};
use haste_data::GameData;
use haste_world::palette::{PackedArray, WireWriter, BLOCKS_PER_SECTION};
use haste_world::World;

/// A realistic global block state count (26.3: ~28k states) so `direct_bits` matches the
/// client's; ids in the payload stay within it.
const STATE_COUNT: u32 = 28_000;
const STONE: u32 = 1;
const DIRT: u32 = 2;
const GRASS: u32 = 8;
const SAND: u32 = 12;
const WATER: u32 = 90;
const ORES: [u32; 2] = [100, 101];
const CAVES: [u32; 8] = [1, 2, 3, 4, 5, 6, 7, 9];

fn overworld() -> DimensionInfo {
    DimensionInfo {
        name: haste_core::Identifier::minecraft("overworld"),
        dimension_type: haste_core::Identifier::minecraft("overworld"),
        min_y: -64,
        height: 384,
        has_skylight: true,
    }
}

/// `blocks.json`/`registries.json` fixture with `STATE_COUNT` dense states so `direct_bits`
/// is the real client's width.
fn game_data() -> Arc<GameData> {
    use std::sync::atomic::{AtomicU64 as A64, Ordering as O};
    static NEXT: A64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "haste-world-bench-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, O::Relaxed)
    ));
    let reports = dir.join("reports");
    std::fs::create_dir_all(&reports).unwrap();
    // ~28k states as 16-state blocks with dense ascending ids; air is state 0.
    let mut blocks = String::from(r#"{"minecraft:air":{"states":[{"id":0,"default":true}]},"#);
    let mut next = 1u32;
    let mut reg = String::from(
        r#"{"minecraft:block":{"default":"minecraft:air","entries":{"minecraft:air":{"protocol_id":0},"#,
    );
    let mut idx = 1u32;
    while next < STATE_COUNT {
        let n = 16.min(STATE_COUNT - next);
        let name = format!("minecraft:b{idx}");
        blocks.push_str(&format!(
            r#""{name}":{{"properties":{{"p":["0","1","2","3","4","5","6","7","8","9","10","11","12","13","14","15"]}},"states":[{s}]}},"#,
            s = (0..n)
                .map(|i| {
                    format!(
                        r#"{{"id":{},"default":{},"properties":{{"p":"{}"}}}}"#,
                        next + i,
                        i == 0,
                        i
                    )
                })
                .collect::<Vec<_>>()
                .join(",")
        ));
        reg.push_str(&format!(r#""{name}":{{"protocol_id":{idx}}},"#));
        next += n;
        idx += 1;
    }
    blocks.pop();
    blocks.push('}');
    reg.pop();
    reg.push_str("}}}");
    std::fs::write(reports.join("blocks.json"), &blocks).unwrap();
    std::fs::write(reports.join("registries.json"), &reg).unwrap();
    Arc::new(GameData::load_from(&reports, None).unwrap())
}

/// Deterministic PRNG (xorshift32) so payloads are identical every run.
struct Rng(u32);
impl Rng {
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
}

/// How a section's block container is put on the wire.
enum Blocks<'a> {
    /// bits-per-entry 0: every entry is this global id.
    Single(u32),
    /// Indirect: palette plus packed palette indices.
    Indirect(&'a [u32], &'a PackedArray),
}

/// Encodes one section: block count, fluid count, block container, biome container
/// (indirect, 2 bits, 4 biomes — the common server shape).
fn encode_section(
    w: &mut WireWriter,
    block_count: i16,
    fluid_count: i16,
    blocks: Blocks<'_>,
    biomes: &PackedArray,
) {
    w.i16_be(block_count);
    w.i16_be(fluid_count);
    match blocks {
        Blocks::Single(v) => {
            w.varint(0); // bpe 0: single value
            w.varint(v);
        }
        Blocks::Indirect(palette, data) => {
            w.varint(data.bits() as u32);
            w.varint(palette.len() as u32);
            for &p in palette {
                w.varint(p);
            }
            for &l in data.longs() {
                w.i64_be(l);
            }
        }
    }
    w.varint(2); // biome bpe
    w.varint(4);
    for b in 0..4u32 {
        w.varint(b);
    }
    for &l in biomes.longs() {
        w.i64_be(l);
    }
}

/// Builds one realistic overworld `ChunkData` (seed varies per chunk; the shape is fixed).
fn realistic_chunk(pos: ChunkPos, seed: u32) -> ChunkData {
    let mut rng = Rng(seed | 1);
    let mut w = WireWriter::new();
    for si in 0..24usize {
        // biome container: 2-bit indices into [0,1,2,3]
        let mut biomes = PackedArray::zeroed(2, 64);
        for i in 0..64 {
            biomes.write_at(i, rng.next() % 4);
        }
        match si {
            // 0..=3 (y -64..-1): solid deepslate belt, single-value stone
            0..=3 => encode_section(&mut w, 4096, 0, Blocks::Single(STONE), &biomes),
            // 4..=7 (y 0..63): stone with sparse ores, 2-bit palette
            4..=7 => {
                let palette = [STONE, ORES[0], ORES[1]];
                let bits = 2;
                let mut data = PackedArray::zeroed(bits, BLOCKS_PER_SECTION);
                let mut bc = 0;
                for i in 0..BLOCKS_PER_SECTION {
                    let idx = if rng.next().is_multiple_of(32) {
                        1 + (rng.next() % 2)
                    } else {
                        0
                    };
                    bc += (idx == 0) as i16;
                    data.write_at(i, idx);
                }
                encode_section(&mut w, bc, 0, Blocks::Indirect(&palette, &data), &biomes)
            }
            // 8 (y 64..79): cave section, full 3-bit palette
            8 => {
                let bits = 3;
                let mut data = PackedArray::zeroed(bits, BLOCKS_PER_SECTION);
                for i in 0..BLOCKS_PER_SECTION {
                    data.write_at(i, rng.next() % CAVES.len() as u32);
                }
                encode_section(&mut w, 4096, 0, Blocks::Indirect(&CAVES, &data), &biomes)
            }
            // 9..=10 (y 80..111): surface belt, stone/dirt/grass/sand/water
            9 | 10 => {
                let palette = [STONE, DIRT, GRASS, SAND, WATER];
                let bits = 3;
                let mut data = PackedArray::zeroed(bits, BLOCKS_PER_SECTION);
                let (mut bc, mut fluids) = (0i16, 0i16);
                for i in 0..BLOCKS_PER_SECTION {
                    let idx = match rng.next() % 10 {
                        0..=2 => 1, // dirt
                        3 => 2,     // grass
                        4 => 3,     // sand
                        5 => 4,     // water
                        _ => 0,     // stone
                    };
                    if idx == 4 {
                        fluids += 1;
                    } else {
                        bc += 1;
                    }
                    data.write_at(i, idx);
                }
                encode_section(
                    &mut w,
                    bc,
                    fluids,
                    Blocks::Indirect(&palette, &data),
                    &biomes,
                )
            }
            // 11..=23 (y 112..319): single-value air
            _ => encode_section(&mut w, 0, 0, Blocks::Single(0), &biomes),
        }
    }

    // two heightmaps (world_surface, motion_blocking), 9 bits, 256 entries
    let mut heightmaps = Vec::with_capacity(2);
    for ty in [0i32, 1] {
        let bits = 9;
        let mut packed = vec![0i64; PackedArray::long_count(bits, 256)];
        for (i, l) in packed.iter_mut().enumerate() {
            // entries 0..7 per long, ground height ~ y 88-100
            for e in 0..(64 / bits) {
                *l |= ((88 + rng.next() % 12) as i64) << (e * bits);
            }
            let _ = i;
        }
        heightmaps.push((ty, packed));
    }

    // 8 block entities (chests/signs/spawners) with small NBT payloads
    let mut block_entities = Vec::new();
    for i in 0..8u32 {
        block_entities.push(BlockEntityData {
            pos: BlockPos::new(
                (i % 16) as i32,
                64 + (i / 2) as i32 * 8,
                ((i * 3) % 16) as i32,
            ),
            kind: (rng.next() % 40) + 1,
            nbt: bytes::Bytes::from(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]),
        });
    }

    // full light for all 26 wire indices, shaped like daylight: above ground (wire idx > 13)
    // sky is all 15s and block light all 0s; below ground both vary
    let wire: usize = 24 + 2;
    let mut sky_mask = vec![0u64; wire.div_ceil(64)];
    let mut block_mask = vec![0u64; wire.div_ceil(64)];
    let mut sky_arrays = Vec::with_capacity(wire);
    let mut block_arrays = Vec::with_capacity(wire);
    for i in 0..wire {
        sky_mask[i / 64] |= 1 << (i % 64);
        block_mask[i / 64] |= 1 << (i % 64);
        let sky = if i > 13 {
            vec![0xFF; 2048]
        } else {
            vec![(i % 16) as u8 | 0xF0; 2048]
        };
        let block = if i > 13 {
            vec![0u8; 2048]
        } else {
            vec![(i % 8) as u8 | 0x70; 2048]
        };
        sky_arrays.push(bytes::Bytes::from(sky));
        block_arrays.push(bytes::Bytes::from(block));
    }

    ChunkData {
        pos,
        heightmaps,
        sections: bytes::Bytes::from(w.data),
        block_entities,
        light: LightData {
            sky_mask,
            block_mask,
            empty_sky_mask: vec![0],
            empty_block_mask: vec![0],
            sky_arrays,
            block_arrays,
        },
    }
}

/// Counting global allocator (bench binary only). Counting is off unless
/// `HASTE_BENCH_COUNT_ALLOC` is set: the two atomics per allocation distort the load path.
struct Counting;
static COUNT: AtomicU64 = AtomicU64::new(0);
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if COUNT.load(Ordering::Relaxed) != 0 {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(l.size() as u64, Ordering::Relaxed);
        }
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, ns: usize) -> *mut u8 {
        if COUNT.load(Ordering::Relaxed) != 0 {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(ns as u64, Ordering::Relaxed);
        }
        unsafe { System.realloc(p, l, ns) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Sums the bytes the chunk keeps allocated: section containers (palette + packed longs),
/// heightmaps, block entities, light. Shared `Bytes` (zero-copy light/NBT slices of the
/// packet) count at their full length, like the wire memory they pin.
fn resident_bytes(c: &haste_world::Chunk) -> u64 {
    use haste_world::palette::Storage;
    let mut total = 0u64;
    let container = |s: &Storage, total: &mut u64| match s {
        Storage::Single(_) => {}
        Storage::Indirect { palette, data } => {
            *total += palette.len() as u64 * 4 + data.longs().len() as u64 * 8;
        }
        Storage::Direct(data) => *total += data.longs().len() as u64 * 8,
    };
    for s in &c.sections {
        container(&s.blocks.storage, &mut total);
        container(&s.biomes.storage, &mut total);
    }
    for (_, h) in &c.heightmaps {
        total += h.longs.len() as u64 * 8;
    }
    for be in c.block_entities.values() {
        total += 48 + be.nbt.len() as u64;
    }
    total += c
        .light
        .sky
        .iter()
        .flatten()
        .map(|n| n.0.len() as u64)
        .sum::<u64>();
    total += c.light.block.iter().map(|n| n.0.len() as u64).sum::<u64>();
    total
}

/// FNV-1a over every block state id, biome id and section count in the chunk (fixed order).
fn state_hash(w: &World, pos: ChunkPos) -> u64 {
    let chunk = w.chunk(pos).unwrap();
    let mut hash = 0xcbf29ce484222325u64;
    let feed = |v: u64, hash: &mut u64| {
        *hash ^= v;
        *hash = hash.wrapping_mul(0x100000001b3);
    };
    for s in &chunk.sections {
        feed(s.block_count as u64, &mut hash);
        feed(s.fluid_count as u64, &mut hash);
        for i in 0..BLOCKS_PER_SECTION {
            feed(s.blocks.get_index(i) as u64, &mut hash);
        }
        for i in 0..64 {
            feed(s.biomes.get_index(i) as u64, &mut hash);
        }
    }
    hash
}

/// The benchmark: 3 runs x N chunks; prints median/p99 per chunk, chunks/s, allocations and
/// allocated bytes per chunk, resident container bytes, and the state hashes.
#[test]
#[ignore = "perf: run with --release --ignored --nocapture"]
fn load_bench() {
    let n = 200;
    // a spread of positions over a 32-wide grid, so neighbour dirtying is realistic
    let chunks: Vec<ChunkData> = (0..n as u32)
        .map(|i| realistic_chunk(ChunkPos::new(i as i32 % 32, (i / 32) as i32), i * 7919 + 3))
        .collect();

    for run in 0..3 {
        let mut world = World::new_with_biomes(overworld(), game_data(), 67);
        let mut times = Vec::with_capacity(n);
        // allocations counted for run 0 only (counting costs ~2 atomics per allocation)
        if run == 0 {
            COUNT.store(1, Ordering::Relaxed);
        }
        let a0 = ALLOCS.load(Ordering::Relaxed);
        let b0 = BYTES.load(Ordering::Relaxed);
        let t0 = Instant::now();
        for cd in &chunks {
            let t = Instant::now();
            world.load_chunk(cd).unwrap();
            times.push(t.elapsed());
            let _ = world.take_dirty_sections();
        }
        let total = t0.elapsed();
        let da = ALLOCS.load(Ordering::Relaxed) - a0;
        let db = BYTES.load(Ordering::Relaxed) - b0;
        COUNT.store(0, Ordering::Relaxed);

        times.sort();
        let median = times[n / 2].as_nanos() as f64 / 1000.0;
        let p99 = times[(n * 99) / 100].as_nanos() as f64 / 1000.0;
        let cps = n as f64 / total.as_secs_f64();
        if run == 0 {
            eprintln!(
                "run {} (alloc counting on): median {median:.1} us, p99 {p99:.1} us, \
                 {cps:.0} chunks/s, {} allocs/chunk, {} alloc bytes/chunk",
                run + 1,
                da / n as u64,
                db / n as u64
            );
        } else {
            eprintln!(
                "run {}: median {median:.1} us, p99 {p99:.1} us, {cps:.0} chunks/s",
                run + 1
            );
        }
    }

    // steady-state timing: repeat the whole 200-chunk load 5 times on a pre-warmed world,
    // report the best full pass (the machine runs other workstreams; min filters them out)
    {
        let mut best_median = f64::INFINITY;
        let mut best_p99 = f64::INFINITY;
        let mut best_cps = 0.0;
        for _ in 0..5 {
            let mut world = World::new_with_biomes(overworld(), game_data(), 67);
            let mut times = Vec::with_capacity(n);
            let t0 = Instant::now();
            for cd in &chunks {
                let t = Instant::now();
                world.load_chunk(cd).unwrap();
                times.push(t.elapsed());
                let _ = world.take_dirty_sections();
            }
            let total = t0.elapsed();
            times.sort();
            let median = times[n / 2].as_nanos() as f64 / 1000.0;
            let p99 = times[(n * 99) / 100].as_nanos() as f64 / 1000.0;
            let cps = n as f64 / total.as_secs_f64();
            if median < best_median {
                best_median = median;
                best_p99 = p99;
                best_cps = cps;
            }
        }
        eprintln!(
            "load_chunk best pass: median {best_median:.1} us, p99 {best_p99:.1} us, \
             {best_cps:.0} chunks/s"
        );
    }

    // decode alone (no world insert, no dirty tracking): shows where load_chunk's time goes
    {
        let mut best = f64::INFINITY;
        for _ in 0..5 {
            let t0 = Instant::now();
            for cd in &chunks {
                let _ = haste_world::Chunk::decode(cd, -4, 24, 15, 7, u32::MAX).unwrap();
            }
            let per = t0.elapsed().as_nanos() as f64 / n as f64 / 1000.0;
            best = best.min(per);
        }
        eprintln!("Chunk::decode alone: best {best:.1} us/chunk");
    }

    // same chunks with light stripped: isolates what light application costs
    // (mask walking + uniform-array scan + Bytes clones)
    {
        let no_light: Vec<ChunkData> = chunks
            .iter()
            .map(|cd| {
                let mut cd = cd.clone();
                cd.light = LightData::default();
                cd
            })
            .collect();
        let mut best = f64::INFINITY;
        for _ in 0..5 {
            let t0 = Instant::now();
            for cd in &no_light {
                let _ = haste_world::Chunk::decode(cd, -4, 24, 15, 7, u32::MAX).unwrap();
            }
            let per = t0.elapsed().as_nanos() as f64 / n as f64 / 1000.0;
            best = best.min(per);
        }
        eprintln!("Chunk::decode without light: best {best:.1} us/chunk");
    }

    // resident size of loaded chunks (containers; allocator overhead excluded)
    let mut world = World::new_with_biomes(overworld(), game_data(), 67);
    let mut rb = 0u64;
    for cd in &chunks {
        world.load_chunk(cd).unwrap();
        rb += resident_bytes(world.chunk(cd.pos).unwrap());
    }
    eprintln!(
        "resident container bytes per chunk: {} (average over {n})",
        rb / n as u64
    );
    eprintln!(
        "state hash chunk 0: {:016x}",
        state_hash(&world, chunks[0].pos)
    );
    eprintln!(
        "state hash chunk 7: {:016x}",
        state_hash(&world, chunks[7].pos)
    );
}

/// Correctness anchor (runs in every `cargo test`): the benchmark chunks decode with the
/// expected shape and content, and the full block-state/biome hash matches the recorded
/// value (taken before the load-path optimizations; any behaviour change fails here without
/// the ignored bench).
#[test]
fn benchmark_chunks_decode_stably() {
    let mut world = World::new_with_biomes(overworld(), game_data(), 67);
    for (i, seed) in [(0i32, 3u32), (1, 7922), (7, 55436)] {
        let cd = realistic_chunk(ChunkPos::new(i, 0), seed);
        world.load_chunk(&cd).unwrap();
    }
    // every block state and biome of chunk 0, hashed (FNV-1a, fixed order)
    assert_eq!(state_hash(&world, ChunkPos::new(0, 0)), 0xa7e8f48376c8ee92);
    assert_eq!(state_hash(&world, ChunkPos::new(7, 0)), 0x5088a771ee4ad23a);
    let c = world.chunk(ChunkPos::new(0, 0)).unwrap();
    assert_eq!(c.sections.len(), 24);
    // deep belt: solid stone
    assert_eq!(c.sections[0].block_count, 4096);
    assert_eq!(c.sections[3].get(0, 0, 0), BlockStateId(STONE));
    // ore belt: stone or an ore
    let s5 = &c.sections[5];
    assert!(s5.block_count > 3900 && s5.block_count < 4096);
    let st = s5.get(3, 3, 3).0;
    assert!(st == STONE || st == ORES[0] || st == ORES[1]);
    // cave section: full 3-bit palette, every entry a caves id
    let s8 = &c.sections[8];
    assert_eq!(s8.block_count, 4096);
    assert!(CAVES.contains(&s8.get(9, 9, 9).0));
    // surface belt: known palette, water counted as fluid
    let s9 = &c.sections[9];
    assert!(s9.fluid_count > 0);
    assert!([STONE, DIRT, GRASS, SAND, WATER].contains(&s9.get(1, 1, 1).0));
    // air above
    assert_eq!(c.sections[11].block_count, 0);
    assert_eq!(c.sections[23].get(15, 15, 15), BlockStateId::AIR);
    // biome containers: 2-bit palette of 4
    assert!(c.sections[0].biome(0, 0, 0).0 < 4);
    // heightmaps and block entities came through
    assert_eq!(c.heightmaps.len(), 2);
    assert!(c.height_at(0, 0).unwrap() > 80);
    assert_eq!(c.block_entities.len(), 8);
    // light: sky arrays applied (sky_mask covers every wire index)
    assert!(c.light.sky[0].is_some());
}
