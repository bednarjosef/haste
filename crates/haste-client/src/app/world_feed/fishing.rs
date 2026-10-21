//! The fishing bobbers' feed (vanilla `FishingHookRenderer`): each frame the bobbers in the entity
//! store are collected at the partial tick, lit at the block they float in like any entity, and
//! handed to the renderer with their lines. The lists are reused, so a frame allocates nothing.

use crate::game::{self, Shared};
use haste_core::view::BlockView as _;
use haste_core::{DVec3, EntityId};
use haste_entity::fishing_bobber::{self, FirstPersonHand, FishingBobber};
use haste_entity::fishing_hook::{swing_factor, NearPlane};
use haste_entity::EntityStore;
use haste_render::entity::{BobberOwner, FishingBobber as Draw};
use haste_render::{Camera, Renderer};
use haste_world::World;

/// `Camera.getNearPlane`'s near distance (vanilla's `Camera` near plane).
const Z_NEAR: f64 = 0.05;

/// The local player's first-person hand from the frame's camera (the eye, yaw and pitch, FOV and
/// aspect). The camera's FOV is the frame's, with its modifiers; vanilla reads the option.
pub(super) fn first_person_hand(
    camera: &Camera,
    view: &game::ViewState,
    partial: f32,
    renderer: &Renderer,
) -> FirstPersonHand {
    let (width, height) = renderer.size();
    let yaw = f64::from(camera.yaw.to_radians());
    let pitch = f64::from(camera.pitch.to_radians());
    let forward = DVec3::new(
        -yaw.sin() * pitch.cos(),
        -pitch.sin(),
        yaw.cos() * pitch.cos(),
    );
    let up = DVec3::new(
        -yaw.sin() * pitch.sin(),
        pitch.cos(),
        yaw.cos() * pitch.sin(),
    );
    let left = up.cross(forward);
    let aspect = f64::from(width) / f64::from(height.max(1));
    let (from, to) = view.first_person.map_or((0.0, 0.0), |fp| fp.attack);
    FirstPersonHand {
        eye: camera.pos,
        near: NearPlane::new(forward, up, left, camera.fov_deg, aspect, Z_NEAR),
        fov: camera.fov_deg,
        swing: swing_factor(from + (to - from) * partial),
    }
}

/// This frame's bobbers from the store, and the same bobbers as the renderer draws them.
#[derive(Default)]
pub(super) struct FishingFeed {
    bobbers: Vec<FishingBobber>,
    draws: Vec<Draw>,
}

impl FishingFeed {
    /// Collects the store's bobbers at `partial` (`fishing_bobber::bobbers`), with the local
    /// player's model (locked here, under the store's read lock) as the owner of our own bobber.
    /// `first_person` is the frame's camera and view in first person (`None` otherwise): our
    /// own line's hand then comes from them (`first_person_hand`), with the window's size.
    pub(super) fn collect(
        &mut self,
        store: &EntityStore,
        shared: &Shared,
        local: EntityId,
        partial: f32,
        first_person: Option<(Camera, &game::ViewState)>,
        renderer: &Renderer,
    ) {
        let hand = first_person.map(|(c, view)| first_person_hand(&c, view, partial, renderer));
        let model = shared.local_model.lock();
        let local = (local, &*model);
        fishing_bobber::bobbers(store, local, hand.as_ref(), partial, &mut self.bobbers);
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
