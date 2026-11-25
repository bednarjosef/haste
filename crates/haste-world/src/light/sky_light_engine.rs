//! Vanilla `SkyLightEngine.checkNode` and its column source updates: the sky layer's
//! reaction to one changed block. Sky light of 15 is not propagated like block light —
//! cells at or above their column's lowest source y *are* sources; the walks below move
//! those sources when a block change occludes or opens a column, and the queue machinery
//! (`engine.rs`) spreads the light sideways (at one less per step, like block light).
//!
//! Vanilla's chunk-border special case for light crossing into columns with absent layers
//! (`propagateFromEmptySections`) is not ported: our absent sections read through the same
//! flattening walk as vanilla's, and the case needs a horizontal chunk border at the bottom
//! row of a section above a gap of absent layers.

use haste_core::Direction;

use super::engine::{
    decrease_all_directions, decrease_skip_one_direction, increase_skip_one_direction, pack,
    unpack_x, unpack_y, unpack_z, Queues, PULL_LIGHT_IN,
};
use super::sky_sources::{self, OPEN_TO_BOTTOM};
use super::storage::{Layer, Storage};

/// Vanilla `REMOVE_SKY_SOURCE_ENTRY`: a level-15 decrease that does not go up (the column
/// above is still a source).
const REMOVE_SKY_SOURCE: u32 = decrease_skip_one_direction(15, Direction::Up);
/// Vanilla `REMOVE_TOP_SKY_SOURCE_ENTRY`: the topmost removed source, darkened from every
/// direction.
const REMOVE_TOP_SKY_SOURCE: u32 = decrease_all_directions(15);
/// Vanilla `ADD_SKY_SOURCE_ENTRY`: a level-15 increase that does not go up (vanilla
/// records an empty-shape flag of false here: the source cell's state is read back when
/// the shape test needs it).
const ADD_SKY_SOURCE: u32 = increase_skip_one_direction(15, false, Direction::Up);

/// Vanilla `SkyLightEngine.checkNode`.
pub(super) fn check_node(store: &mut Storage<'_>, queues: &mut Queues, node: u64) {
    let (x, y, z) = (unpack_x(node), unpack_y(node), unpack_z(node));
    // skylight is on (the dimension has it): the column's lowest source y, else "none"
    let lowest = if store.has_skylight() {
        sky_sources::lowest_source_y(store.world(), x, z)
    } else {
        i32::MAX
    };
    if lowest != i32::MAX {
        update_sources_in_column(store, queues, x, z, lowest);
    }
    if !store.storing(Layer::Sky, node) {
        return;
    }
    if y >= lowest {
        // still a source: re-seed it (decrease what lit from it, then add it back)
        queues.enqueue_decrease(node, REMOVE_SKY_SOURCE);
        queues.enqueue_increase(node, ADD_SKY_SOURCE);
    } else {
        let old_level = store.stored_level(Layer::Sky, node);
        if old_level > 0 {
            store.set_stored_level(Layer::Sky, node, 0);
            queues.enqueue_decrease(node, decrease_all_directions(old_level));
        } else {
            queues.enqueue_decrease(node, PULL_LIGHT_IN);
        }
    }
}

/// Vanilla `updateSourcesInColumn`: remove the sources below the new lowest source y, add
/// the ones above it.
fn update_sources_in_column(
    store: &mut Storage<'_>,
    queues: &mut Queues,
    x: i32,
    z: i32,
    lowest: i32,
) {
    let world_bottom = store.dimension().min_y;
    remove_sources_below(store, queues, x, z, lowest, world_bottom);
    add_sources_above(store, queues, x, z, lowest, world_bottom);
}

/// The world's section span, as `(min_section, max_section_exclusive)`.
fn section_span(store: &Storage<'_>) -> (i32, i32) {
    let dim = store.dimension();
    let min = dim.min_section();
    (min, min + dim.section_count() as i32)
}

/// Vanilla `removeSourcesBelow`: zeroes every source cell from `lowest - 1` downwards
/// until the column stops being lit, queueing decreases that pull the light the removed
/// sources spread. The topmost removed cell darkens in every direction; the rest do not
/// darken upwards (the column above them is still a source).
fn remove_sources_below(
    store: &mut Storage<'_>,
    queues: &mut Queues,
    x: i32,
    z: i32,
    lowest: i32,
    world_bottom: i32,
) {
    if lowest <= world_bottom {
        return;
    }
    let start_y = lowest - 1;
    let (min_section, _) = section_span(store);
    let mut section_y = start_y >> 4;
    while section_y >= min_section {
        if store.storing(Layer::Sky, pack(x, section_y << 4, z)) {
            let section_bottom = section_y << 4;
            let section_top = section_bottom + 15;
            let mut y = section_top.min(start_y);
            while y >= section_bottom {
                let node = pack(x, y, z);
                if store.stored_level(Layer::Sky, node) != 15 {
                    return;
                }
                store.set_stored_level(Layer::Sky, node, 0);
                queues.enqueue_decrease(
                    node,
                    if y == start_y {
                        REMOVE_TOP_SKY_SOURCE
                    } else {
                        REMOVE_SKY_SOURCE
                    },
                );
                y -= 1;
            }
        }
        section_y -= 1;
    }
}

/// Vanilla `addSourcesAbove`: sets every cell from `max(lowest, world_bottom)` upwards to
/// 15 until the column is already lit, queueing increases only where the light can escape
/// sideways (below all four neighbour columns' lowest sources, or the lowest cell itself).
fn add_sources_above(
    store: &mut Storage<'_>,
    queues: &mut Queues,
    x: i32,
    z: i32,
    lowest: i32,
    world_bottom: i32,
) {
    let neighbor_lowest = [
        sky_sources::lowest_source_y(store.world(), x - 1, z),
        sky_sources::lowest_source_y(store.world(), x + 1, z),
        sky_sources::lowest_source_y(store.world(), x, z - 1),
        sky_sources::lowest_source_y(store.world(), x, z + 1),
    ]
    .into_iter()
    .max()
    .unwrap_or(OPEN_TO_BOTTOM);
    let start_y = lowest.max(world_bottom);
    let (_, max_section) = section_span(store);
    let mut section_y = start_y >> 4;
    while section_y < max_section {
        if store.storing(Layer::Sky, pack(x, section_y << 4, z)) {
            let section_bottom = section_y << 4;
            let section_top = section_bottom + 15;
            let mut y = section_bottom.max(start_y);
            while y <= section_top {
                let node = pack(x, y, z);
                if store.stored_level(Layer::Sky, node) == 15 {
                    return;
                }
                store.set_stored_level(Layer::Sky, node, 15);
                if y < neighbor_lowest || y == lowest {
                    queues.enqueue_increase(node, ADD_SKY_SOURCE);
                }
                y += 1;
            }
        }
        section_y += 1;
    }
}
