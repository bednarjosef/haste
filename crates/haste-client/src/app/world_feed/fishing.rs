//! The fishing bobbers' feed (vanilla `FishingHookRenderer`): each frame the bobbers in the entity
//! store are collected at the partial tick, lit at the block they float in like any entity, and
//! handed to the renderer with their lines. The lists are reused, so a frame allocates nothing.

use haste_core::view::BlockView as _;
use haste_entity::fishing_bobber::{self, FishingBobber};
use haste_entity::EntityStore;
use haste_render::entity::{BobberOwner, FishingBobber as Draw};
use haste_render::Renderer;
use haste_world::World;

/// This frame's bobbers from the store, and the same bobbers as the renderer draws them.
#[derive(Default)]
pub(super) struct FishingFeed {
    bobbers: Vec<FishingBobber>,
    draws: Vec<Draw>,
}

impl FishingFeed {
    /// Collects the store's bobbers at `partial` (`fishing_bobber::bobbers`).
    pub(super) fn collect(&mut self, store: &EntityStore, partial: f32) {
        fishing_bobber::bobbers(store, partial, &mut self.bobbers);
    }

    /// Lights each bobber at its block (the sky and block light there, as the entities are
    /// lit) and sets the renderer's bobbers for this frame.
    pub(super) fn light_and_set(&mut self, world: &World, renderer: &mut Renderer) {
        self.draws.clear();
        for b in &self.bobbers {
            let p = haste_core::BlockPos::new(
                b.pos.x.floor() as i32,
                b.pos.y.floor() as i32,
                b.pos.z.floor() as i32,
            );
            self.draws.push(Draw {
                pos: b.pos,
                light: (world.sky_light(p), world.block_light(p)),
                line: b.line,
                owner: b.owner.map(|o| BobberOwner {
                    id: o.id,
                    holding_right: o.holding_right,
                }),
            });
        }
        renderer.set_fishing_bobbers(&self.draws);
    }
}
