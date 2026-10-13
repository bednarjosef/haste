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

/// Untinted vertex colour.
pub const TINT_WHITE: u32 = 0xFFFFFF;

/// The per-face AO neighbour offsets (see `face_corner_table`), built once.
static FACE_CORNERS: std::sync::OnceLock<[[[[i32; 3]; 3]; 4]; 6]> = std::sync::OnceLock::new();

fn face_corners() -> &'static [[[[i32; 3]; 3]; 4]; 6] {
    FACE_CORNERS.get_or_init(face_corner_table)
}

/// Per-block-state and per-biome tables the mesher reads (shared with the workers).
#[derive(Clone)]
pub struct MeshTables {
    pub tints: Arc<crate::tint::TintTable>,
    pub colors: Arc<crate::tint::BiomeColors>,
    pub fluids: Arc<crate::fluid::FluidTable>,
}

impl Default for MeshTables {
    fn default() -> Self {
        MeshTables {
            tints: Arc::new(crate::tint::TintTable::empty()),
            colors: Arc::new(crate::tint::BiomeColors::default()),
            fluids: Arc::new(crate::fluid::FluidTable::empty()),
        }
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
        lights: [[u8; 2]; 4],
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

/// `vertex::pack_vertex` on plain values (no QuadVertex indirection): identical math.
fn pack_vertex_raw(
    position: [f32; 3],
    uv: [f32; 2],
    light: [u8; 2],
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
/// lazily built per-state info.
struct MeshContext<'a> {
    snapshot: &'a MeshSnapshot,
    baked: &'a haste_assets::BakedBlockModels,
    atlas: &'a dyn haste_assets::AtlasRegions,
    tables: &'a MeshTables,
    /// Section origin in world blocks (fluid and tint lookups want world positions).
    origin: BlockPos,
    /// Lazily built per-state info, indexed by state id (grown on demand, None
    /// until first seen).
    states: Vec<Option<std::rc::Rc<StateInfo>>>,
    /// Reusable scratch for the fluid mesher (no per-block allocation).
    fluid_scratch: Vec<crate::fluid::FluidQuad>,
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
    let mut ctx = MeshContext {
        snapshot,
        baked,
        atlas,
        tables,
        origin: pos.block_min(),
        states: Vec::new(),
        fluid_scratch: Vec::with_capacity(11),
    };
    let mut sink = MeshSink::new();

    for ly in 0..16i32 {
        for lz in 0..16i32 {
            for lx in 0..16i32 {
                let state = ctx.cell(lx + 1, ly + 1, lz + 1).state;
                if state == BlockStateId::AIR {
                    continue;
                }
                let info = ctx.state_info(state);
                if let Some(fluid) = info.fluid {
                    mesh_fluid(&mut ctx, lx, ly, lz, fluid, &mut sink);
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

                    // Per-vertex light: face-adjacent cell + the vertex's AO
                    // neighbours, averaged (smooth lighting).
                    let lights: [[u8; 2]; 4] = core::array::from_fn(|vi| {
                        let (ao_level, light) =
                            vertex_light_for(&ctx, lx, ly, lz, quad.direction, vi);
                        let shade = shade_index(quad.shade_face);
                        [light, pack_ao_shade(ao_level, shade)]
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
        vertices: sink.vertices,
        layers,
        aabb: (sink.min, sink.max),
        visibility: crate::occlusion::section_visibility(snapshot, baked, pos),
    }
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
    use crate::fluid::Fluid;
    let block_pos = ctx.origin.add(lx, ly, lz);
    let mut quads = std::mem::take(&mut ctx.fluid_scratch);
    quads.clear();
    {
        let view = FluidView {
            snapshot: ctx.snapshot,
            baked: ctx.baked,
        };
        crate::fluid::fluid_quads(&view, &ctx.tables.fluids, block_pos, fluid, &mut quads);
    }
    if !quads.is_empty() {
        let tint = match fluid.fluid {
            Fluid::Water => block_tint(
                ctx.snapshot,
                ctx.tables,
                crate::tint::TintKind::Water,
                block_pos,
            ),
            Fluid::Lava => TINT_WHITE,
        };
        let layer = crate::fluid::fluid_layer(fluid.fluid);
        let own = ctx.snapshot.light(block_pos);
        let top_light = max_light(own, ctx.snapshot.light(block_pos.add(0, 1, 0)));
        let bottom_light = max_light(own, ctx.snapshot.light(block_pos.add(0, -1, 0)));
        let offset = [lx as f32, ly as f32, lz as f32];
        for q in &quads {
            let light = if q.dir == haste_assets::Direction::Down {
                bottom_light
            } else {
                top_light
            };
            let packed = [light, pack_ao_shade(3, shade_index(Some(q.dir)))];
            let positions = q
                .positions
                .map(|p| [p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]]);
            sink.push_quad(layer, &positions, &q.uvs, [packed; 4], tint, q.sprite);
        }
    }
    ctx.fluid_scratch = quads;
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

/// Per-vertex AO level + packed light for face vertex `vi` (Pomme
/// compute_face_ao, inlined with the per-face neighbour table). Coordinates
/// are section-local; the +1 maps the section into the snapshot's border.
fn vertex_light_for(
    ctx: &MeshContext,
    lx: i32,
    ly: i32,
    lz: i32,
    dir: haste_assets::Direction,
    vi: usize,
) -> (u8, u8) {
    // (side1, side2, corner) offsets per vertex, in face_positions order.
    let corners = face_corners()[dir.index()][vi];
    let [s1, s2, corner] = corners;
    let (x, y, z) = (lx + 1, ly + 1, lz + 1);

    let opaque = |d: [i32; 3]| -> bool {
        ctx.baked
            .is_full_opaque(ctx.cell(x + d[0], y + d[1], z + d[2]).state)
    };
    let ao = vertex_ao_level(opaque(s1), opaque(s2), opaque(corner));

    // Smooth light: the face-adjacent cell's light plus the three AO-neighbour
    // cells, averaged (vanilla), packed to nibbles. Neighbour offsets are
    // applied on top of the face-adjacent cell (the original `vertex_light`
    // semantics: `base = block_pos + n`, neighbours `base + d`).
    let n = dir.offset();
    let mut s = (ctx.cell(x + n[0], y + n[1], z + n[2]).light >> 4) as u32;
    let mut b = (ctx.cell(x + n[0], y + n[1], z + n[2]).light & 0xF) as u32;
    for d in [s1, s2, corner] {
        let l = ctx
            .cell(x + n[0] + d[0], y + n[1] + d[1], z + n[2] + d[2])
            .light;
        s += (l >> 4) as u32;
        b += (l & 0xF) as u32;
    }
    let sky = ((s + 2) / 4).min(15) as u8;
    let block = ((b + 2) / 4).min(15) as u8;
    (ao, pack_light(sky, block))
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
