// This file is derived from Pomme (https://github.com/josefbednar/pomme),
// pomme-client/src/renderer/chunk/mesher.rs (mesh_section / emit_baked_model /
// compute_face_ao / MeshSink), reused under GPL-3.0 and reworked for Haste:
// meshes per SectionPos against haste-assets' BakedBlockModels over a captured
// BlockView snapshot; light is packed as sky/block nibbles instead of baked
// brightness; the worker pool design follows Pomme's MeshDispatcher (bounded
// result channel, buffer pool, nearest-first camera queue). See THIRD_PARTY.md.

mod culling;
mod dispatch;
mod face_occlusion;
mod fluid_block;
mod fluid_view;
mod lighting;
mod merge;
mod sink;
mod skip;
mod snapshot;
mod tables;
#[cfg(test)]
mod tests;

pub use dispatch::{Mesher, SnapshotSource};
pub use face_occlusion::FaceOccluders;
pub use merge::set_greedy_merge_enabled;
pub use skip::SkipTable;
pub use snapshot::MeshSnapshot;
pub use tables::{AoTable, MeshTables};

use haste_assets::RenderLayer;
use haste_core::{BlockPos, BlockStateId, SectionPos};

use crate::vertex::{pack_ao_shade, pack_light, PackedVertex};
use fluid_block::mesh_fluid;
use lighting::{shade_index, vertex_light_for};
use merge::{
    flush_staged, merge_face, JunctionScratch, MergeFace, MergeKey, MergeQuad, StagedQuad,
};
#[cfg(test)]
pub(crate) use sink::axis_facing;
use sink::{layer_index, MeshSink};
use snapshot::{Cell, EXT};

pub use sink::SectionLayerMesh;
pub use sink::{QuadSpan, GROUP_ORDER};

/// One section's meshed geometry, split by render layer.
pub struct SectionMesh {
    pub pos: SectionPos,
    /// Vertices shared by all layers (empty in the quad format).
    pub vertices: Vec<PackedVertex>,
    /// Rect quad records (quad format only).
    pub quads: Vec<crate::quad_format::PackedQuad>,
    /// General quad records (quad format only): cross planes, tilted fluids
    /// and split merged rectangles' fan triangles.
    pub generals: Vec<crate::quad_format::PackedGeneral>,
    /// Layers in draw order: indexed (vertex format) or spans (quad format).
    pub layers: Vec<sink::SectionLayerMesh>,
    /// Section-local AABB (min/max in blocks) for frustum culling.
    pub aabb: ([f32; 3], [f32; 3]),
    /// Face-to-face sightlines through the section (cave culling).
    pub visibility: crate::occlusion::VisibilitySet,
    /// Set only by [`SectionMesh::removed`]: the chunk is gone, so the renderer frees the
    /// section. An ordinary mesh with no geometry (air, or solid stone with every face
    /// hidden) is not a removal: it keeps its visibility entry.
    pub removed: bool,
}

impl SectionMesh {
    /// The marker a mesh worker sends for a section whose chunk is gone: `upload_mesh`
    /// frees the resident mesh and visibility entry instead of storing geometry.
    pub fn removed(pos: SectionPos) -> SectionMesh {
        SectionMesh {
            pos,
            vertices: Vec::new(),
            quads: Vec::new(),
            generals: Vec::new(),
            layers: Vec::new(),
            aabb: ([0.0; 3], [0.0; 3]),
            visibility: crate::occlusion::VisibilitySet::none(),
            removed: true,
        }
    }

    /// Whether this mesh only asks the renderer to drop what is resident at its position
    /// (a forgotten chunk).
    pub fn is_removal(&self) -> bool {
        self.removed
    }
}

/// Untinted vertex colour.
pub const TINT_WHITE: u32 = 0xFFFFFF;

/// Vanilla `BlockModelLighter.prepareQuadShape`'s faceCubic per quad: flat
/// along the face normal AND (on the block boundary OR the state's collision
/// shape is a full cube), with vanilla's 1e-4 / 0.9999 thresholds. When not
/// cubic, vanilla samples the AO/light center from the block's own cell
/// instead of the face-adjacent one.
fn quad_cubic(quad: &haste_assets::BakedQuad, state_cubic: bool) -> bool {
    use haste_assets::Direction as D;
    let (min, max) = quad_extremes(quad);
    // (normal axis, at the low boundary): the axis the face is flat along.
    let (axis, low) = match quad.direction {
        D::Down | D::Up => (1, quad.direction == D::Down),
        D::North | D::South => (2, quad.direction == D::North),
        D::West | D::East => (0, quad.direction == D::West),
    };
    let flat = min[axis] == max[axis];
    let on_boundary = if low {
        min[axis] < 1.0e-4
    } else {
        max[axis] > 0.9999
    };
    flat && (on_boundary || state_cubic)
}

/// The min/max corner of a baked quad's model-space positions.
fn quad_extremes(quad: &haste_assets::BakedQuad) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for p in &quad.positions {
        for a in 0..3 {
            min[a] = min[a].min(p[a]);
            max[a] = max[a].max(p[a]);
        }
    }
    (min, max)
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
    /// Vanilla `faceCubic` (`BlockModelLighter.prepareQuadShape`): the quad
    /// is flat along the face normal AND either lies on the block boundary or
    /// the state's collision shape is a full cube. Decides the AO/light
    /// center sample: the face-adjacent cell when cubic, else the block's
    /// own cell.
    cubic: bool,
    sprite: u16,
    /// Greedy-merge signature (see [`MergeFace`]): Some when this quad is a
    /// full unit face whose sprite-local UVs are exactly the 0/1 corner
    /// pattern, so identical neighbours with identical light can merge into
    /// one repeating quad.
    merge: Option<MergeFace>,
}

/// Per-state meshing info, built lazily per section on first sight of a state.
/// (The state population is tiny compared to the block population.)
struct StateInfo {
    quads: Vec<EmitQuad>,
    /// Render layer of the state's quads.
    layer: RenderLayer,
    /// The state's fluid, if any.
    fluid: Option<crate::fluid::FluidState>,
    /// Face-plane occlusion coverage (vanilla `Block.shouldRenderFace`'s shape
    /// comparison): what this state's opaque quads hide of a neighbour's
    /// flush face, per cullface direction.
    cover: culling::FaceCover,
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
    /// The section's quads between emission and the sink: the greedy merge
    /// needs them all before any is written (see `StagedQuad`).
    staged: Vec<StagedQuad>,
    /// The greedy merge's junction marks and the ring of the rectangle
    /// being emitted (see `merge/junction.rs`).
    junction: JunctionScratch,
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
            staged: Vec::new(),
            junction: JunctionScratch::new(),
            state_key: None,
        }
    }

    /// Reset the per-section scratch. The state cache is deliberately NOT
    /// cleared: it is a pure function of the immutable baked models, atlas and
    /// tables, so it persists across sections (clearing it would redo the
    /// atlas lookups and re-allocate every state's quad vector per section).
    fn begin_section(&mut self) {
        self.sink.clear();
        self.fluid_scratch.clear();
        self.staged.clear();
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
            // Vanilla's faceCubic test needs the state's collision-full flag
            // (the quad-geometry half is per quad below).
            let state_cubic = self.tables.ao.is_cubic(state);
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
                        merge: merge_face(quad, &uvs),
                        positions: quad.positions,
                        cullface: quad.cullface,
                        shade_face: quad.shade_face,
                        direction: quad.direction,
                        tinted: quad.tint_index.is_some(),
                        cubic: quad_cubic(quad, state_cubic),
                        sprite,
                    }
                })
                .collect();
            self.states[idx] = Some(std::rc::Rc::new(StateInfo {
                cover: culling::FaceCover::from_quads(self.baked.quads(state), self.atlas),
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
            staged,
            junction,
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
                        // Cullface: vanilla `Block.shouldRenderFace`: the state's
                        // skipRendering rule (skip.rs), then the dump's occlusion
                        // shapes; None on an old dump keeps the model rule
                        // (full-opaque neighbour, or the neighbour's opaque quads
                        // covering this quad's extent on the shared plane).
                        if let Some(cull) = quad.cullface {
                            let o = cull.offset();
                            let neighbor = ctx.cell(lx + 1 + o[0], ly + 1 + o[1], lz + 1 + o[2]);
                            let visible = !ctx.tables.skip.skips(state, neighbor.state, cull)
                                && match ctx.tables.face_occluders.should_render_face(
                                    state,
                                    neighbor.state,
                                    cull,
                                ) {
                                    Some(v) => v,
                                    None => {
                                        !ctx.baked.is_full_opaque(neighbor.state)
                                            && !ctx
                                                .state_info(neighbor.state)
                                                .cover
                                                .covers(cull, &quad.positions)
                                    }
                                };
                            if !visible {
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
                                vertex_light_for(&ctx, lx, ly, lz, quad.direction, quad.cubic, vi);
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

                        // Greedy merging: a full unit face with uniform light
                        // (and no per-vertex tint — the tint is per quad) can
                        // merge with identical neighbours; the shader's
                        // sprite-local UV wrap tiles the texture across the
                        // merged quad, so the pixels stay identical. Translucent
                        // keeps per-quad order (blending between coplanar
                        // faces depends on it).
                        let mergeable = merge::greedy_merge_enabled()
                            && quad.merge.is_some()
                            && info.layer != RenderLayer::Translucent
                            && lights[0] == lights[1]
                            && lights[1] == lights[2]
                            && lights[2] == lights[3];
                        if mergeable {
                            staged.push(StagedQuad::Merge(MergeQuad {
                                key: MergeKey {
                                    layer: layer_index(info.layer) as u8,
                                    dir: quad.direction.index() as u8,
                                    sprite: quad.sprite,
                                    tint: quad_tint,
                                    light: lights[0],
                                    face: *quad.merge.as_ref().unwrap(),
                                },
                                cell: [lx as u8, ly as u8, lz as u8],
                            }));
                        } else {
                            staged.push(StagedQuad::Direct {
                                layer: info.layer,
                                positions,
                                uvs: quad.uvs,
                                lights,
                                tint: quad_tint,
                                sprite: quad.sprite,
                            });
                        }
                    }
                }
            }
        }
        flush_staged(staged, sink, junction);

        // The section's final output: the sink concatenates its buckets into
        // per-layer index vectors (see `MeshSink::take_layers`).
        let (vertices, quads, generals, layers) = sink.take_layers();

        SectionMesh {
            pos,
            vertices,
            quads,
            generals,
            layers,
            aabb: (sink.min, sink.max),
            visibility: crate::occlusion::section_visibility(snapshot, baked, pos),
            removed: false,
        }
    })
}
