//! Corner heights, flow direction and the backward-up-face visibility check: the geometry
//! half of vanilla's `LiquidBlockRenderer`.

use haste_core::BlockPos;

use crate::fluid::registry::{Fluid, FluidState, FluidTable, FluidWorld};

/// The four top-corner heights of a fluid block, in (x, z) order
/// [(0,0), (0,1), (1,1), (1,0)] (north-west, south-west, south-east, north-east).
pub fn corner_heights(
    world: &dyn FluidWorld,
    table: &FluidTable,
    p: BlockPos,
    fluid: FluidState,
) -> [f32; 4] {
    let same = |q: BlockPos| {
        table
            .fluid(world.state(q))
            .filter(|f| f.fluid == fluid.fluid)
    };
    if same(p.add(0, 1, 0)).is_some() {
        return [1.0; 4];
    }
    // Column height: 1 when the same fluid continues above, own height, 0 for open non-fluid
    // blocks, -1 (ignored) for solid ones.
    let column = |q: BlockPos| -> f32 {
        match same(q) {
            Some(f) => {
                if same(q.add(0, 1, 0)).is_some() {
                    1.0
                } else {
                    f.height
                }
            }
            None if world.solid(q) => -1.0,
            None => 0.0,
        }
    };
    let own = fluid.height;
    let corner = |dx: i32, dz: i32| -> f32 {
        let side_x = column(p.add(dx, 0, 0));
        let side_z = column(p.add(0, 0, dz));
        if side_x >= 1.0 || side_z >= 1.0 {
            return 1.0;
        }
        let (mut total, mut weight) = (0.0f32, 0.0f32);
        let mut add = |h: f32| {
            if h >= 0.8 {
                total += h * 10.0;
                weight += 10.0;
            } else if h >= 0.0 {
                total += h;
                weight += 1.0;
            }
        };
        if side_x > 0.0 || side_z > 0.0 {
            let diagonal = column(p.add(dx, 0, dz));
            if diagonal >= 1.0 {
                return 1.0;
            }
            add(diagonal);
        }
        add(own);
        add(side_x);
        add(side_z);
        if weight > 0.0 {
            total / weight
        } else {
            own
        }
    };
    [corner(-1, -1), corner(-1, 1), corner(1, 1), corner(1, -1)]
}

/// Flow direction on the surface (x, z), pointing downhill; zero when flat.
pub fn flow_direction(h: [f32; 4]) -> [f32; 2] {
    // h = [nw, sw, se, ne]; +x is east, +z is south.
    let fx = (h[0] + h[1]) - (h[2] + h[3]);
    let fz = (h[0] + h[3]) - (h[1] + h[2]);
    let len = (fx * fx + fz * fz).sqrt();
    if len < 1e-4 {
        [0.0, 0.0]
    } else {
        [fx / len, fz / len]
    }
}

/// Vanilla's `FluidRenderer.tesselate` face inset (0.001 blocks).
pub const INSET: f32 = 0.001;

/// `FluidRenderer.MAX_FLUID_HEIGHT`: a source block's height (8/9).
pub const MAX_FLUID_HEIGHT: f32 = 0.888_888_9;

/// `FluidState.shouldRenderBackwardUpFace`: the underside of the surface is drawn only when
/// one of the nine blocks of the layer above (centred on `above`) is neither the same fluid
/// nor a solid-rendering block, i.e. when a player could see the surface from below it.
pub fn should_render_backward_up_face(
    world: &dyn FluidWorld,
    table: &FluidTable,
    above: BlockPos,
    fluid: Fluid,
) -> bool {
    for ox in -1..=1 {
        for oz in -1..=1 {
            let q = above.add(ox, 0, oz);
            let state = world.state(q);
            let same = table.fluid(state).is_some_and(|f| f.fluid == fluid);
            if !same && !world.opaque(q) {
                return true;
            }
        }
    }
    false
}
