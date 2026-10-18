//! The world a dropped item's client-side tick reads (`ItemEntity.tick`, simulated on the client
//! because its move simulation type is `SERVER_AND_CLIENT`): the friction of the block under it,
//! the fluid it is in and the block collision of its `move`, answered from the loaded chunks and
//! the game data.

use haste_core::view::{BlockPhysics, BlockView, FluidKind};
use haste_core::{Aabb, BlockPos, DVec3};
use haste_data::GameData;
use haste_entity::item_entity::ItemWorld;

/// `ItemEntity`'s dimensions (`EntityType.ITEM` is 0.25 x 0.25).
const ITEM_SIZE: f64 = 0.25;
/// `ItemEntity.getBlockPosBelowThatAffectsMyMovement`: `getOnPos(0.999999F)`.
const ON_POS_OFFSET: f64 = 0.999_999_f32 as f64;
/// `Entity.getFluidInteractionBox`: the bounding box deflated by 0.001.
const FLUID_DEFLATE: f64 = 0.001;

/// The client's world and game data as the item tick's [`ItemWorld`].
pub(super) struct ItemWorldView<'a> {
    pub view: &'a dyn BlockView,
    pub data: &'a GameData,
}

impl ItemWorld for ItemWorldView<'_> {
    fn friction_below(&self, pos: DVec3) -> f32 {
        let below = BlockPos::new(
            pos.x.floor() as i32,
            (pos.y - ON_POS_OFFSET).floor() as i32,
            pos.z.floor() as i32,
        );
        self.data.friction(self.view.block_state(below))
    }

    /// `Entity.getFluidHeight` (`EntityFluidInteraction.update`): the highest surface of `kind`
    /// among the cells the fluid interaction box touches, measured from the item's feet.
    fn fluid_height(&self, pos: DVec3, kind: FluidKind) -> f32 {
        let b = Aabb::entity(pos, ITEM_SIZE, ITEM_SIZE);
        let min = b.min + DVec3::splat(FLUID_DEFLATE);
        let max = b.max - DVec3::splat(FLUID_DEFLATE);
        let mut height = 0.0f64;
        for x in min.x.floor() as i32..=(max.x.ceil() as i32 - 1) {
            for z in min.z.floor() as i32..=(max.z.ceil() as i32 - 1) {
                for y in min.y.floor() as i32..=(max.y.ceil() as i32 - 1) {
                    let p = BlockPos::new(x, y, z);
                    let Some(f) = self.data.fluid(self.view.block_state(p)) else {
                        continue;
                    };
                    if f.kind != kind {
                        continue;
                    }
                    // `FluidState.getHeight`: a full block under the same fluid.
                    let full = self
                        .data
                        .fluid(self.view.block_state(p.add(0, 1, 0)))
                        .is_some_and(|a| a.kind == kind);
                    let surface = y as f64 + if full { 1.0 } else { f.height as f64 };
                    if surface >= min.y {
                        height = height.max(surface - b.min.y);
                    }
                }
            }
        }
        height as f32
    }

    /// `Entity.move(MoverType.SELF, ...)`: the block collision sweep, and `onGround` when the
    /// downward move was clipped (`verticalCollision && movement.y < 0`).
    fn move_with_collision(&self, pos: DVec3, delta: DVec3, on_ground: &mut bool) -> DVec3 {
        let aabb = Aabb::entity(pos, ITEM_SIZE, ITEM_SIZE);
        let moved = haste_physics::collision::collide(aabb, delta, self.view, &self.data.shapes);
        *on_ground = delta.y < 0.0 && moved.y != delta.y;
        moved
    }
}
