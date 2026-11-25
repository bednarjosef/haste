//! Vanilla `BlockLightEngine.checkNode`: the block layer's reaction to one changed block.
//! The propagation itself is shared (`engine.rs`), shape occlusion included.

use haste_core::view::BlockPhysics;

use super::engine::{decrease_all_directions, increase_light_from_emission, Queues, PULL_LIGHT_IN};
use super::storage::{Layer, Storage};

/// Queues the light updates for a changed block, vanilla `BlockLightEngine.checkNode`:
/// a node whose emission dropped below its stored level is darkened (and its light
/// Decreased from every neighbour that lit it), a node that still emits re-seeds from its
/// own emission, and every other node pulls its light back in from its neighbours.
pub(super) fn check_node(store: &mut Storage<'_>, queues: &mut Queues, node: u64) {
    if !store.storing(Layer::Block, node) {
        return;
    }
    let state = store.state_at(node);
    let emission = state.map_or(0, |s| store.game_data().light_emission(s));
    let old_level = store.stored_level(Layer::Block, node);
    if emission < old_level {
        store.set_stored_level(Layer::Block, node, 0);
        queues.enqueue_decrease(node, decrease_all_directions(old_level));
    } else {
        queues.enqueue_decrease(node, PULL_LIGHT_IN);
    }
    if emission > 0 {
        queues.enqueue_increase(
            node,
            increase_light_from_emission(
                emission,
                state.is_none_or(|s| store.game_data().shapes.is_empty_shape(s)),
            ),
        );
    }
}
