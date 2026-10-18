//! Meshing performance and output stability on terrain-like sections: a deterministic
//! synthetic world (fixed seed; stone below the surface, dirt and grass, sand, ores, caves,
//! a lake, leaves and flowers) over 9x9 chunks, meshed with the real jar's baked models,
//! atlas and tint/fluid tables from the cache.
//!
//! - `mesher_perf`: measures capture and mesh separately (sections/s, µs p50/p99 per section,
//!   vertices/section, allocations per section via a counting global allocator) and prints
//!   the numbers.
//! - `mesher_output_stable`: meshes the same sections and hashes the vertex data, compared
//!   against the hash produced by the original mesher. Any change to the mesher's output
//!   breaks this test.
//!
//! Needs the client jar and datagen reports in the cache (source scripts/env.sh); skipped
//! without them.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Instant;

use haste_assets::RenderLayer;
use haste_core::view::BlockStateTable;
use haste_core::{BlockPos, BlockStateId, ChunkPos, SectionPos};
use haste_render::mesher::{mesh_section_with, MeshSnapshot, MeshTables};
use haste_render::{fluid, tint};

/// Counts allocations and bytes while active (bench only; thread-local would hide the
/// worker threads, so this is a global counter).
static ALLOCATED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static ALLOCATION_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATION_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        ALLOCATED.fetch_add(layout.size(), std::sync::atomic::Ordering::Relaxed);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATION_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        ALLOCATED.fetch_add(new_size, std::sync::atomic::Ordering::Relaxed);
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// Fixed seed: the same world every run, so numbers are comparable and the stability
/// test's hash is reproducible.
const SEED: u64 = 0x5eed_1234;

/// One block of the generated world (state + light), for the whole 9x9-chunk area.
struct World {
    /// Blocks per (x, y, z) with x, z in 0..144, y in 0..128.
    blocks: Vec<BlockStateId>,
    sky: Vec<u8>,
}

/// Deterministic hash of a world position (a cheap positional noise source).
fn pos_hash(x: i32, y: i32, z: i32, salt: u64) -> u64 {
    let mut h = SEED ^ salt;
    h = h.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(x as u64);
    h = h.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(y as u64);
    h = h.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(z as u64);
    h ^= h >> 29;
    h.wrapping_mul(0xBF58476D1CE4E5B9)
}

impl World {
    const SIZE: i32 = 144; // 9 chunks
    const HEIGHT: i32 = 128;

    fn new(states: &dyn BlockStateTable) -> World {
        // Find each block's state by name and property values (bake-time lookup, not hot).
        let state = |name: &str, props: &[(&str, &str)]| {
            (0..states.len() as u32)
                .map(BlockStateId)
                .find(|s| {
                    states.block_of(*s).path() == name
                        && props
                            .iter()
                            .all(|(k, v)| states.property_value(*s, k) == Some(*v))
                })
                .unwrap_or_else(|| panic!("no state for {name} {props:?}"))
        };
        let stone = state("stone", &[]);
        let dirt = state("dirt", &[]);
        let grass = state("grass_block", &[("snowy", "false")]);
        let sand = state("sand", &[]);
        let water = state("water", &[]);
        let ore = state("iron_ore", &[]);
        let leaves = state("oak_leaves", &[("persistent", "true")]);
        let flower = state("dandelion", &[]);

        // Rolling surface height: sum of sines plus a hash-based jitter, 60..72.
        let height = |x: i32, z: i32| {
            let (fx, fz) = (x as f32, z as f32);
            let base = 64.0
                + 6.0 * (fx * 0.031).sin() * (fz * 0.027).cos()
                + 3.0 * (fx * 0.071 + fz * 0.043).sin();
            let jitter = (pos_hash(x, 0, z, 7) % 5) as f32;
            (base + jitter) as i32
        };

        let mut blocks = Vec::with_capacity((Self::SIZE * Self::SIZE * Self::HEIGHT) as usize);
        let mut sky = Vec::with_capacity(blocks.capacity());
        let mut sky_column = vec![15u8; (Self::SIZE * Self::SIZE) as usize];
        for z in 0..Self::SIZE {
            for x in 0..Self::SIZE {
                let h = height(x, z);
                for y in 0..Self::HEIGHT {
                    // Surface: grass above the lake line, sand around it.
                    let s = if y > h {
                        if y <= 62 {
                            water
                        } else {
                            BlockStateId::AIR
                        }
                    } else if y == h {
                        if h <= 63 {
                            sand
                        } else {
                            grass
                        }
                    } else if y > h - 4 {
                        if h <= 63 {
                            sand
                        } else {
                            dirt
                        }
                    } else {
                        // Caves: two overlapping noise pockets, never at the world floor.
                        let cave = pos_hash(x, y, z, 11) % 100 < 4
                            || pos_hash(x / 3, y / 2, z / 3, 13) % 100 < 8;
                        if y > 4 && y < h - 6 && cave {
                            BlockStateId::AIR
                        } else if pos_hash(x, y, z, 17).is_multiple_of(512) {
                            ore
                        } else {
                            stone
                        }
                    };
                    // Sky light: 15 above the first solid block, 0 under it (caves dark).
                    if y <= h && s != BlockStateId::AIR {
                        sky_column[(z * Self::SIZE + x) as usize] = 0;
                    }
                    blocks.push(s);
                    sky.push(sky_column[(z * Self::SIZE + x) as usize]);
                    // Flowers and leaf blobs sit on grass, in the open.
                    if s == BlockStateId::AIR && y == h + 1 && h > 63 {
                        let r = pos_hash(x, y, z, 19);
                        if r.is_multiple_of(64) {
                            *blocks.last_mut().unwrap() = flower;
                        } else if r % 64 < 6 {
                            *blocks.last_mut().unwrap() = leaves;
                        }
                    }
                }
                // Reset the column light for the next column.
                sky_column[(z * Self::SIZE + x) as usize] = 15;
            }
        }
        World { blocks, sky }
    }

    fn get(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        self.blocks[((y * Self::SIZE + z) * Self::SIZE + x) as usize]
    }
}

impl haste_core::view::BlockView for World {
    fn block_state(&self, p: BlockPos) -> BlockStateId {
        if p.y < 0
            || p.y >= Self::HEIGHT
            || p.x < 0
            || p.x >= Self::SIZE
            || p.z < 0
            || p.z >= Self::SIZE
        {
            return BlockStateId::AIR;
        }
        self.get(p.x, p.y, p.z)
    }
    fn is_chunk_loaded(&self, _: ChunkPos) -> bool {
        true
    }
    fn sky_light(&self, p: BlockPos) -> u8 {
        if p.y < 0
            || p.y >= Self::HEIGHT
            || p.x < 0
            || p.x >= Self::SIZE
            || p.z < 0
            || p.z >= Self::SIZE
        {
            return 15;
        }
        self.sky[((p.y * Self::SIZE + p.z) * Self::SIZE + p.x) as usize]
    }
    fn block_light(&self, _p: BlockPos) -> u8 {
        0
    }
    fn min_y(&self) -> i32 {
        0
    }
    fn height(&self) -> i32 {
        Self::HEIGHT
    }
}

/// A biome source with two biomes split by a diagonal, so tints exercise blending.
struct TwoBiomes {
    plains: haste_core::BiomeId,
    forest: haste_core::BiomeId,
}

impl tint::BiomeSource for TwoBiomes {
    fn biome(&self, p: BlockPos) -> haste_core::BiomeId {
        if p.x + p.z > World::SIZE {
            self.forest
        } else {
            self.plains
        }
    }
}

/// The sections the benchmark meshes: all non-empty ones over the 9x9 chunks
/// (every section of the 8 middle chunks plus the 1-block border of the outer ones).
fn benchmark_sections() -> Vec<SectionPos> {
    (0..9)
        .flat_map(|cx| (0..9).map(move |cz| (cx, cz)))
        .flat_map(|(cx, cz)| (0..8).map(move |y| SectionPos::new(cx, y, cz)))
        .collect()
}

fn setup() -> Option<(
    haste_assets::BlocksJsonStates,
    haste_assets::BakedBlockModels,
    haste_assets::AtlasBuilder,
    MeshTables,
)> {
    let jar = haste_core::paths::client_jar();
    if !jar.exists() {
        eprintln!("ignored: needs the client jar in the cache");
        return None;
    }
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::cache_dir().join("assets"), "26.3")
            .unwrap();
    let states = haste_assets::BlocksJsonStates::load(
        &haste_core::paths::datagen_reports().join("blocks.json"),
    )
    .unwrap();
    let mut atlas = haste_assets::AtlasBuilder::build(&mut assets, &[], false).unwrap();
    let baked = haste_assets::BakedBlockModels::bake(&mut assets, &states, &mut atlas).unwrap();
    // Real occlusion shapes for the lake's fluid faces and the terrain face culling when
    // the game data is available; the bench needs the cache anyway (jar + reports), so
    // this is the common path.
    let (occluders, face_occluders) = haste_data::GameData::load()
        .map(|data| {
            (
                fluid::FluidOccluders::from_shapes(&states, &data.shapes, &baked),
                haste_render::mesher::FaceOccluders::from_shapes(&states, &data.shapes),
            )
        })
        .unwrap_or_else(|_| {
            (
                fluid::FluidOccluders::absent(),
                haste_render::mesher::FaceOccluders::absent(),
            )
        });
    let tables = MeshTables::new(
        std::sync::Arc::new(tint::TintTable::new(&states)),
        std::sync::Arc::new(
            tint::BiomeColors::new(
                &[
                    tint::BiomeClimate::plains(),
                    tint::BiomeClimate {
                        temperature: 0.7,
                        downfall: 0.8,
                        ..tint::BiomeClimate::plains()
                    },
                ],
                &tint::Colormaps::load(&mut assets).unwrap(),
            )
            .with_radius(tint::DEFAULT_BLEND_RADIUS),
        ),
        std::sync::Arc::new(fluid::FluidTable::new(
            &states,
            &fluid::Fluids::vanilla(&fluid::FluidSprites::from_atlas(&atlas)),
            &fluid::FluidRules::vanilla(),
            fluid::FluidSprites::from_atlas(&atlas),
        )),
        // The bench world is full cubes: AO treats them all as occluders.
        std::sync::Arc::new(haste_render::mesher::AoTable::all_solid(states.len())),
        std::sync::Arc::new(occluders),
        std::sync::Arc::new(face_occluders),
    );
    Some((states, baked, atlas, tables))
}

/// Hash of one section's meshed vertex data (the stability oracle). Order-insensitive:
/// the baked quads of a model come from a HashMap iteration (haste-assets model.rs
/// `ElementDef.faces`), so vertex order varies per process while the geometry does not.
/// Each triangle is hashed with its three vertices' full content, rotated so the smallest
/// vertex comes first (winding preserved), and the per-layer triangle multisets are
/// hashed sorted.
fn mesh_hash(mesh: &haste_render::SectionMesh) -> u64 {
    let vertex_key = |i: u32| {
        let v = &mesh.vertices[i as usize];
        let mut h = DefaultHasher::new();
        v.pos.hash(&mut h);
        v.uv.hash(&mut h);
        v.light.hash(&mut h);
        v.sprite.hash(&mut h);
        v.tint_b.hash(&mut h);
        h.finish()
    };
    let mut h = DefaultHasher::new();
    mesh.vertices.len().hash(&mut h);
    for (layer, indices) in &mesh.layers {
        layer_index(*layer).hash(&mut h);
        let mut triangles: Vec<u64> = indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let keys = [vertex_key(t[0]), vertex_key(t[1]), vertex_key(t[2])];
                // Rotate so the smallest key leads (winding intact), then hash.
                let rot = keys
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, k)| **k)
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                let mut th = DefaultHasher::new();
                for k in 0..3 {
                    keys[(rot + k) % 3].hash(&mut th);
                }
                th.finish()
            })
            .collect();
        triangles.sort_unstable();
        triangles.len().hash(&mut h);
        for t in triangles {
            t.hash(&mut h);
        }
    }
    h.finish()
}

fn layer_index(layer: RenderLayer) -> u8 {
    match layer {
        RenderLayer::Solid => 0,
        RenderLayer::Cutout => 1,
        RenderLayer::CutoutMipped => 2,
        RenderLayer::Translucent => 3,
    }
}

#[test]
#[ignore = "needs the client jar and datagen reports in the cache"]
fn mesher_output_stable() {
    let Some((states, baked, atlas, tables)) = setup() else {
        return;
    };
    let world = World::new(&states);
    let biomes = TwoBiomes {
        plains: haste_core::BiomeId(0),
        forest: haste_core::BiomeId(1),
    };
    let mut hash = DefaultHasher::new();
    let mut quads = 0usize;
    let mut vertices = 0usize;
    let mut sprites = (u16::MAX, 0u16);
    for pos in benchmark_sections() {
        let snapshot = MeshSnapshot::capture_with_biomes(
            &world,
            Some(&biomes),
            tint::DEFAULT_BLEND_RADIUS,
            pos,
        );
        let mesh = mesh_section_with(&snapshot, &baked, &atlas, &tables, pos);
        mesh_hash(&mesh).hash(&mut hash);
        quads += mesh.layers.iter().map(|(_, i)| i.len() / 6).sum::<usize>();
        vertices += mesh.vertices.len();
        for v in &mesh.vertices {
            sprites.0 = sprites.0.min(v.sprite);
            sprites.1 = sprites.1.max(v.sprite);
        }
    }
    eprintln!(
        "stability: {quads} quads, {vertices} vertices, sprite range {}..{}",
        sprites.0, sprites.1
    );
    // Expected: the original mesher's output until the diagonal substitution's
    // permeability test moved to the one-out cells (vanilla's side + normal)
    // and the zero-light fallback was gated off: the vertices next to
    // occluders change. Quad count, vertices and sprites are unchanged.
    // Then the face-coverage cull (vanilla `Block.shouldRenderFace`'s shape
    // comparison, mesher/culling.rs) removed the interior faces between
    // partial blocks that cover each other (snow layers, slabs): fewer quads.
    // Then the 2026-10-18 dump regeneration (the `occlusion` field) changed the
    // water faces the outline rule left visible; on this world of full cubes,
    // empty shapes and non-adjacent flowers both culling rules agree, so the
    // occlusion-based rules (mesher/face_occlusion.rs, fluid/occlusion.rs) keep
    // the same quads and vertices as the model rules.
    let expected: u64 = 9_991_751_837_968_507_622;
    assert_eq!(hash.finish(), expected, "mesher output changed");
    assert!(quads > 100_000, "suspiciously few quads: {quads}");
    eprintln!("{quads} quads, hash {:016x}", hash.finish());
}

#[test]
#[ignore = "needs the client jar and datagen reports in the cache"]
fn mesher_perf() {
    let Some((states, baked, atlas, tables)) = setup() else {
        return;
    };
    let world = World::new(&states);
    let biomes = TwoBiomes {
        plains: haste_core::BiomeId(0),
        forest: haste_core::BiomeId(1),
    };
    let sections = benchmark_sections();

    // Warm-up (page in the baked models, allocator, caches), then 3 runs.
    let mut snapshots = Vec::with_capacity(sections.len());
    for pos in &sections {
        snapshots.push(MeshSnapshot::capture_with_biomes(
            &world,
            Some(&biomes),
            tint::DEFAULT_BLEND_RADIUS,
            *pos,
        ));
    }
    for snapshot in &snapshots {
        let _ = mesh_section_with(
            snapshot,
            &baked,
            &atlas,
            &tables,
            snapshot_section(snapshot),
        );
    }

    for run in 1..=3 {
        // Capture: per-section timings, then summarized (sections/s, p50, p99).
        let mut timings: Vec<f64> = Vec::with_capacity(sections.len());
        let t = Instant::now();
        snapshots.clear();
        for pos in &sections {
            let start = Instant::now();
            snapshots.push(MeshSnapshot::capture_with_biomes(
                &world,
                Some(&biomes),
                tint::DEFAULT_BLEND_RADIUS,
                *pos,
            ));
            timings.push(start.elapsed().as_secs_f64() * 1e6);
        }
        let capture_total = t.elapsed();
        timings.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = sections.len();
        let capture_p50 = timings[n * 50 / 100];
        let capture_p99 = timings[n * 99 / 100];

        // Mesh (one thread): measures mesh_section_with only.
        let alloc0 = ALLOCATION_COUNT.load(std::sync::atomic::Ordering::Relaxed);
        let bytes0 = ALLOCATED.load(std::sync::atomic::Ordering::Relaxed);
        timings.clear();
        let t = Instant::now();
        let mut vertices = 0usize;
        for snapshot in &snapshots {
            let start = Instant::now();
            let mesh = mesh_section_with(
                snapshot,
                &baked,
                &atlas,
                &tables,
                snapshot_section(snapshot),
            );
            timings.push(start.elapsed().as_secs_f64() * 1e6);
            vertices += mesh.vertices.len();
        }
        let mesh_total = t.elapsed();
        let allocs = ALLOCATION_COUNT.load(std::sync::atomic::Ordering::Relaxed) - alloc0;
        let bytes = ALLOCATED.load(std::sync::atomic::Ordering::Relaxed) - bytes0;

        timings.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p50 = timings[n * 50 / 100];
        let p99 = timings[n * 99 / 100];
        let max = timings[n - 1];
        eprintln!(
            "run {run}: {n} sections, {vertices} vertices ({}/section), \
             capture {:.0} sections/s ({:.0} µs p50, {:.0} µs p99), \
             mesh {:.0} sections/s ({:.0} µs p50, {:.0} µs p99, {:.0} µs max), \
             mesh allocations {:.1}/section ({:.1} KB)",
            vertices / n,
            n as f64 / capture_total.as_secs_f64(),
            capture_p50,
            capture_p99,
            n as f64 / mesh_total.as_secs_f64(),
            p50,
            p99,
            max,
            allocs as f64 / n as f64,
            bytes as f64 / n as f64 / 1024.0,
        );
    }
}

/// A snapshot's section position (the capture order matches `benchmark_sections`).
fn snapshot_section(snapshot: &MeshSnapshot) -> SectionPos {
    snapshot.origin().add(1, 1, 1).section()
}
