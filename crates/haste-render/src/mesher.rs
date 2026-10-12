// This file is derived from Pomme (https://github.com/josefbednar/pomme),
// pomme-client/src/renderer/chunk/mesher.rs (mesh_section / emit_baked_model /
// compute_face_ao / MeshSink), reused under GPL-3.0 and reworked for Haste:
// meshes per SectionPos against haste-assets' BakedBlockModels over a captured
// BlockView snapshot; light is packed as sky/block nibbles instead of baked
// brightness; the worker pool design follows Pomme's MeshDispatcher (bounded
// result channel, buffer pool, nearest-first camera queue). See THIRD_PARTY.md.

use std::sync::Arc;

use haste_assets::RenderLayer;
use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId, SectionPos};
use parking_lot::Mutex;
use rayon::prelude::*;

use crate::vertex::{pack_ao_shade, pack_light, pack_vertex, PackedVertex, QuadVertex};

/// One section's meshed geometry, split by render layer.
pub struct SectionMesh {
    pub pos: SectionPos,
    /// Vertices shared by all layers (section-local packed).
    pub vertices: Vec<PackedVertex>,
    /// (layer, indices) pairs in draw order; indices reference `vertices`.
    pub layers: Vec<(RenderLayer, Vec<u32>)>,
    /// Section-local AABB (min/max in blocks) for frustum culling.
    pub aabb: ([f32; 3], [f32; 3]),
    /// Face-to-face sightlines through the section (cave culling).
    pub visibility: crate::occlusion::VisibilitySet,
}

/// Tint sources resolved at mesh time (palette written into the shader's
/// uniform; indices carried per vertex). Phase 1: white + a few fixed colors.
pub const TINT_WHITE: u8 = 0;
#[allow(dead_code)]
pub const TINT_WATER: u8 = 1;
pub const TINT_PALETTE_LEN: usize = 2;

/// A snapshot of the 3x3x3 section neighbourhood, captured on the render/game
/// thread and sent to the mesh workers. Cheap random access, no locks.
pub struct MeshSnapshot {
    /// (x, y, z) block offsets -1..17 relative to the section origin: the
    /// section plus one-block borders into all neighbours (blocks for face
    /// culling and AO). Y is clamped at the world bounds (air outside).
    blocks: Vec<BlockStateId>,
    /// Sky light per block, same layout (15 where no data).
    sky: Vec<u8>,
    /// Block light per block, same layout.
    block: Vec<u8>,
    /// Section origin in world blocks.
    origin: BlockPos,
    /// World bounds for y clamping (kept for debug/documentation).
    #[allow(dead_code)]
    min_y: i32,
    #[allow(dead_code)]
    height: i32,
}

/// The section plus a one-block border (face culling, AO and smooth light only look one block
/// out): 18^3 = 5832 cells instead of the whole 3x3x3 neighbourhood.
const EXT: i32 = 18;
const EXT2: i32 = EXT * EXT;
const EXT3: usize = (EXT2 * EXT) as usize;

impl MeshSnapshot {
    /// Capture the neighbourhood of `pos` from a `BlockView`. Reads
    /// 18^3 = 5832 block states + light (the section and a one-block border),
    /// amortized by the dirty-section pipeline.
    pub fn capture(view: &dyn BlockView, pos: SectionPos) -> MeshSnapshot {
        let origin = pos.block_min().add(-1, -1, -1);
        let mut blocks = Vec::with_capacity(EXT3);
        let mut sky = Vec::with_capacity(EXT3);
        let mut block = Vec::with_capacity(EXT3);
        for dy in 0..EXT {
            let y = origin.y + dy;
            let in_bounds = y >= view.min_y() && y < view.min_y() + view.height();
            for dz in 0..EXT {
                for dx in 0..EXT {
                    let p = BlockPos::new(origin.x + dx, y, origin.z + dz);
                    if in_bounds {
                        blocks.push(view.block_state(p));
                        sky.push(view.sky_light(p));
                        block.push(view.block_light(p));
                    } else {
                        blocks.push(BlockStateId::AIR);
                        sky.push(15);
                        block.push(0);
                    }
                }
            }
        }
        MeshSnapshot {
            blocks,
            sky,
            block,
            origin,
            min_y: view.min_y(),
            height: view.height(),
        }
    }

    /// Block state at world position; out-of-bounds y reads as air.
    pub fn block_state(&self, p: BlockPos) -> BlockStateId {
        let (dx, dy, dz) = (
            p.x - self.origin.x,
            p.y - self.origin.y,
            p.z - self.origin.z,
        );
        if (0..EXT).contains(&dx) && (0..EXT).contains(&dy) && (0..EXT).contains(&dz) {
            self.blocks[(dy * EXT2 + dz * EXT + dx) as usize]
        } else {
            BlockStateId::AIR
        }
    }

    /// (sky, block) light at world position; 15/0 outside.
    pub fn light(&self, p: BlockPos) -> (u8, u8) {
        let (dx, dy, dz) = (
            p.x - self.origin.x,
            p.y - self.origin.y,
            p.z - self.origin.z,
        );
        if (0..EXT).contains(&dx) && (0..EXT).contains(&dy) && (0..EXT).contains(&dz) {
            (
                self.sky[(dy * EXT2 + dz * EXT + dx) as usize],
                self.block[(dy * EXT2 + dz * EXT + dx) as usize],
            )
        } else {
            (15, 0)
        }
    }

    #[allow(dead_code)]
    pub fn origin(&self) -> BlockPos {
        self.origin
    }
}

/// Per-section meshing accumulator. Vertices are shared; indices split by
/// render layer (Pomme's MeshSink, extended from solid/cutout to four layers).
#[derive(Default)]
struct MeshSink {
    vertices: Vec<QuadVertex>,
    /// Per-vertex packed light (nibbles, AO, shade, tint), parallel to `vertices`.
    lights: Vec<[u8; 4]>,
    /// Per-vertex atlas sprite index, parallel to `vertices`.
    sprites: Vec<u16>,
    /// Layer -> index list. RenderLayer order gives draw order.
    layers: [Vec<u32>; 4],
    /// Minimum extent seen, for the AABB.
    min: [f32; 3],
    max: [f32; 3],
}

fn layer_index(layer: RenderLayer) -> usize {
    match layer {
        RenderLayer::Solid => 0,
        RenderLayer::Cutout => 1,
        RenderLayer::CutoutMipped => 2,
        RenderLayer::Translucent => 3,
    }
}

impl MeshSink {
    fn note_position(&mut self, pos: [f32; 3]) {
        for (a, &p) in pos.iter().enumerate() {
            self.min[a] = self.min[a].min(p);
            self.max[a] = self.max[a].max(p);
        }
    }

    fn push_quad(
        &mut self,
        layer: RenderLayer,
        positions: &[[f32; 3]; 4],
        uvs: &[[f32; 2]; 4],
        lights: [[u8; 4]; 4],
        sprite: u16,
    ) {
        let base = self.vertices.len() as u32;
        for i in 0..4 {
            self.vertices.push(QuadVertex {
                position: positions[i],
                uv: uvs[i],
            });
            self.lights.push(lights[i]);
            self.sprites.push(sprite);
            self.note_position(positions[i]);
        }
        // Flip the diagonal when the default one cuts a bright/dark AO corner
        // (Pomme's emit_face_into anisotropy fix): if 0+2 sums brighter than
        // 1+3, route the shared edge through 1-3.
        let ao = |v: usize| (lights[v][1] & 3) as u32;
        let indices = &mut self.layers[layer_index(layer)];
        if ao(0) + ao(2) > ao(1) + ao(3) {
            indices.extend_from_slice(&[base + 1, base + 2, base + 3, base + 3, base, base + 1]);
        } else {
            indices.extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 3, base]);
        }
    }
}

/// Face shade index for packing: 0..5 = down/up/north/south/west/east,
/// 6 = unshaded (shade: false elements).
fn shade_index(dir: Option<haste_assets::Direction>) -> u8 {
    match dir {
        Some(d) => d.index() as u8,
        None => 6,
    }
}

/// Per-vertex (sky, block) light: the face-adjacent cell's light plus the three
/// AO-neighbour cells, averaged (vanilla smooth lighting). Packed to nibbles.
fn vertex_light(snapshot: &MeshSnapshot, base: BlockPos, neighbors: &[[i32; 3]; 3]) -> u8 {
    let (cs, cb) = snapshot.light(base);
    let mut s = cs as u32;
    let mut b = cb as u32;
    for d in neighbors {
        let (ns, nb) = snapshot.light(base.add(d[0], d[1], d[2]));
        s += ns as u32;
        b += nb as u32;
    }
    let sky = ((s + 2) / 4).min(15) as u8;
    let block = ((b + 2) / 4).min(15) as u8;
    pack_light(sky, block)
}

/// Vanilla ambient occlusion level (Pomme block_ao::vertex_ao_level):
/// 0 (three occluders) darkest .. 3 unoccluded.
fn vertex_ao_level(side1: bool, side2: bool, corner: bool) -> u8 {
    3 - (side1 as u8 + side2 as u8 + corner as u8)
}

/// Mesh one section: per block, emit its baked quads with cullface culling,
/// AO, smooth light, tint and layer routing.
pub fn mesh_section(
    snapshot: &MeshSnapshot,
    baked: &haste_assets::BakedBlockModels,
    atlas: &dyn haste_assets::AtlasRegions,
    pos: SectionPos,
) -> SectionMesh {
    let origin = pos.block_min();
    let mut sink = MeshSink {
        min: [f32::MAX; 3],
        max: [f32::MIN; 3],
        ..MeshSink::default()
    };

    for ly in 0..16i32 {
        for lz in 0..16i32 {
            for lx in 0..16i32 {
                let block_pos = origin.add(lx, ly, lz);
                let state = snapshot.block_state(block_pos);
                if state == BlockStateId::AIR {
                    continue;
                }
                let quads = baked.quads(state);
                if quads.is_empty() {
                    continue;
                }
                let layer = baked.render_layer(state);
                let offset = [lx as f32, ly as f32, lz as f32];

                for quad in quads {
                    // Cullface: skip the face when the neighbour is a full
                    // opaque block (vanilla's model culling).
                    if let Some(cull) = quad.cullface {
                        let o = cull.offset();
                        let neighbor = snapshot.block_state(block_pos.add(o[0], o[1], o[2]));
                        if baked.is_full_opaque(neighbor) {
                            continue;
                        }
                    }
                    let region = atlas.region_of(&quad.texture);
                    let sprite = region.sprite as u16;
                    // Baked UVs are atlas texels; the vertex format wants sprite-local 0..1.
                    let [rx, ry, rw, rh] = region.pixel_rect.map(|v| v as f32);
                    let uvs = quad
                        .uvs
                        .map(|t| [(t[0] - rx) / rw.max(1.0), (t[1] - ry) / rh.max(1.0)]);

                    let positions: [[f32; 3]; 4] = quad
                        .positions
                        .map(|p| [p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]]);

                    // Per-vertex light: face-adjacent cell + the vertex's AO
                    // neighbours, averaged (smooth lighting).
                    let lights: [[u8; 4]; 4] = core::array::from_fn(|vi| {
                        let (ao_level, light) =
                            vertex_light_for(snapshot, baked, block_pos, quad.direction, vi);
                        let shade = shade_index(quad.shade_face);
                        [light, pack_ao_shade(ao_level, shade), TINT_WHITE, 0]
                    });

                    sink.push_quad(layer, &positions, &uvs, lights, sprite);
                }
            }
        }
    }

    // Compact into per-layer (vertices, indices).
    let layers: Vec<(RenderLayer, Vec<u32>)> = sink
        .layers
        .into_iter()
        .enumerate()
        .filter(|(_, idx)| !idx.is_empty())
        .map(|(i, idx)| (layer_from_index(i), idx))
        .collect();

    SectionMesh {
        pos,
        vertices: sink
            .vertices
            .iter()
            .zip(sink.lights.iter().copied())
            .zip(sink.sprites.iter().copied())
            .map(|((v, light), sprite)| pack_vertex(v, sprite, light))
            .collect(),
        layers,
        aabb: (sink.min, sink.max),
        visibility: crate::occlusion::section_visibility(snapshot, baked, pos),
    }
}

fn layer_from_index(i: usize) -> RenderLayer {
    match i {
        0 => RenderLayer::Solid,
        1 => RenderLayer::Cutout,
        2 => RenderLayer::CutoutMipped,
        _ => RenderLayer::Translucent,
    }
}

/// Per-vertex AO level + packed light for face vertex `vi` (Pomme
/// compute_face_ao, inlined with the per-face neighbour table).
fn vertex_light_for(
    snapshot: &MeshSnapshot,
    baked: &haste_assets::BakedBlockModels,
    block_pos: BlockPos,
    dir: haste_assets::Direction,
    vi: usize,
) -> (u8, u8) {
    // (side1, side2, corner) offsets per vertex, in face_positions order.
    let (u_axis, v_axis) = face_axes(dir);
    let corners = face_corner_offsets(dir, u_axis, v_axis);
    let [s1, s2, corner] = corners[vi];

    let opaque = |d: [i32; 3]| -> bool {
        baked.is_full_opaque(snapshot.block_state(block_pos.add(d[0], d[1], d[2])))
    };
    let ao = vertex_ao_level(opaque(s1), opaque(s2), opaque(corner));
    let light = vertex_light(
        snapshot,
        block_pos.add(dir.offset()[0], dir.offset()[1], dir.offset()[2]),
        &[s1, s2, corner],
    );
    (ao, light)
}

/// The two in-plane axes of a face (u, v), each a unit offset.
fn face_axes(dir: haste_assets::Direction) -> ([i32; 3], [i32; 3]) {
    use haste_assets::Direction as D;
    match dir {
        D::Down | D::Up => ([1, 0, 0], [0, 0, 1]),
        D::North | D::South => ([1, 0, 0], [0, 1, 0]),
        D::West | D::East => ([0, 0, 1], [0, 1, 0]),
    }
}

/// The four (side1, side2, corner) neighbour offsets of a face's vertices, in
/// face_positions vertex order (vanilla AdjacencyInfo).
fn face_corner_offsets(
    dir: haste_assets::Direction,
    u: [i32; 3],
    v: [i32; 3],
) -> [[[i32; 3]; 3]; 4] {
    use haste_assets::Direction as D;
    // Vertex order per face_positions: which corner of the u-v plane each
    // vertex sits at.
    let corner_signs: [[i32; 2]; 4] = match dir {
        D::Down => [[0, 1], [0, 0], [1, 0], [1, 1]],
        D::Up => [[0, 0], [0, 1], [1, 1], [1, 0]],
        D::North => [[1, 1], [1, 0], [0, 0], [0, 1]],
        D::South => [[0, 1], [0, 0], [1, 0], [1, 1]],
        D::West => [[0, 1], [0, 0], [1, 0], [1, 1]],
        D::East => [[1, 1], [1, 0], [0, 0], [0, 1]],
    };
    let n = dir.offset();
    corner_signs.map(|[su, sv]| {
        // 0 -> -1, 1 -> +1: the side neighbour toward this vertex along each axis.
        let (fu, fv) = (su * 2 - 1, sv * 2 - 1);
        [
            [n[0] + fu * u[0], n[1] + fu * u[1], n[2] + fu * u[2]],
            [n[0] + fv * v[0], n[1] + fv * v[1], n[2] + fv * v[2]],
            [
                n[0] + fu * u[0] + fv * v[0],
                n[1] + fu * u[1] + fv * v[1],
                n[2] + fu * u[2] + fv * v[2],
            ],
        ]
    })
}

/// The worker pool: meshes dirty sections off the render thread. Results come
/// back through a bounded channel (back-pressure), drained per frame with a
/// budget (Pomme MeshDispatcher + drain_results, on rayon instead of raw
/// threads).
pub struct Mesher {
    baked: Arc<haste_assets::BakedBlockModels>,
    atlas: Arc<dyn haste_assets::AtlasRegions + Send + Sync>,
    pending: Mutex<Vec<(SectionPos, Arc<MeshSnapshot>)>>,
    results: crossbeam_channel::Receiver<SectionMesh>,
    result_tx: crossbeam_channel::Sender<SectionMesh>,
}

impl Mesher {
    pub fn new(
        baked: Arc<haste_assets::BakedBlockModels>,
        atlas: Arc<dyn haste_assets::AtlasRegions + Send + Sync>,
    ) -> Mesher {
        let (tx, rx) = crossbeam_channel::bounded(256);
        Mesher {
            baked,
            atlas,
            pending: Mutex::new(Vec::new()),
            results: rx,
            result_tx: tx,
        }
    }

    /// Submit a section for meshing. The snapshot is captured on the caller's
    /// thread (world data is not Send across threads); deduplicates pending
    /// submissions for the same section.
    pub fn submit(&self, view: &dyn BlockView, pos: SectionPos) {
        {
            let mut pending = self.pending.lock();
            if pending.iter().any(|(p, _)| *p == pos) {
                return;
            }
            pending.push((pos, Arc::new(MeshSnapshot::capture(view, pos))));
        }
        self.kick();
    }

    /// Mesh all pending sections on the rayon pool. Called from the render
    /// loop; rayon's work-stealing keeps the render thread responsive because
    /// this call only *schedules* the work (spawn) and returns.
    fn kick(&self) {
        let jobs: Vec<(SectionPos, Arc<MeshSnapshot>)> = self.pending.lock().drain(..).collect();
        if jobs.is_empty() {
            return;
        }
        let baked = self.baked.clone();
        let atlas = self.atlas.clone();
        let tx = self.result_tx.clone();
        rayon::spawn(move || {
            jobs.into_par_iter()
                .map(|(pos, snapshot)| mesh_section(&snapshot, &baked, &*atlas, pos))
                .for_each(|mesh| {
                    let _ = tx.send(mesh);
                });
        });
    }

    /// Drain finished meshes, up to `budget` per call.
    pub fn drain(&self, budget: usize) -> Vec<SectionMesh> {
        let mut out = Vec::with_capacity(budget.min(32));
        while out.len() < budget {
            match self.results.try_recv() {
                Ok(m) => out.push(m),
                Err(_) => break,
            }
        }
        out
    }

    pub fn pending_len(&self) -> usize {
        self.pending.lock().len() + self.results.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic flat world: one layer of `block` over bedrock, for the
    /// end-to-end mesh tests (real jar, ignored without cache).
    struct FlatWorld {
        ground: BlockStateId,
        /// Layers y = 0..layers.
        layers: i32,
    }

    impl haste_core::view::BlockView for FlatWorld {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            // A finite 16x16 platform (one section footprint), not an infinite
            // plane: side faces of the edge blocks must survive culling.
            if (0..self.layers).contains(&pos.y)
                && (0..16).contains(&pos.x)
                && (0..16).contains(&pos.z)
            {
                self.ground
            } else {
                BlockStateId::AIR
            }
        }
        fn is_chunk_loaded(&self, _pos: haste_core::ChunkPos) -> bool {
            true
        }
        fn sky_light(&self, pos: BlockPos) -> u8 {
            if pos.y <= 0 {
                0
            } else {
                15
            }
        }
        fn block_light(&self, _pos: BlockPos) -> u8 {
            0
        }
        fn min_y(&self) -> i32 {
            -64
        }
        fn height(&self) -> i32 {
            384
        }
    }

    #[test]
    #[ignore = "needs the client jar and datagen reports in the cache"]
    fn jar_mesh_flat_stone_section() {
        let jar = haste_core::paths::client_jar();
        if !jar.exists() {
            eprintln!("ignored: needs the client jar in the cache");
            return;
        }
        let mut assets = haste_assets::Assets::open(
            &jar,
            &haste_core::paths::cache_dir().join("assets"),
            "26.3",
        )
        .unwrap();
        let states = haste_assets::BlocksJsonStates::load(
            &haste_core::paths::datagen_reports().join("blocks.json"),
        )
        .unwrap();
        let mut atlas = haste_assets::AtlasBuilder::build(&mut assets, &[], false).unwrap();
        let baked = haste_assets::BakedBlockModels::bake(&mut assets, &states, &mut atlas).unwrap();

        // stone state id 1.
        let world = FlatWorld {
            ground: BlockStateId(1),
            layers: 1,
        };
        let snapshot = MeshSnapshot::capture(&world, SectionPos::new(0, 0, 0));
        let mesh = mesh_section(&snapshot, &baked, &atlas, SectionPos::new(0, 0, 0));

        // One layer of stone: only the top faces of y=0 survive (sides culled
        // between neighbours, bottom at world bottom is against air but y=-1
        // is air so the bottom face shows too => 16*16 top + 16*16 bottom
        // + 4 edges * 16 = quads).
        let total_quads: usize = mesh.layers.iter().map(|(_, idx)| idx.len() / 6).sum();
        eprintln!("total quads: {total_quads}");
        // Histogram by cullface direction to debug face emission.
        use std::collections::HashMap;
        let mut hist: HashMap<Option<haste_assets::Direction>, usize> = HashMap::new();
        for (sid, quads) in [(BlockStateId(1), baked.quads(BlockStateId(1)))] {
            let _ = sid;
            for q in quads {
                *hist.entry(q.cullface).or_default() += 1;
            }
        }
        eprintln!("stone baked quads by cullface: {hist:?}");
        // All in the solid layer.
        assert_eq!(mesh.layers.len(), 1);
        assert_eq!(mesh.layers[0].0, haste_assets::RenderLayer::Solid);
        // Vertices packed, sprite ids resolved from the atlas.
        assert!(mesh.vertices.iter().all(|v| v.sprite != 0));
        // UVs are sprite-local: full faces span 0..1 (4095 in the fixed-point format).
        assert!(mesh
            .vertices
            .iter()
            .all(|v| v.uv[0] <= 4095 && v.uv[1] <= 4095));
        assert!(
            mesh.vertices.iter().any(|v| v.uv[0] == 0)
                && mesh.vertices.iter().any(|v| v.uv[0] == 4095)
        );
        // AABB spans the section.
        assert_eq!(mesh.aabb.0, [0.0, 0.0, 0.0]);
        assert_eq!(mesh.aabb.1, [16.0, 1.0, 16.0]);

        // A 16x16x4 grass slab: faces between grass blocks are culled. Visible: 256 top,
        // 256 bottom, 4 * 16 * 4 sides, plus the tinted overlay on each side face.
        use haste_core::view::BlockStateTable;
        let grass = (0..states.len() as u32)
            .map(BlockStateId)
            .find(|s| {
                states.block_of(*s).path() == "grass_block"
                    && states.property_value(*s, "snowy") == Some("false")
            })
            .unwrap();
        let world = FlatWorld {
            ground: grass,
            layers: 4,
        };
        let snapshot = MeshSnapshot::capture(&world, SectionPos::new(0, 0, 0));
        let mesh = mesh_section(&snapshot, &baked, &atlas, SectionPos::new(0, 0, 0));
        let quads: usize = mesh.layers.iter().map(|(_, idx)| idx.len() / 6).sum();
        eprintln!(
            "grass slab: {quads} quads, {} vertices",
            mesh.vertices.len()
        );
        assert_eq!(quads, 256 + 256 + 256 + 256, "culled grass slab");
    }

    #[test]
    fn ao_level_counts_occluders() {
        assert_eq!(vertex_ao_level(false, false, false), 3);
        assert_eq!(vertex_ao_level(true, false, false), 2);
        assert_eq!(vertex_ao_level(true, true, false), 1);
        assert_eq!(vertex_ao_level(true, true, true), 0);
    }

    /// Pomme's `compute_face_ao` neighbour triples (vanilla AdjacencyInfo),
    /// per face, in face_positions vertex order. side1/side2 order is
    /// irrelevant to the AO level (symmetric), but the corner must match.
    fn pomme_ao_rows(dir: haste_assets::Direction) -> [[[i32; 3]; 3]; 4] {
        use haste_assets::Direction as D;
        match dir {
            D::Up => [
                [[0, 1, -1], [-1, 1, 0], [-1, 1, -1]],
                [[0, 1, 1], [-1, 1, 0], [-1, 1, 1]],
                [[0, 1, 1], [1, 1, 0], [1, 1, 1]],
                [[0, 1, -1], [1, 1, 0], [1, 1, -1]],
            ],
            D::Down => [
                [[0, -1, 1], [-1, -1, 0], [-1, -1, 1]],
                [[0, -1, -1], [-1, -1, 0], [-1, -1, -1]],
                [[0, -1, -1], [1, -1, 0], [1, -1, -1]],
                [[0, -1, 1], [1, -1, 0], [1, -1, 1]],
            ],
            D::North => [
                [[1, 0, -1], [0, 1, -1], [1, 1, -1]],
                [[1, 0, -1], [0, -1, -1], [1, -1, -1]],
                [[-1, 0, -1], [0, -1, -1], [-1, -1, -1]],
                [[-1, 0, -1], [0, 1, -1], [-1, 1, -1]],
            ],
            D::South => [
                [[-1, 0, 1], [0, 1, 1], [-1, 1, 1]],
                [[-1, 0, 1], [0, -1, 1], [-1, -1, 1]],
                [[1, 0, 1], [0, -1, 1], [1, -1, 1]],
                [[1, 0, 1], [0, 1, 1], [1, 1, 1]],
            ],
            D::West => [
                [[-1, 0, -1], [-1, 1, 0], [-1, 1, -1]],
                [[-1, 0, -1], [-1, -1, 0], [-1, -1, -1]],
                [[-1, 0, 1], [-1, -1, 0], [-1, -1, 1]],
                [[-1, 0, 1], [-1, 1, 0], [-1, 1, 1]],
            ],
            D::East => [
                [[1, 0, 1], [1, 1, 0], [1, 1, 1]],
                [[1, 0, 1], [1, -1, 0], [1, -1, 1]],
                [[1, 0, -1], [1, -1, 0], [1, -1, -1]],
                [[1, 0, -1], [1, 1, 0], [1, 1, -1]],
            ],
        }
    }

    #[test]
    fn face_corner_offsets_match_vanilla_adjacency() {
        use haste_assets::Direction as D;
        for dir in D::ALL {
            let (u, v) = face_axes(dir);
            let mine = face_corner_offsets(dir, u, v);
            let reference = pomme_ao_rows(dir);
            for vi in 0..4 {
                // Corner offset must match exactly (it decides the AO diagonal).
                assert_eq!(
                    mine[vi][2], reference[vi][2],
                    "{dir:?} vertex {vi} corner mismatch"
                );
                // Side offsets must be the same pair (order may differ: both
                // feed the symmetric vertex_ao_level).
                let mut mine_sides = [mine[vi][0], mine[vi][1]];
                let mut ref_sides = [reference[vi][0], reference[vi][1]];
                mine_sides.sort();
                ref_sides.sort();
                assert_eq!(mine_sides, ref_sides, "{dir:?} vertex {vi} side mismatch");
            }
        }
    }
}
