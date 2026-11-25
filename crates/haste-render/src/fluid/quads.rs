//! Vertex output: the quads of a fluid block, face culling and occlusion, flow-direction UVs
//! and the overlay sprite (vanilla's `FluidRenderer.tesselate`).

use haste_assets::Direction;
use haste_core::BlockPos;

use crate::fluid::geometry::{
    corner_heights, fluid_flow, should_render_backward_up_face, INSET, MAX_FLUID_HEIGHT,
};
use crate::fluid::registry::{FluidState, FluidTable, FluidWorld};
use crate::fluid_occlusion::is_face_occluded;

/// One emitted fluid quad (block-local positions 0..1, sprite-local UVs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FluidQuad {
    pub positions: [[f32; 3]; 4],
    pub uvs: [[f32; 2]; 4],
    pub sprite: u16,
    /// Face direction (shade and light).
    pub dir: Direction,
}

fn reversed(q: FluidQuad) -> FluidQuad {
    FluidQuad {
        positions: [
            q.positions[3],
            q.positions[2],
            q.positions[1],
            q.positions[0],
        ],
        uvs: [q.uvs[3], q.uvs[2], q.uvs[1], q.uvs[0]],
        ..q
    }
}

/// The quads of the fluid at `p` (block-local), appended to `out`.
pub fn fluid_quads(
    world: &dyn FluidWorld,
    table: &FluidTable,
    p: BlockPos,
    fluid: FluidState,
    out: &mut Vec<FluidQuad>,
) {
    let sprites = &table.sprites;
    let def = table.defs.def(fluid.fluid);
    let (still, flow) = (def.still, def.flow);
    let same = |q: BlockPos| {
        table
            .fluid(world.state(q))
            .is_some_and(|f| f.fluid == fluid.fluid)
    };
    let h = corner_heights(world, table, p, fluid);

    // `FluidRenderer.tesselate` pulls every face a hair into the block so fluid does not
    // z-fight with the faces of glass, slabs and other neighbours: top corners down by
    // `INSET` (which the sides then share), sides in by `INSET`, and the bottom (plus the
    // sides' bottom edge) up by `INSET` only when the bottom face is drawn.
    // Faces are culled against the neighbour's occlusion shape (`isFaceOccludedByNeighbor`):
    // the bottom with vanilla's MAX_FLUID_HEIGHT, the top with the lowest corner (a full
    // block above hides only a full-height surface), each side with its higher corner.
    // The bottom and the sides also go through `shouldRenderFace`, whose
    // `isFaceOccludedBySelf` drops faces the fluid block's own state covers (the water
    // surface inside a waterlogged top slab or stair).
    let occluded = |q: BlockPos, dir: Direction, height: f32| {
        is_face_occluded(dir, height, world.face_occluder(q, dir))
    };
    let self_occluded = |dir: Direction| {
        let opp = dir.opposite();
        is_face_occluded(opp, 1.0, world.face_occluder(p, opp))
    };
    let below = p.add(0, -1, 0);
    let render_down = !same(below)
        && !self_occluded(Direction::Down)
        && !occluded(below, Direction::Down, MAX_FLUID_HEIGHT);
    let bottom = if render_down { INSET } else { 0.0 };
    let above = p.add(0, 1, 0);
    let min_h = h.iter().copied().fold(1.0f32, f32::min);
    let render_up = !same(above) && !occluded(above, Direction::Up, min_h);
    let [nw, sw, se, ne] = if render_up { h.map(|c| c - INSET) } else { h };

    // Top: vertex order like a model's up face (NW, SW, SE, NE).
    if render_up {
        let positions = [
            [0.0, nw, 0.0],
            [0.0, sw, 1.0],
            [1.0, se, 1.0],
            [1.0, ne, 0.0],
        ];
        let corners = [[0.0f32, 0.0f32], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
        let [fx, fz] = fluid_flow(world, table, p, fluid);
        let (uvs, sprite) = if fx == 0.0 && fz == 0.0 {
            (corners, still)
        } else {
            // Vanilla rotates the middle-half window of the flow sprite by
            // `atan2(flow.z, flow.x) - pi/2`: v runs along the flow (u = 0.5 + rz*fx -
            // rx*fz scaled, v = 0.5 + rx*fx + rz*fz scaled).
            let uvs = corners.map(|[x, z]| {
                let (rx, rz) = (x - 0.5, z - 0.5);
                [
                    0.5 + 0.5 * (rx * fz - rz * fx),
                    0.5 + 0.5 * (rx * fx + rz * fz),
                ]
            });
            (uvs, flow)
        };
        let top = FluidQuad {
            positions,
            uvs,
            sprite,
            dir: Direction::Up,
        };
        out.push(top);
        if should_render_backward_up_face(world, table, p.add(0, 1, 0), fluid.fluid) {
            out.push(reversed(top));
        }
    }

    // Bottom (model down-face order).
    if render_down {
        out.push(FluidQuad {
            positions: [
                [0.0, bottom, 1.0],
                [0.0, bottom, 0.0],
                [1.0, bottom, 0.0],
                [1.0, bottom, 1.0],
            ],
            uvs: [[0.0, 1.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
            sprite: still,
            dir: Direction::Down,
        });
    }

    // Sides: (direction, neighbour offset, the face's two top corners as (x, z, height) in
    // model vertex order: top-left then top-right seen from outside). The face plane sits
    // `INSET` inside the block (vanilla's `z + 0.001F` / `x + 1.0F - 0.001F`).
    let (lo, hi) = (INSET, 1.0 - INSET);
    type Side = (Direction, [i32; 3], [f32; 3], [f32; 3]);
    let sides: [Side; 4] = [
        (Direction::North, [0, 0, -1], [1.0, lo, ne], [0.0, lo, nw]),
        (Direction::South, [0, 0, 1], [0.0, hi, sw], [1.0, hi, se]),
        (Direction::West, [-1, 0, 0], [lo, 0.0, nw], [lo, 1.0, sw]),
        (Direction::East, [1, 0, 0], [hi, 1.0, se], [hi, 0.0, ne]),
    ];
    for (dir, o, a, b) in sides {
        let q = p.add(o[0], o[1], o[2]);
        if same(q) || self_occluded(dir) || occluded(q, dir, a[2].max(b[2])) {
            continue;
        }
        let overlay = def.overlay && table.overlay(world.state(q));
        // Model side-face order: top-left, bottom-left, bottom-right, top-right.
        let positions = [
            [a[0], a[2], a[1]],
            [a[0], bottom, a[1]],
            [b[0], bottom, b[1]],
            [b[0], b[2], b[1]],
        ];
        let uvs = [
            [0.0, (1.0 - a[2]) * 0.5],
            [0.0, 0.5],
            [0.5, 0.5],
            [0.5, (1.0 - b[2]) * 0.5],
        ];
        let quad = FluidQuad {
            positions,
            uvs,
            sprite: if overlay { sprites.water_overlay } else { flow },
            dir,
        };
        out.push(quad);
        if !overlay {
            out.push(reversed(quad));
        }
    }
}
