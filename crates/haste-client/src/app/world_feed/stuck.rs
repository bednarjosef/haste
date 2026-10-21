//! Hands the renderer this frame's stuck-in-body and shoulder state: the store's per-frame
//! `StuckState` (`EntityStore::stuck_states`) becomes the renderer's `StuckInput`, with the
//! stinger and parrot models the layers draw.

use haste_entity::{EntityStore, StuckState};
use haste_render::entity::{StuckInput, StuckModels};
use haste_render::Renderer;

/// This frame's stuck state and the renderer's copy of it (both reused between frames).
#[derive(Default)]
pub(super) struct StuckFeed {
    states: Vec<StuckState>,
    inputs: Vec<StuckInput>,
}

impl StuckFeed {
    /// Fills this frame's states from `store` at `partial`.
    pub(super) fn fill(&mut self, store: &EntityStore, partial: f32) {
        store.stuck_states(partial, &mut self.states);
    }

    /// Sets `renderer`'s stuck state from the filled states and the layer models.
    pub(super) fn set_stuck(&mut self, renderer: &mut Renderer) {
        self.inputs.clear();
        self.inputs.extend(self.states.iter().map(|s| StuckInput {
            id: s.id,
            arrows: s.arrows,
            stingers: s.stingers,
            parrots: s.parrots,
            crouching: s.crouching,
            head_yaw: s.head_yaw,
            head_pitch: s.head_pitch,
            walk_pos: s.walk_pos,
            walk_speed: s.walk_speed,
        }));
        let models = haste_entity::stuck_models();
        renderer.set_stuck(
            &self.inputs,
            StuckModels {
                stinger: models.stinger,
                parrot: models.parrot,
            },
        );
    }
}
