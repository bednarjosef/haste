//! The world an arrow's client-side tick reads (`AbstractArrow.tick`, simulated on the client
//! because projectiles are `SERVER_AND_CLIENT` moved): the block it sticks into, the ray clip
//! of its movement and the water it flies through, answered from the loaded chunks and the
//! game data.

use haste_core::view::{BlockView, CollisionShapes};
use haste_core::{BlockPos, DVec3};
use haste_data::GameData;
use haste_entity::arrow::ArrowWorld;
use haste_physics::BlockPhysics;

/// The client's world and game data as the arrow tick's [`ArrowWorld`].
pub(super) struct ArrowWorldView<'a> {
    pub view: &'a dyn BlockView,
    pub data: &'a GameData,
}

impl ArrowWorld for ArrowWorldView<'_> {
    /// `AbstractArrow.tick`'s stuck check: the block at the arrow's position has a collision
    /// box containing the position (`shape.toAabbs()` moved by the block).
    fn collision_contains(&self, block: BlockPos, p: DVec3) -> bool {
        let state = self.view.block_state(block);
        self.data.shapes.collision_boxes(state).iter().any(|b| {
            let b = b.offset(block.as_dvec3());
            p.cmpgt(b.min).all() && p.cmplt(b.max).all()
        })
    }

    /// `Level.clipIncludingBorder` with `ClipContext.Block.COLLIDER`: the first collision box
    /// the segment enters, walking the cells in order (the world border half of vanilla's
    /// clip is not modelled).
    fn clip(&self, from: DVec3, to: DVec3) -> Option<DVec3> {
        let delta = to - from;
        let mut cell = from.floor().as_ivec3();
        let end = to.floor().as_ivec3();
        let step = delta.signum().as_ivec3();
        let mut t_max = [f64::INFINITY; 3];
        let mut t_delta = [f64::INFINITY; 3];
        for a in 0..3 {
            if delta[a] != 0.0 {
                let next = if delta[a] > 0.0 {
                    cell[a] as f64 + 1.0
                } else {
                    cell[a] as f64
                };
                t_max[a] = (next - from[a]) / delta[a];
                t_delta[a] = 1.0 / delta[a].abs();
            }
        }
        // A segment crosses at most |dx|+|dy|+|dz|+1 cells.
        let max_cells = (end - cell).abs().element_sum() + 1;
        for _ in 0..=max_cells {
            let pos = BlockPos::new(cell.x, cell.y, cell.z);
            let state = self.view.block_state(pos);
            let mut best: Option<f64> = None;
            for b in self.data.shapes.collision_boxes(state) {
                if let Some((t, _, _)) =
                    haste_physics::raycast::clip_box(&b.offset(pos.as_dvec3()), from, delta)
                {
                    if best.is_none_or(|x| t < x) {
                        best = Some(t);
                    }
                }
            }
            if let Some(t) = best {
                return Some(from + delta * t);
            }
            // Step to the next cell on the axis whose boundary is nearest.
            let a = (0..3)
                .min_by(|&i, &j| t_max[i].total_cmp(&t_max[j]))
                .unwrap_or(0);
            if t_max[a] >= 1.0 {
                return None;
            }
            cell[a] += step[a];
            t_max[a] += t_delta[a];
        }
        None
    }

    /// `Entity.isInWater()`: the block at the arrow's position is water.
    fn in_water(&self, pos: DVec3) -> bool {
        let p = BlockPos::from_dvec3(pos);
        self.data
            .fluid(self.view.block_state(p))
            .is_some_and(|f| f.kind == haste_core::view::FluidKind::Water)
    }
}
