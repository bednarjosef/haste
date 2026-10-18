//! Per-state occlusion shapes for the fluid mesher, after vanilla's
//! `BlockState.getOcclusionShape` / `getFaceOcclusionShape` (`BlockBehaviour.initCache`):
//! the shape `FluidRenderer.isFaceOccludedByState` compares the fluid's face against
//! (`Shapes.blockOccludes`, see `crate::fluid_occlusion`).
//!
//! The occlusion shape comes from the oracle dump's `occlusion` field (the 2026-10-18
//! regeneration): the state's own shape for blocks that can occlude, `Shapes.empty()` for
//! the `noOcclusion` family (glass, leaves, panes, ...), and a smaller shape for the four
//! overrides (fences, fence gates, lecterns, sculk shriekers). Classification follows
//! vanilla `BlockStateBase.initCache`: an empty occlusion shape never occludes, a full cube
//! is solid render and occludes as `Shapes.block()`, anything else occludes as its boxes.
//!
//! Dumps from before the field fall back to the outline plus what vanilla's rules imply
//! about it (an empty outline is an empty occlusion shape; a full-cube outline occludes as
//! a full block exactly when the state is solid render — the `noOcclusion` family with
//! full outlines like glass or leaves never is; any other outline is the occluder as its
//! boxes). The classes that cannot see: `noOcclusion` blocks with partial outlines whose
//! shape still touches a shared face, and the four overrides above.
//!
//! The table is built once per connection (no allocation per block): each state's boxes
//! live in one flat arena and the per-state span indexes into it, like vanilla's cached
//! `occlusionShapesByFace`.

use haste_assets::BakedBlockModels;
use haste_core::view::{BlockStateTable, CollisionShapes};
use haste_core::{Aabb, BlockStateId};

use crate::fluid_occlusion::Occluder;

/// Per-state occlusion shape, precomputed for [`crate::fluid::FluidWorld::face_occluder`].
#[derive(Clone, Debug, Default)]
pub struct FluidOccluders {
    /// All partial shapes' boxes, back to back; the spans index into this.
    boxes: Vec<[f32; 6]>,
    spans: Vec<Span>,
}

/// One state's occlusion shape (vanilla caches the same classification per face in
/// `BlockStateBase.initCache`; a full block is the full face everywhere, an empty shape
/// nowhere). A state id past the table has no span: the caller's approximation applies.
#[derive(Clone, Copy, Debug)]
enum Span {
    /// `Shapes.empty()`: never occludes.
    Empty,
    /// `Shapes.block()`: the full cube.
    Full,
    /// A partial shape, as `boxes[start..start + len]`.
    Boxes { start: u32, len: u32 },
}

/// Dump coordinates are rounded to 1e-6, so equality needs that much slack.
fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6
}

/// `Shapes.block()`: the single box spanning the whole unit cube.
fn is_unit_cube(b: &Aabb) -> bool {
    [b.min.x, b.min.y, b.min.z].iter().all(|&v| near(v, 0.0))
        && [b.max.x, b.max.y, b.max.z].iter().all(|&v| near(v, 1.0))
}

impl FluidOccluders {
    /// From the game data the mesher already loads: haste-data's per-state occlusion
    /// shapes (the oracle dump's `occlusion`, vanilla `getOcclusionShape`), the outline
    /// shapes and the baked models' solid-render flag for dumps that predate the field.
    pub fn from_shapes(
        states: &dyn BlockStateTable,
        shapes: &haste_data::BlockShapes,
        baked: &BakedBlockModels,
    ) -> FluidOccluders {
        Self::from_sources(
            states.len(),
            |s| shapes.occlusion_boxes(s),
            |s| shapes.outline_boxes(s),
            |s| baked.is_full_opaque(s),
        )
    }

    /// The classification over an occlusion source (the dump's `occlusion` field) with the
    /// outline classification as the fallback for states it has no data for
    /// (`from_shapes` above; tests feed both directly).
    pub fn from_sources<'a>(
        state_count: usize,
        occlusion: impl Fn(BlockStateId) -> Option<&'a [Aabb]>,
        outline: impl Fn(BlockStateId) -> &'a [Aabb],
        solid_render: impl Fn(BlockStateId) -> bool,
    ) -> FluidOccluders {
        let mut out = FluidOccluders {
            boxes: Vec::new(),
            spans: Vec::with_capacity(state_count),
        };
        for id in 0..state_count as u32 {
            let state = BlockStateId(id);
            let span = match occlusion(state) {
                // Vanilla initCache over the occlusion shape itself: a full-cube occlusion
                // shape is solid render by definition (isShapeFullBlock), so it occludes
                // as the full block without consulting the models.
                Some([]) => Span::Empty,
                Some(boxes) if boxes.len() == 1 && is_unit_cube(&boxes[0]) => Span::Full,
                Some(boxes) => out.span_for(boxes),
                // A dump from before the occlusion field: the outline classification.
                None => {
                    let boxes = outline(state);
                    if boxes.is_empty() {
                        Span::Empty
                    } else if boxes.len() == 1 && is_unit_cube(&boxes[0]) {
                        if solid_render(state) {
                            Span::Full
                        } else {
                            Span::Empty
                        }
                    } else {
                        out.span_for(boxes)
                    }
                }
            };
            out.spans.push(span);
        }
        out
    }

    /// The span for a partial shape: its boxes, appended to the arena.
    fn span_for(&mut self, boxes: &[Aabb]) -> Span {
        let start = self.boxes.len() as u32;
        self.boxes.extend(boxes.iter().map(|b| {
            [
                b.min.x as f32,
                b.min.y as f32,
                b.min.z as f32,
                b.max.x as f32,
                b.max.y as f32,
                b.max.z as f32,
            ]
        }));
        Span::Boxes {
            start,
            len: self.boxes.len() as u32 - start,
        }
    }

    /// The classification over any outline source (tests, and `from_shapes` above).
    /// `outline` must answer the state's outline boxes, `solid_render` its
    /// `BlockState.isSolidRender`.
    pub fn from_outlines<'a>(
        state_count: usize,
        outline: impl Fn(BlockStateId) -> &'a [Aabb],
        solid_render: impl Fn(BlockStateId) -> bool,
    ) -> FluidOccluders {
        Self::from_sources(state_count, |_| None, outline, solid_render)
    }

    /// No shape data at all (tests, before the game data is installed): every lookup
    /// falls past the table and the caller's full-if-opaque approximation applies.
    pub fn absent() -> FluidOccluders {
        FluidOccluders::default()
    }

    /// The state's occlusion shape for `is_face_occluded`. `opaque` answers the caller's
    /// `isSolidRender` and runs only for states the table has no data for (no game data
    /// yet, or an id past the table — a modded server can send those; they fall back to
    /// the same approximation instead of panicking), so the common path pays one span
    /// lookup.
    pub fn occluder(&self, state: BlockStateId, opaque: impl FnOnce() -> bool) -> Occluder<'_> {
        match self.spans.get(state.0 as usize) {
            Some(Span::Full) => Occluder::Full,
            Some(Span::Empty) => Occluder::Empty,
            Some(Span::Boxes { start, len }) => {
                Occluder::Boxes(&self.boxes[*start as usize..(*start + *len) as usize])
            }
            None => {
                if opaque() {
                    Occluder::Full
                } else {
                    Occluder::Empty
                }
            }
        }
    }
}
