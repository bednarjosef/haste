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

use crate::vertex::{pack_ao_shade, pack_light, pack_vertex, PackedVertex};

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

impl SectionMesh {
    /// GPU bytes the vertices will occupy in an arena.
    pub fn vertex_bytes(&self) -> u64 {
        (self.vertices.len() as u64) * std::mem::size_of::<PackedVertex>() as u64
    }

    /// GPU bytes the indices will occupy in an arena.
    pub fn index_bytes(&self) -> u64 {
        self.layers.iter().map(|(_, i)| i.len() as u64 * 4).sum()
    }
}

/// Untinted vertex colour.
pub const TINT_WHITE: u32 = 0xFFFFFF;

/// The per-face AO neighbour offsets (see `face_corner_table`), built once.
static FACE_CORNERS: std::sync::OnceLock<[[[[i32; 3]; 3]; 4]; 6]> = std::sync::OnceLock::new();

fn face_corners() -> &'static [[[[i32; 3]; 3]; 4]; 6] {
    FACE_CORNERS.get_or_init(face_corner_table)
}

/// The per-face side-0 cell offsets (see `face_side0_table`), built once.
static FACE_SIDE0: std::sync::OnceLock<[[i32; 3]; 6]> = std::sync::OnceLock::new();

fn face_side0() -> &'static [[i32; 3]; 6] {
    FACE_SIDE0.get_or_init(face_side0_table)
}

/// Per-state ambient-occlusion occluder flag (vanilla `getShadeBrightness <=
/// 0.2`): whether the state's collision shape is a full cube, minus the glass
/// family whose class overrides the brightness to 1.0. Built from
/// `haste_data::BlockShapes`; the fallback treats every state as a full cube
/// except air and fluids (approximate like all fallback shapes).
#[derive(Clone)]
pub struct AoTable {
    solid: Vec<bool>,
}

impl AoTable {
    /// From haste-data's shape data (oracle-backed when the dump exists).
    pub fn from_shapes(
        states: &dyn haste_core::view::BlockStateTable,
        shapes: &haste_data::BlockShapes,
    ) -> AoTable {
        let mut solid = Vec::with_capacity(states.len());
        for id in 0..states.len() as u32 {
            solid.push(shapes.is_ao_solid(BlockStateId(id)));
        }
        AoTable { solid }
    }

    /// Fallback with no shape data (tests, before the game data is known):
    /// every in-range state occludes (stone-like), ids past the table read as
    /// non-occluding.
    pub fn all_solid(n: usize) -> AoTable {
        AoTable {
            solid: vec![true; n],
        }
    }

    /// Whether `state` darkens its neighbours' AO. Out-of-table ids (modded
    /// servers) read as non-occluding rather than panicking; the placeholder
    /// resolution the other tables do needs the state table, which tests
    /// without one cannot provide, and an unknown block that renders should
    /// not darken either (vanilla resolves it to stone, but the difference is
    /// one AO corner on an unseen block).
    pub fn is_solid(&self, state: BlockStateId) -> bool {
        self.solid.get(state.0 as usize).copied().unwrap_or(false)
    }
}

/// Per-block-state and per-biome tables the mesher reads (shared with the workers).
#[derive(Clone)]
pub struct MeshTables {
    pub tints: Arc<crate::tint::TintTable>,
    pub colors: Arc<crate::tint::BiomeColors>,
    pub fluids: Arc<crate::fluid::FluidTable>,
    /// AO occluder test per state (vanilla collision-shape rule).
    pub ao: Arc<AoTable>,
    /// Set identity for the mesher's per-state cache: a fresh value per
    /// constructed set (monotonic counter), so a new Renderer's tables
    /// never alias a previous set's pointer identities after a reconnect.
    pub generation: u64,
}

/// Monotonic source of `MeshTables::generation`: every construction takes
/// the next value, so two sets are never the same cache generation even when
/// the allocator reuses a freed set's addresses.
static TABLE_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl MeshTables {
    pub fn new(
        tints: Arc<crate::tint::TintTable>,
        colors: Arc<crate::tint::BiomeColors>,
        fluids: Arc<crate::fluid::FluidTable>,
        ao: Arc<AoTable>,
    ) -> MeshTables {
        MeshTables {
            tints,
            colors,
            fluids,
            ao,
            generation: TABLE_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }
}

impl Default for MeshTables {
    fn default() -> Self {
        MeshTables::new(
            Arc::new(crate::tint::TintTable::empty()),
            Arc::new(crate::tint::BiomeColors::default()),
            Arc::new(crate::fluid::FluidTable::empty()),
            // No shape data known: no state occludes AO (flat lighting).
            Arc::new(AoTable::all_solid(0)),
        )
    }
}

/// A snapshot of the 3x3x3 section neighbourhood, captured on the render/game
/// thread and sent to the mesh workers. Cheap random access, no locks.
pub struct MeshSnapshot {
    /// Interleaved (state, sky, block) per cell, offsets -1..17 relative to the
    /// section origin: the section plus one-block borders into all neighbours
    /// (blocks for face culling and AO). Y is clamped at the world bounds (air
    /// outside). One array so a cell lookup touches one cache line.
    cells: Vec<Cell>,
    /// Biome cells around the section (tint blending).
    biomes: crate::tint::BiomeGrid,
    /// Section origin in world blocks.
    origin: BlockPos,
    /// World bounds for y clamping (kept for debug/documentation).
    #[allow(dead_code)]
    min_y: i32,
    #[allow(dead_code)]
    height: i32,
}

/// One captured cell: block state plus its two light nibbles.
#[derive(Clone, Copy)]
struct Cell {
    state: BlockStateId,
    /// (sky << 4) | block, pre-packed like the vertex format's light byte.
    light: u8,
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
        Self::capture_with_biomes(view, None, 0, pos)
    }

    /// `capture` plus the biome cells within `blend_radius` blocks of the section (for
    /// tints); without a biome source every block is biome 0.
    pub fn capture_with_biomes(
        view: &dyn BlockView,
        biome_source: Option<&dyn crate::tint::BiomeSource>,
        blend_radius: i32,
        pos: SectionPos,
    ) -> MeshSnapshot {
        let biomes = match biome_source {
            Some(src) => crate::tint::BiomeGrid::capture(src, pos.block_min(), blend_radius),
            None => crate::tint::BiomeGrid::uniform(haste_core::BiomeId(0)),
        };
        let origin = pos.block_min().add(-1, -1, -1);
        let mut cells = Vec::with_capacity(EXT3);
        for dy in 0..EXT {
            let y = origin.y + dy;
            let in_bounds = y >= view.min_y() && y < view.min_y() + view.height();
            for dz in 0..EXT {
                for dx in 0..EXT {
                    if in_bounds {
                        let p = BlockPos::new(origin.x + dx, y, origin.z + dz);
                        cells.push(Cell {
                            state: view.block_state(p),
                            light: pack_light(view.sky_light(p), view.block_light(p)),
                        });
                    } else {
                        cells.push(Cell {
                            state: BlockStateId::AIR,
                            light: pack_light(15, 0),
                        });
                    }
                }
            }
        }
        MeshSnapshot {
            cells,
            biomes,
            origin,
            min_y: view.min_y(),
            height: view.height(),
        }
    }

    #[inline]
    fn cell(&self, dx: i32, dy: i32, dz: i32) -> Cell {
        debug_assert!((0..EXT).contains(&dx) && (0..EXT).contains(&dy) && (0..EXT).contains(&dz));
        self.cells[(dy * EXT2 + dz * EXT + dx) as usize]
    }

    /// Block state at world position; out-of-bounds y reads as air.
    pub fn block_state(&self, p: BlockPos) -> BlockStateId {
        let (dx, dy, dz) = (
            p.x - self.origin.x,
            p.y - self.origin.y,
            p.z - self.origin.z,
        );
        if (0..EXT).contains(&dx) && (0..EXT).contains(&dy) && (0..EXT).contains(&dz) {
            self.cell(dx, dy, dz).state
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
            let light = self.cell(dx, dy, dz).light;
            (light >> 4, light & 0xF)
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
/// Vertices are packed directly (the intermediate QuadVertex/light/tint/sprite
/// columns of the original are folded into one `PackedVertex` push).
struct MeshSink {
    vertices: Vec<PackedVertex>,
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
    fn new() -> MeshSink {
        MeshSink {
            vertices: Vec::new(),
            layers: Default::default(),
            min: [f32::MAX; 3],
            max: [f32::MIN; 3],
        }
    }

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
        lights: [[u8; 3]; 4],
        tint: u32,
        sprite: u16,
    ) {
        let base = self.vertices.len() as u32;
        for i in 0..4 {
            self.vertices.push(pack_vertex_raw(
                positions[i],
                uvs[i],
                lights[i],
                tint,
                sprite,
            ));
            self.note_position(positions[i]);
        }
        // Flip the diagonal when the default one cuts a bright/dark AO corner
        // (Pomme's emit_face_into anisotropy fix): if 0+2 sums brighter than
        // 1+3, route the shared edge through 1-3. The AO byte holds the
        // occluder count, so brighter is the smaller sum.
        let ao = |v: usize| (lights[v][1] & 7) as u32;
        let indices = &mut self.layers[layer_index(layer)];
        if ao(0) + ao(2) < ao(1) + ao(3) {
            indices.extend_from_slice(&[base + 1, base + 2, base + 3, base + 3, base, base + 1]);
        } else {
            indices.extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 3, base]);
        }
    }
}

/// `vertex::pack_vertex` on plain values (no QuadVertex indirection): identical math.
fn pack_vertex_raw(
    position: [f32; 3],
    uv: [f32; 2],
    light: [u8; 3],
    tint: u32,
    sprite: u16,
) -> PackedVertex {
    pack_vertex(
        &crate::vertex::QuadVertex { position, uv },
        sprite,
        light,
        tint,
    )
}

/// Face shade index for packing: 0..5 = down/up/north/south/west/east,
/// 6 = unshaded (shade: false elements).
fn shade_index(dir: Option<haste_assets::Direction>) -> u8 {
    match dir {
        Some(d) => d.index() as u8,
        None => 6,
    }
}

/// Vanilla ambient occlusion brightness as an occluder count (vanilla
/// `BlockModelLighter.prepareQuadAmbientOcclusion`): the corner's brightness
/// is the average of four shade samples — the two sides, the diagonal and the
/// face-adjacent cell — each 1.0 or 0.2 (`getShadeBrightness`), so brightness
/// = 1.0 - 0.2 * count, count 0..4. The diagonal is the caller's to resolve:
/// vanilla reads side 0's sample when both of the vertex's side cells are
/// non-light-permeable.
fn vertex_ao_level(side1: bool, side2: bool, diagonal: bool, center: bool) -> u8 {
    side1 as u8 + side2 as u8 + diagonal as u8 + center as u8
}

/// Mesh one section: per block, emit its baked quads with cullface culling,
/// AO, smooth light, tint and layer routing.
pub fn mesh_section(
    snapshot: &MeshSnapshot,
    baked: &haste_assets::BakedBlockModels,
    atlas: &dyn haste_assets::AtlasRegions,
    pos: SectionPos,
) -> SectionMesh {
    mesh_section_with(snapshot, baked, atlas, &MeshTables::default(), pos)
}

/// The blended tint of `kind` at block `p` from the snapshot's biome cells.
fn block_tint(
    snapshot: &MeshSnapshot,
    tables: &MeshTables,
    kind: crate::tint::TintKind,
    p: BlockPos,
) -> u32 {
    tables
        .colors
        .blended(kind, p.x, p.z, |x, z| snapshot.biomes.get(x, p.y, z))
}

/// A baked quad reduced to what the mesh loop needs, computed once per block
/// state (atlas lookup, sprite-local UVs, cull offset, packed shade) instead of
/// per block: the same block state appears thousands of times per world.
struct EmitQuad {
    /// Sprite-local UVs (baked atlas UVs mapped into the sprite's 0..1 space).
    uvs: [[f32; 2]; 4],
    /// Model-space positions (0..1 block units), translated per block.
    positions: [[f32; 3]; 4],
    /// Cullface direction (None = always drawn).
    cullface: Option<haste_assets::Direction>,
    /// Face used for cardinal shading / AO.
    shade_face: Option<haste_assets::Direction>,
    /// Face direction (light sampling and AO neighbours).
    direction: haste_assets::Direction,
    /// Whether the quad takes the block's tint.
    tinted: bool,
    sprite: u16,
}

/// Per-state meshing info, built lazily per section on first sight of a state.
/// (The state population is tiny compared to the block population.)
struct StateInfo {
    quads: Vec<EmitQuad>,
    /// Render layer of the state's quads.
    layer: RenderLayer,
    /// The state's fluid, if any.
    fluid: Option<crate::fluid::FluidState>,
}

/// The per-section meshing context: snapshot access by local offset plus the
/// lazily built per-state info. The state cache and the fluid scratch are
/// borrowed from the per-thread scratch (see `MeshScratch`).
struct MeshContext<'a> {
    snapshot: &'a MeshSnapshot,
    baked: &'a haste_assets::BakedBlockModels,
    atlas: &'a dyn haste_assets::AtlasRegions,
    tables: &'a MeshTables,
    /// Section origin in world blocks (fluid and tint lookups want world positions).
    origin: BlockPos,
    /// Lazily built per-state info, indexed by state id (grown on demand, None
    /// until first seen).
    states: &'a mut Vec<Option<std::rc::Rc<StateInfo>>>,
    /// Reusable scratch for the fluid mesher (no per-block allocation).
    fluid_scratch: &'a mut Vec<crate::fluid::FluidQuad>,
}

// Per-thread meshing scratch: the sink's output vectors, the per-state cache
// and the fluid mesher's quad buffer, kept (cleared, not freed) between
// sections so meshing a section allocates only the vectors handed to the
// `SectionMesh` as its final output. One per thread: rayon workers each mesh
// on their own thread, and `mesh_section_with` is never reentrant.
thread_local! {
    static SCRATCH: std::cell::RefCell<MeshScratch> = std::cell::RefCell::new(MeshScratch::new());
}

struct MeshScratch {
    sink: MeshSink,
    states: Vec<Option<std::rc::Rc<StateInfo>>>,
    fluid_scratch: Vec<crate::fluid::FluidQuad>,
    /// The (baked, atlas, fluids) set `states` was built from; `None` before
    /// the first mesh. The cache is only valid for one set: a second Renderer
    /// in the same process (reconnect, a new world) reuses the mesh thread,
    /// so the cache is cleared whenever the identity changes.
    state_key: Option<SourceKey>,
}

/// Identity of the inputs the state cache is derived from: the baked models,
/// the atlas (its data pointer; UVs/sprite ids come from it), the fluid
/// table and the table set's generation. The pointers say which set it is
/// when addresses are unique; the generation catches a new set allocated at
/// a freed set's addresses (ABA across a reconnect).
#[derive(Clone, Copy, PartialEq, Eq)]
struct SourceKey {
    baked: usize,
    atlas: usize,
    fluids: usize,
    generation: u64,
}

impl MeshScratch {
    fn new() -> MeshScratch {
        MeshScratch {
            sink: MeshSink::new(),
            states: Vec::new(),
            fluid_scratch: Vec::with_capacity(11),
            state_key: None,
        }
    }

    /// Reset the per-section scratch. The state cache is deliberately NOT
    /// cleared: it is a pure function of the immutable baked models, atlas and
    /// tables, so it persists across sections (clearing it would redo the
    /// atlas lookups and re-allocate every state's quad vector per section).
    fn begin_section(&mut self) {
        self.sink.vertices.clear();
        for idx in &mut self.sink.layers {
            idx.clear();
        }
        self.sink.min = [f32::MAX; 3];
        self.sink.max = [f32::MIN; 3];
        self.fluid_scratch.clear();
    }
}

impl<'a> MeshContext<'a> {
    /// The `StateInfo` for `state`, building it on first sight (one build per
    /// distinct state per section; terrain repeats a state thousands of times).
    fn state_info(&mut self, state: BlockStateId) -> std::rc::Rc<StateInfo> {
        let idx = state.0 as usize;
        if idx >= self.states.len() {
            self.states.resize(idx + 1, None);
        }
        if self.states[idx].is_none() {
            let quads: Vec<EmitQuad> = self
                .baked
                .quads(state)
                .iter()
                .map(|quad| {
                    let region = self.atlas.region_of(&quad.texture);
                    let sprite = region.sprite as u16;
                    // Baked UVs are atlas texels; the vertex format wants sprite-local 0..1.
                    let [rx, ry, rw, rh] = region.pixel_rect.map(|v| v as f32);
                    let uvs = quad
                        .uvs
                        .map(|t| [(t[0] - rx) / rw.max(1.0), (t[1] - ry) / rh.max(1.0)]);
                    EmitQuad {
                        uvs,
                        positions: quad.positions,
                        cullface: quad.cullface,
                        shade_face: quad.shade_face,
                        direction: quad.direction,
                        tinted: quad.tint_index.is_some(),
                        sprite,
                    }
                })
                .collect();
            self.states[idx] = Some(std::rc::Rc::new(StateInfo {
                quads,
                layer: self.baked.render_layer(state),
                fluid: self.tables.fluids.fluid(state),
            }));
        }
        self.states[idx].as_ref().unwrap().clone()
    }

    /// The snapshot cell at section-local coords (block -1..17; outside reads
    /// as lit air, matching `MeshSnapshot::block_state`'s out-of-range answer).
    #[inline]
    fn cell(&self, x: i32, y: i32, z: i32) -> Cell {
        if (0..EXT).contains(&x) && (0..EXT).contains(&y) && (0..EXT).contains(&z) {
            self.snapshot.cell(x, y, z)
        } else {
            Cell {
                state: BlockStateId::AIR,
                light: pack_light(15, 0),
            }
        }
    }
}

/// `mesh_section` with tint and fluid tables.
pub fn mesh_section_with(
    snapshot: &MeshSnapshot,
    baked: &haste_assets::BakedBlockModels,
    atlas: &dyn haste_assets::AtlasRegions,
    tables: &MeshTables,
    pos: SectionPos,
) -> SectionMesh {
    SCRATCH.with(|cell| {
        let mut scratch = cell.borrow_mut();
        // The state cache is only valid for the (baked, atlas, fluids) set it
        // was built from: a second Renderer in the same process (reconnect, a
        // new world) reuses the mesh thread, so clear it when the identity
        // changes. Pointer identity: all three outlive the meshing call.
        let key = SourceKey {
            baked: baked as *const _ as usize,
            atlas: atlas as *const _ as *const () as usize,
            fluids: std::sync::Arc::as_ptr(&tables.fluids) as usize,
            generation: tables.generation,
        };
        if scratch.state_key != Some(key) {
            scratch.states.clear();
            scratch.state_key = Some(key);
        }
        scratch.begin_section();
        let MeshScratch {
            sink,
            states,
            fluid_scratch,
            state_key: _,
        } = &mut *scratch;
        let mut ctx = MeshContext {
            snapshot,
            baked,
            atlas,
            tables,
            origin: pos.block_min(),
            states,
            fluid_scratch,
        };

        for ly in 0..16i32 {
            for lz in 0..16i32 {
                for lx in 0..16i32 {
                    let state = ctx.cell(lx + 1, ly + 1, lz + 1).state;
                    if state == BlockStateId::AIR {
                        continue;
                    }
                    let info = ctx.state_info(state);
                    if let Some(fluid) = info.fluid {
                        mesh_fluid(&mut ctx, lx, ly, lz, fluid, sink);
                    }
                    if info.quads.is_empty() {
                        continue;
                    }
                    let offset = [lx as f32, ly as f32, lz as f32];
                    // The block's tint, computed on its first tinted quad.
                    let mut tint: Option<u32> = None;

                    for quad in &info.quads {
                        // Cullface: skip the face when the neighbour is a full
                        // opaque block (vanilla's model culling).
                        if let Some(cull) = quad.cullface {
                            let o = cull.offset();
                            let neighbor = ctx.cell(lx + 1 + o[0], ly + 1 + o[1], lz + 1 + o[2]);
                            if ctx.baked.is_full_opaque(neighbor.state) {
                                continue;
                            }
                        }
                        let positions: [[f32; 3]; 4] = quad
                            .positions
                            .map(|p| [p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]]);

                        // Per-vertex light, following vanilla
                        // ModelBlockRenderer.AmbientOcclusionFace: face-adjacent
                        // cell + the vertex's AO neighbours, blended. [sky16,
                        // ao|shade, block16].
                        let lights: [[u8; 3]; 4] = core::array::from_fn(|vi| {
                            let (ao_level, [sky16, block16]) =
                                vertex_light_for(&ctx, lx, ly, lz, quad.direction, vi);
                            let shade = shade_index(quad.shade_face);
                            [sky16, pack_ao_shade(ao_level, shade), block16]
                        });
                        let quad_tint = if quad.tinted {
                            *tint.get_or_insert_with(|| {
                                block_tint(
                                    snapshot,
                                    tables,
                                    tables.tints.kind(state),
                                    ctx.origin.add(lx, ly, lz),
                                )
                            })
                        } else {
                            TINT_WHITE
                        };

                        sink.push_quad(
                            info.layer,
                            &positions,
                            &quad.uvs,
                            lights,
                            quad_tint,
                            quad.sprite,
                        );
                    }
                }
            }
        }

        // The section's final output: the sink's vectors, moved (not copied)
        // into the SectionMesh. Refill the scratch with right-sized vectors:
        // consecutive sections are similar in size, so the next section pays
        // one allocation per vector instead of growing them geometrically.
        let vertex_len = sink.vertices.len();
        let index_lens: [usize; 4] = core::array::from_fn(|i| sink.layers[i].len());
        let vertices = std::mem::take(&mut sink.vertices);
        let index_vecs = std::mem::take(&mut sink.layers);
        sink.vertices = Vec::with_capacity(vertex_len);
        sink.layers = index_lens.map(Vec::with_capacity);

        // Compact into per-layer (vertices, indices).
        let layers: Vec<(RenderLayer, Vec<u32>)> = index_vecs
            .into_iter()
            .enumerate()
            .filter(|(_, idx)| !idx.is_empty())
            .map(|(i, idx)| (layer_from_index(i), idx))
            .collect();

        SectionMesh {
            pos,
            vertices,
            layers,
            aabb: (sink.min, sink.max),
            visibility: crate::occlusion::section_visibility(snapshot, baked, pos),
        }
    })
}

/// The snapshot as the fluid mesher sees it.
struct FluidView<'a> {
    snapshot: &'a MeshSnapshot,
    baked: &'a haste_assets::BakedBlockModels,
}

impl crate::fluid::FluidWorld for FluidView<'_> {
    fn state(&self, p: BlockPos) -> BlockStateId {
        self.snapshot.block_state(p)
    }
    fn opaque(&self, p: BlockPos) -> bool {
        self.baked.is_full_opaque(self.snapshot.block_state(p))
    }
    fn solid(&self, p: BlockPos) -> bool {
        crate::fluid::baked_solid(self.baked, self.snapshot.block_state(p))
    }
}

/// Max of two packed (sky, block) light values per channel.
fn max_light(a: (u8, u8), b: (u8, u8)) -> u8 {
    pack_light(a.0.max(b.0), a.1.max(b.1))
}

/// Emits the fluid surface of one block (see `fluid.rs` for the rules). Quads
/// go through the context's reusable scratch Vec (no per-block allocation).
fn mesh_fluid(
    ctx: &mut MeshContext,
    lx: i32,
    ly: i32,
    lz: i32,
    fluid: crate::fluid::FluidState,
    sink: &mut MeshSink,
) {
    let block_pos = ctx.origin.add(lx, ly, lz);
    let mut quads = std::mem::take(&mut *ctx.fluid_scratch);
    quads.clear();
    {
        let view = FluidView {
            snapshot: ctx.snapshot,
            baked: ctx.baked,
        };
        crate::fluid::fluid_quads(&view, &ctx.tables.fluids, block_pos, fluid, &mut quads);
    }
    if !quads.is_empty() {
        let tint = match ctx.tables.fluids.defs.def(fluid.fluid).tint {
            crate::fluid::FluidTint::BiomeWater => block_tint(
                ctx.snapshot,
                ctx.tables,
                crate::tint::TintKind::Water,
                block_pos,
            ),
            crate::fluid::FluidTint::None => TINT_WHITE,
        };
        let layer = crate::fluid::fluid_layer(&ctx.tables.fluids.defs, fluid.fluid);
        let own = ctx.snapshot.light(block_pos);
        let top_light = max_light(own, ctx.snapshot.light(block_pos.add(0, 1, 0)));
        let bottom_light = max_light(own, ctx.snapshot.light(block_pos.add(0, -1, 0)));
        // Fluid light is a whole level (no blend): *16 into the smooth units
        // the chunk vertex carries.
        let light16 = |l: u8| {
            [
                (l >> 4) as u32 * crate::vertex::LIGHT16_PER_LEVEL as u32,
                (l & 0xF) as u32 * crate::vertex::LIGHT16_PER_LEVEL as u32,
            ]
        };
        let top16 = light16(top_light);
        let bottom16 = light16(bottom_light);
        let offset = [lx as f32, ly as f32, lz as f32];
        for q in &quads {
            let [sky16, block16] = if q.dir == haste_assets::Direction::Down {
                bottom16
            } else {
                top16
            };
            let packed = [
                sky16 as u8,
                pack_ao_shade(0, shade_index(Some(q.dir))),
                block16 as u8,
            ];
            let positions = q
                .positions
                .map(|p| [p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]]);
            sink.push_quad(layer, &positions, &q.uvs, [packed; 4], tint, q.sprite);
        }
    }
    *ctx.fluid_scratch = quads;
}

fn layer_from_index(i: usize) -> RenderLayer {
    match i {
        0 => RenderLayer::Solid,
        1 => RenderLayer::Cutout,
        2 => RenderLayer::CutoutMipped,
        _ => RenderLayer::Translucent,
    }
}

/// The four (side1, side2, corner) neighbour offsets of each face vertex, in
/// face_positions vertex order (vanilla AdjacencyInfo), precomputed per face
/// direction: `face_corner_offsets` is a pure function of the direction, so the
/// mesh loop reads a table instead of recomputing it per vertex.
fn face_corner_table() -> [[[[i32; 3]; 3]; 4]; 6] {
    let mut table = [[[[0i32; 3]; 3]; 4]; 6];
    for (di, dir) in haste_assets::Direction::ALL.into_iter().enumerate() {
        let (u_axis, v_axis) = face_axes(dir);
        table[di] = face_corner_offsets(dir, u_axis, v_axis);
    }
    table
}

/// The "side 0" cell offset per face (vanilla `BlockModelLighter.AdjacencyInfo`
/// `corners[0]` plus the face normal): vanilla substitutes this cell's shade and
/// light for a vertex's diagonal sample whenever both of that vertex's side
/// cells are non-light-permeable — whichever vertex the diagonal belongs to
/// (so for two of the four vertices it is not one of their own sides; that is
/// vanilla's behaviour, kept as the oracle).
fn face_side0_table() -> [[i32; 3]; 6] {
    use haste_assets::Direction as D;
    // AdjacencyInfo.corners[0] per face, in Haste's Direction::ALL order
    // (down, up, north, south, west, east).
    let side0 = [
        D::West, // down
        D::East, // up
        D::Up,   // north
        D::West, // south
        D::Up,   // west
        D::Down, // east
    ];
    let mut table = [[0i32; 3]; 6];
    for (di, dir) in D::ALL.into_iter().enumerate() {
        let n = dir.offset();
        let s = side0[di].offset();
        table[di] = [n[0] + s[0], n[1] + s[1], n[2] + s[2]];
    }
    table
}

/// One AO/light sample cell: whether it darkens (`getShadeBrightness <= 0.2`)
/// and its packed light.
#[derive(Clone, Copy)]
struct AoSample {
    occludes: bool,
    light: u8,
}

/// The per-vertex corner blend, vanilla `BlockModelLighter`'s cubic path:
/// the AO occluder count over the two sides, the diagonal (the caller has
/// already applied vanilla's side-0 substitution) and the center, plus the
/// light blend of `LightCoordsUtil.smoothBlend` — the center's light averaged
/// with the three samples, where a fully-dark neighbour takes the center's
/// light and a zero-sky one its sky, but only when the center itself is lit
/// (sky > 2 or block > 2). The light comes back in vanilla's smooth units
/// (16 per level, 0..240 — the exact average, like vanilla keeps into the
/// lightmap; `LIGHT16_PER_LEVEL`).
fn ao_corner_blend(sides: [AoSample; 2], diagonal: AoSample, center: AoSample) -> (u8, [u8; 2]) {
    let ao = vertex_ao_level(
        sides[0].occludes,
        sides[1].occludes,
        diagonal.occludes,
        center.occludes,
    );
    let (cs, cb) = (center.light >> 4, center.light & 0xF);
    let lit = cs > 2 || cb > 2;
    let mut sky = cs as u32;
    let mut block = cb as u32;
    for s in sides.into_iter().chain([diagonal]) {
        let (mut s, mut b) = (s.light >> 4, s.light & 0xF);
        if lit {
            if s == 0 && b == 0 {
                s = cs;
                b = cb;
            } else if s == 0 {
                s = cs;
            }
        }
        sky += s as u32;
        block += b as u32;
    }
    // Vanilla keeps the exact average in 16th-of-a-level units (0..240) into
    // the lightmap; the vertex carries it (`LIGHT16_PER_LEVEL`).
    (
        ao,
        [
            (sky * 4).min(crate::vertex::LIGHT16_MAX as u32) as u8,
            (block * 4).min(crate::vertex::LIGHT16_MAX as u32) as u8,
        ],
    )
}

/// Per-vertex AO occluder count + packed light for face vertex `vi` (vanilla
/// `BlockModelLighter.prepareQuadAmbientOcclusion`, the cubic path).
/// Coordinates are section-local; the +1 maps the section into the snapshot's
/// border.
fn vertex_light_for(
    ctx: &MeshContext,
    lx: i32,
    ly: i32,
    lz: i32,
    dir: haste_assets::Direction,
    vi: usize,
) -> (u8, [u8; 2]) {
    // (side1, side2, corner) offsets per vertex, in face_positions order.
    let corners = face_corners()[dir.index()][vi];
    let [s1, s2, corner] = corners;
    let (x, y, z) = (lx + 1, ly + 1, lz + 1);

    let cell = |d: [i32; 3]| ctx.cell(x + d[0], y + d[1], z + d[2]);
    // The AO occluder test (`BlockBehaviour.getShadeBrightness`): a full
    // collision cube darkens (0.2), not texture opacity — glass and slabs
    // keep their neighbours bright.
    let sample = |d: [i32; 3]| AoSample {
        occludes: ctx.tables.ao.is_solid(cell(d).state),
        light: cell(d).light,
    };
    // Vanilla `BlockState.isLightPermeable` (whether the diagonal sample is
    // reachable): approximated as solid-render (`baked.is_full_opaque`); the
    // light-dampening half of the vanilla test has no data in the client yet
    // (the server's light values already carry it).
    let permeable = |d: [i32; 3]| !ctx.baked.is_full_opaque(cell(d).state);

    // The diagonal sample: the corner cell, or vanilla's side-0 cell when
    // both side cells are non-permeable (vanilla reads side 0's shade and
    // light there — not this vertex's own side).
    let diagonal = if !permeable(s1) && !permeable(s2) {
        sample(face_side0()[dir.index()])
    } else {
        sample(corner)
    };

    // Center sample: the face-adjacent cell (vanilla's cubic path, which any
    // rendered quad of a full-collision block takes; a partial block's
    // non-cubic quads read the block's own cell in vanilla, which for
    // non-full blocks is bright like the neighbour usually is). The corner
    // table's offsets already carry the face normal (checked against
    // vanilla's AdjacencyInfo by `face_corner_offsets_match_vanilla_adjacency`),
    // so sampling one cell further out would make two adjacent coplanar
    // faces read different cells and draw a line along every block edge.
    ao_corner_blend([sample(s1), sample(s2)], diagonal, sample(dir.offset()))
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
/// Something mesh workers can capture section snapshots from on their own threads (the
/// client's shared, lock-protected world). `None` when the section's chunk is gone.
pub trait SnapshotSource: Send + Sync {
    /// Captures `pos` with the biome cells within `blend_radius` blocks (for tints).
    fn capture(&self, pos: SectionPos, blend_radius: i32) -> Option<MeshSnapshot>;
}

pub struct Mesher {
    baked: Arc<haste_assets::BakedBlockModels>,
    atlas: Arc<dyn haste_assets::AtlasRegions + Send + Sync>,
    tables: Mutex<MeshTables>,
    pending: Mutex<Vec<(SectionPos, Arc<MeshSnapshot>)>>,
    results: crossbeam_channel::Receiver<SectionMesh>,
    result_tx: crossbeam_channel::Sender<SectionMesh>,
    /// Sections handed to workers whose mesh has not been sent yet.
    in_flight: Arc<std::sync::atomic::AtomicUsize>,
}

impl Mesher {
    pub fn new(
        baked: Arc<haste_assets::BakedBlockModels>,
        atlas: Arc<dyn haste_assets::AtlasRegions + Send + Sync>,
    ) -> Mesher {
        // Unbounded: workers never block a rayon thread waiting for the render thread.
        let (tx, rx) = crossbeam_channel::unbounded();
        Mesher {
            baked,
            atlas,
            tables: Mutex::new(MeshTables::default()),
            pending: Mutex::new(Vec::new()),
            results: rx,
            result_tx: tx,
            in_flight: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// Submit a section for meshing. The snapshot is captured on the caller's
    /// thread (world data is not Send across threads); deduplicates pending
    /// submissions for the same section.
    pub fn submit(&self, view: &dyn BlockView, pos: SectionPos) {
        self.submit_with_biomes(view, None, pos);
    }

    /// `submit` with the biome source for tints.
    pub fn submit_with_biomes(
        &self,
        view: &dyn BlockView,
        biomes: Option<&dyn crate::tint::BiomeSource>,
        pos: SectionPos,
    ) {
        let radius = self.tables.lock().colors.radius();
        {
            let mut pending = self.pending.lock();
            if pending.iter().any(|(p, _)| *p == pos) {
                return;
            }
            pending.push((
                pos,
                Arc::new(MeshSnapshot::capture_with_biomes(view, biomes, radius, pos)),
            ));
        }
        self.kick();
    }

    /// Replaces the tint/fluid tables used for sections meshed from now on.
    pub fn set_tables(&self, tables: MeshTables) {
        *self.tables.lock() = tables;
    }

    pub fn tables(&self) -> MeshTables {
        self.tables.lock().clone()
    }

    /// Capture and mesh `positions` on the worker pool: the render thread only schedules.
    pub fn submit_from(&self, source: &Arc<dyn SnapshotSource>, positions: &[SectionPos]) {
        if positions.is_empty() {
            return;
        }
        use std::sync::atomic::Ordering;
        let jobs = positions.to_vec();
        self.in_flight.fetch_add(jobs.len(), Ordering::Relaxed);
        let baked = self.baked.clone();
        let tables = self.tables();
        let radius = tables.colors.radius();
        let atlas = self.atlas.clone();
        let tx = self.result_tx.clone();
        let source = source.clone();
        let in_flight = self.in_flight.clone();
        rayon::spawn(move || {
            jobs.into_par_iter().for_each(|pos| {
                if let Some(snapshot) = source.capture(pos, radius) {
                    let _ = tx.send(mesh_section_with(&snapshot, &baked, &*atlas, &tables, pos));
                }
                in_flight.fetch_sub(1, Ordering::Relaxed);
            });
        });
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
        let tables = self.tables.lock().clone();
        let tx = self.result_tx.clone();
        rayon::spawn(move || {
            jobs.into_par_iter()
                .map(|(pos, snapshot)| mesh_section_with(&snapshot, &baked, &*atlas, &tables, pos))
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
        self.pending.lock().len()
            + self.results.len()
            + self.in_flight.load(std::sync::atomic::Ordering::Relaxed)
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
    fn state_cache_rekeys_per_atlas() {
        // The per-state cache is thread-local and persists across sections;
        // it is only valid for the (baked, atlas, fluids) set it was built
        // from. A second Renderer in the same process (reconnect, a new
        // world) reuses the mesh thread, so meshing with a different atlas
        // on the same thread must rebuild, not reuse the stale entries.
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
        let mut atlas1 = haste_assets::AtlasBuilder::build(&mut assets, &[], false).unwrap();
        let baked =
            haste_assets::BakedBlockModels::bake(&mut assets, &states, &mut atlas1).unwrap();

        // A different atlas over the same baked models: only stone is packed,
        // so stone's sprite id differs from the full atlas's.
        let stone = haste_core::Identifier::parse("minecraft:block/stone").unwrap();
        let atlas2 = haste_assets::AtlasBuilder::build(&mut assets, &[stone], false).unwrap();
        let sprite1 = atlas1.region("minecraft:block/stone").sprite as u16;
        let sprite2 = atlas2.region("minecraft:block/stone").sprite as u16;
        assert_ne!(
            sprite1, sprite2,
            "test premise: atlases place stone differently"
        );

        // One layer of stone: every emitted vertex comes from the stone
        // texture, so every vertex's sprite names the atlas it was meshed with.
        let world = FlatWorld {
            ground: BlockStateId(1),
            layers: 1,
        };
        let snapshot = MeshSnapshot::capture(&world, SectionPos::new(0, 0, 0));
        let pos = SectionPos::new(0, 0, 0);

        // A fresh MeshTables per call: each set carries its own generation,
        // so a set whose allocator addresses reuse a previous set's (the
        // reconnect ABA) still re-keys the cache.
        let mesh1 = mesh_section_with(&snapshot, &baked, &atlas1, &MeshTables::default(), pos);
        assert!(
            mesh1.vertices.iter().all(|v| v.sprite == sprite1),
            "atlas1 mesh must carry atlas1's stone sprite"
        );

        let mesh2 = mesh_section_with(&snapshot, &baked, &atlas2, &MeshTables::default(), pos);
        assert_eq!(mesh2.vertices.len(), mesh1.vertices.len());
        assert!(
            mesh2.vertices.iter().all(|v| v.sprite == sprite2),
            "atlas2 mesh must be rebuilt with atlas2's stone sprite, not the cached one"
        );

        let mesh3 = mesh_section_with(&snapshot, &baked, &atlas1, &MeshTables::default(), pos);
        assert!(
            mesh3.vertices.iter().all(|v| v.sprite == sprite1),
            "switching back to atlas1 must re-key again"
        );
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

    /// The client jar's baked models and atlas (None without the cache: the caller
    /// returns, so the test is skipped rather than failed).
    fn load_baked() -> Option<(
        haste_assets::BlocksJsonStates,
        haste_assets::BakedBlockModels,
        haste_assets::AtlasBuilder,
    )> {
        let jar = haste_core::paths::client_jar();
        if !jar.exists() {
            eprintln!("ignored: needs the client jar in the cache");
            return None;
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
        Some((states, baked, atlas))
    }

    /// `minecraft:grass_block`'s non-snowy state.
    fn grass_state(states: &haste_assets::BlocksJsonStates) -> BlockStateId {
        use haste_core::view::BlockStateTable;
        (0..states.len() as u32)
            .map(BlockStateId)
            .find(|s| {
                states.block_of(*s).path() == "grass_block"
                    && states.property_value(*s, "snowy") == Some("false")
            })
            .expect("grass_block in blocks.json")
    }

    /// One vertex's tint (0xRRGGBB), unpacked from the packed bytes.
    fn vertex_tint(v: &PackedVertex) -> u32 {
        ((v.light[2] as u32) << 16) | ((v.light[3] as u32) << 8) | v.tint_b[0] as u32
    }

    #[test]
    #[ignore = "needs the client jar and datagen reports in the cache"]
    fn every_tinted_quad_of_a_block_takes_its_blended_colour() {
        let Some((states, baked, atlas)) = load_baked() else {
            return;
        };
        use haste_core::view::BlockStateTable as _;
        // A snowy biome's grass colour: explicit, and nothing like the default green.
        let mut snowy = crate::tint::BiomeClimate::plains();
        snowy.grass_color = Some(0x3A7D5C);
        let tables = MeshTables::new(
            Arc::new(crate::tint::TintTable::new(&states)),
            Arc::new(
                crate::tint::BiomeColors::new(&[snowy], &crate::tint::Colormaps::uniform(0x91BD59))
                    .with_radius(0),
            ),
            Arc::new(crate::fluid::FluidTable::empty()),
            Arc::new(AoTable::all_solid(states.len())),
        );
        struct Snowy;
        impl crate::tint::BiomeSource for Snowy {
            fn biome(&self, _pos: BlockPos) -> haste_core::BiomeId {
                haste_core::BiomeId(0)
            }
        }
        let grass = grass_state(&states);
        let world = FlatWorld {
            ground: grass,
            layers: 1,
        };
        let pos = SectionPos::new(0, 0, 0);
        let snapshot = MeshSnapshot::capture_with_biomes(&world, Some(&Snowy), 0, pos);
        let mesh = mesh_section_with(&snapshot, &baked, &atlas, &tables, pos);

        let sprite = |texture: &str| atlas.region(texture).sprite as u16;
        let (top, side, overlay, bottom) = (
            sprite("minecraft:block/grass_block_top"),
            sprite("minecraft:block/grass_block_side"),
            sprite("minecraft:block/grass_block_side_overlay"),
            sprite("minecraft:block/dirt"),
        );
        // Every quad with a tint index (the top face and the side overlay, both
        // tintindex 0 in block/grass_block) takes the block's blended grass colour;
        // the untinted side and bottom quads stay white.
        let mut tinted = 0usize;
        let mut untinted = 0usize;
        for v in &mesh.vertices {
            let tint = vertex_tint(v);
            match v.sprite {
                s if s == top || s == overlay => {
                    assert_eq!(tint, 0x3A7D5C, "tinted quad's colour");
                    tinted += 1;
                }
                s if s == side || s == bottom => {
                    assert_eq!(tint, TINT_WHITE, "untinted quad's colour");
                    untinted += 1;
                }
                _ => {}
            }
        }
        assert!(tinted >= 4 && untinted >= 4, "both kinds must be meshed");
    }

    /// The grass block's side overlay is the model's second element: four quads
    /// lying exactly on the side faces, one tintindex over the same plane the side
    /// texture covers. The mesher must emit it unchanged (vanilla draws both
    /// quads; the terrain depth test lets them share a depth), or the side loses
    /// the biome tint the overlay carries.
    #[test]
    #[ignore = "needs the client jar and datagen reports in the cache"]
    fn side_overlay_quad_is_meshed_on_the_faces_plane() {
        let Some((states, baked, atlas)) = load_baked() else {
            return;
        };
        let grass = grass_state(&states);
        let world = FlatWorld {
            ground: grass,
            layers: 1,
        };
        let pos = SectionPos::new(0, 0, 0);
        let snapshot = MeshSnapshot::capture(&world, pos);
        let mesh = mesh_section_with(&snapshot, &baked, &atlas, &MeshTables::default(), pos);

        // The packed position, back to section-local blocks (a quantization step is
        // far below any model feature, so this recovers the exact quad).
        let decode = |p: [u16; 4]| {
            std::array::from_fn(|i| {
                f32::from(p[i]) / 65535.0 * crate::vertex::POS_RANGE - crate::vertex::POS_BIAS
            })
        };
        let close = |a: [f32; 3], b: [f32; 3]| {
            a.iter()
                .zip(b)
                .all(|(x, y)| (x - y).abs() < 2.0 / 65535.0 * crate::vertex::POS_RANGE)
        };
        let sprite_of = |texture: &str| atlas.region(texture).sprite as u16;
        let overlay_sprite = sprite_of("minecraft:block/grass_block_side_overlay");
        let side_sprite = sprite_of("minecraft:block/grass_block_side");
        let at = |quad: &haste_assets::BakedQuad| quad.direction == haste_assets::Direction::North;
        let base = baked
            .quads(grass)
            .iter()
            .find(|q| at(q) && q.tint_index.is_none())
            .expect("the grass block's untinted side quad");
        let overlay = baked
            .quads(grass)
            .iter()
            .find(|q| at(q) && q.tint_index.is_some())
            .expect("the grass block's tinted side overlay quad");
        // The model's two elements share the face plane: same direction, same four
        // vertices. The mesh must keep the overlay on that plane, untouched.
        assert_eq!(overlay.positions, base.positions, "coplanar model elements");

        for want in base.positions {
            assert!(
                mesh.vertices
                    .iter()
                    .any(|v| v.sprite == side_sprite && close(decode(v.pos), want)),
                "the side quad on the face: {want:?} is not in the mesh"
            );
            assert!(
                mesh.vertices
                    .iter()
                    .any(|v| v.sprite == overlay_sprite && close(decode(v.pos), want)),
                "the overlay quad shares the face plane, but {want:?} is not in the mesh"
            );
        }
        assert!(
            mesh.vertices.iter().any(|v| v.sprite == overlay_sprite),
            "the grass block's side overlay must be meshed"
        );
    }

    #[test]
    fn table_generations_are_unique() {
        // Every constructed set takes the next counter value: two sets never
        // share a generation, even if the allocator reuses a freed set's
        // addresses (the reconnect ABA the cache key must survive).
        let a = MeshTables::default();
        let b = MeshTables::default();
        assert_ne!(a.generation, b.generation);
        let c = MeshTables::new(
            Arc::new(crate::tint::TintTable::empty()),
            Arc::new(crate::tint::BiomeColors::default()),
            Arc::new(crate::fluid::FluidTable::empty()),
            Arc::new(AoTable::all_solid(0)),
        );
        assert_ne!(a.generation, c.generation);
        assert_ne!(b.generation, c.generation);
    }

    #[test]
    fn ao_level_counts_occluders() {
        // Vanilla's brightness is 1.0 - 0.2 * count over the four samples
        // (two sides, diagonal, center).
        assert_eq!(vertex_ao_level(false, false, false, false), 0);
        assert_eq!(vertex_ao_level(true, false, false, false), 1);
        assert_eq!(vertex_ao_level(true, true, false, false), 2);
        assert_eq!(vertex_ao_level(true, true, true, false), 3);
        assert_eq!(vertex_ao_level(true, true, true, true), 4);
    }

    /// Hand-computed corner blend (vanilla BlockModelLighter's shade average
    /// and LightCoordsUtil.smoothBlend), one up-face vertex: both side cells
    /// solid-render, so the diagonal sample is vanilla's side-0 substitution,
    /// and a zero-light neighbour takes the center's light.
    #[test]
    fn corner_blend_matches_vanilla_rules() {
        let stone = |sky: u8, block: u8| AoSample {
            occludes: true,
            light: pack_light(sky, block),
        };
        let air = |sky: u8, block: u8| AoSample {
            occludes: false,
            light: pack_light(sky, block),
        };
        // Center (the cell above the block): air, light (sky 12, block 0).
        // Sides: both stone (ao-solid, solid-render), the west one's light
        // fully dark (0, 0), the south one's (15, 0). Both non-permeable, so
        // the diagonal is replaced by vanilla's side-0 sample: air, bright,
        // (15, 0).
        // AO: stone + stone + air + air = 2 occluders (brightness 0.6).
        // Light: the center is lit (12 > 2), so the west side's zero light
        // becomes the center's (12, 0); the sky levels sum to 12 + 12 + 15 +
        // 15 = 54, whose exact average in 16th units is 54 * 4 = 216 (level
        // 13.5 — the fraction is the point of the smooth units); block 0.
        let (ao, [sky16, block16]) = ao_corner_blend(
            [stone(15, 0), stone(0, 0)],
            air(15, 0), // the substituted diagonal (side 0)
            air(12, 0), // center
        );
        assert_eq!(ao, 2);
        assert_eq!(sky16, 216, "zero-light neighbour takes the center's sky");
        assert_eq!(block16, 0);

        // Without the zero-light fallback the sky levels would sum to 12 + 0
        // + 15 + 15 = 42 (168 in 16ths) — the test above pins the rule.
        let (_, [l2, _]) = ao_corner_blend([stone(15, 0), stone(9, 0)], air(15, 0), air(12, 0));
        assert_eq!(l2, 204, "no substitution when the neighbour is lit");

        // The center below the blend threshold: no substitution at all
        // (vanilla applies the fallbacks only when the center is lit).
        let (_, [l3, _]) = ao_corner_blend([stone(15, 0), stone(0, 0)], air(15, 0), air(2, 0));
        assert_eq!(l3, 128, "dark center: zero light stays zero");
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

    /// The side-0 cell per face is vanilla AdjacencyInfo.corners[0] plus the
    /// face normal (the cell whose shade and light vanilla substitutes for an
    /// unreachable diagonal).
    #[test]
    fn face_side0_matches_vanilla_adjacency() {
        use haste_assets::Direction as D;
        // (face, AdjacencyInfo.corners[0])
        let vanilla = [
            (D::Down, D::West),
            (D::Up, D::East),
            (D::North, D::Up),
            (D::South, D::West),
            (D::West, D::Up),
            (D::East, D::Down),
        ];
        for (dir, side0) in vanilla {
            let n = dir.offset();
            let s = side0.offset();
            assert_eq!(
                face_side0()[dir.index()],
                [n[0] + s[0], n[1] + s[1], n[2] + s[2]],
                "{dir:?} side-0 cell"
            );
        }
    }

    /// A world whose sky light is a function of y alone: 9 at y=1, 3 at y=2.
    /// The smooth light of an up face must then be 9 (its four cells all sit at
    /// y=1); sampling the neighbours one cell further out would average the y=2
    /// cells in and give 5.
    struct LitByHeight {
        ground: BlockStateId,
        layers: i32,
    }

    impl haste_core::view::BlockView for LitByHeight {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
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
            match pos.y {
                1 => 9,
                2 => 3,
                _ => 15,
            }
        }
        fn block_light(&self, _pos: BlockPos) -> u8 {
            0
        }
        fn min_y(&self) -> i32 {
            0
        }
        fn height(&self) -> i32 {
            512
        }
    }

    /// The vertex light's sky channel, in whole levels (smooth units / 16).
    fn vertex_sky_levels(v: &PackedVertex) -> u8 {
        v.light[0] / crate::vertex::LIGHT16_PER_LEVEL as u8
    }

    #[test]
    #[ignore = "needs the client jar and datagen reports in the cache"]
    fn smooth_light_reads_the_ao_neighbour_cells() {
        let Some((states, baked, atlas)) = load_baked() else {
            return;
        };
        let ground = grass_state(&states);
        // One layer of opaque blocks, so the top faces' four cells all sit at
        // y=1 (the plane above the blocks) and their light is 9.
        let world = LitByHeight { ground, layers: 1 };
        let pos = SectionPos::new(0, 0, 0);
        let snapshot = MeshSnapshot::capture(&world, pos);
        let mesh = mesh_section(&snapshot, &baked, &atlas, pos);
        // The up faces take the shade index 1 and are tinted; find them through
        // the tinted top sprite.
        let up = atlas.region("minecraft:block/grass_block_top").sprite as u16;
        let mut seen = 0;
        for v in &mesh.vertices {
            if v.sprite == up {
                assert_eq!(
                    vertex_sky_levels(v),
                    9,
                    "up face light samples the y=1 cells"
                );
                seen += 1;
            }
        }
        assert!(seen > 0, "the section has top faces");
    }

    /// The same scene: the *side* faces of the layer must read the light of the
    /// cells beside them. Their four cells per vertex are the face-adjacent
    /// column's cell (light 15) and its row at y (15) and at y±1 (9 for the
    /// upper corner cells), so the side faces carry 15 on the lower corners and
    /// 12 on the upper ones. Sampling the neighbours one cell further out along
    /// the face normal (as this code did before) would read the y=2 cells and
    /// give 9 instead.
    #[test]
    #[ignore = "needs the client jar and datagen reports in the cache"]
    fn smooth_light_of_side_faces_reads_the_adjacent_column() {
        let Some((states, baked, atlas)) = load_baked() else {
            return;
        };
        let ground = grass_state(&states);
        let world = LitByHeight { ground, layers: 1 };
        let pos = SectionPos::new(0, 0, 0);
        let snapshot = MeshSnapshot::capture(&world, pos);
        let mesh = mesh_section(&snapshot, &baked, &atlas, pos);
        let side = atlas.region("minecraft:block/grass_block_side").sprite as u16;
        let mut lights = std::collections::BTreeSet::new();
        for v in &mesh.vertices {
            if v.sprite == side {
                lights.insert(v.light[0]);
            }
        }
        assert_eq!(
            lights.into_iter().collect::<Vec<_>>(),
            vec![192, 240],
            "side face light: the corner cells beside the face, not further out"
        );
    }
}
