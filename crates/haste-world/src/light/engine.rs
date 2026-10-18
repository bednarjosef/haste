//! The propagation queues shared by both light layers: vanilla `LightEngine`'s increase and
//! decrease machinery (`runLightUpdates` / `propagateIncreases` / `propagateDecreases`) and
//! its `QueueEntry` encoding.
//!
//! A queue entry is a packed `(level, direction mask)` pair with the same bit layout as
//! vanilla `LightEngine.QueueEntry`: level in bits 0..4, one direction bit per direction
//! (bit 4 + the direction's ordinal), a "from empty shape" flag at bit 10 (the spreading
//! state fails `LightEngine.isEmptyShape`, so its face occlusion shape is empty and the
//! increase never needs to read it back) and an "increase from emission" flag at bit 11.
//!
//! Positions are packed like vanilla `BlockPos.asLong` (x 26 bits at 38, z 26 at 12, y 12
//! at 0), enough for the world's ±30 M horizontal range.

use haste_core::view::BlockPhysics;
use haste_core::{BlockStateId, Direction};

use super::storage::{light_dampening, Layer, Storage};

/// Vanilla `PULL_LIGHT_IN_ENTRY`: a level-1 decrease from every direction, which never
/// removes anything but re-seeds an increase from every lit neighbour of the node.
pub(super) const PULL_LIGHT_IN: u32 = entry_level(ALL_DIRECTIONS, 1);
/// All six direction bits.
const ALL_DIRECTIONS: u32 = 0b11_1111_0000;
/// Vanilla `FLAG_FROM_EMPTY_SHAPE`: the spreading state's face occlusion shape is empty.
const FROM_EMPTY_SHAPE: u32 = 1 << 10;
/// Vanilla `FLAG_INCREASE_FROM_EMISSION`.
const FROM_EMISSION: u32 = 1 << 11;

#[inline]
const fn with_direction(entry: u32, d: Direction) -> u32 {
    entry | 1 << (d as u8 + 4)
}

#[inline]
const fn without_direction(entry: u32, d: Direction) -> u32 {
    entry & !(1 << (d as u8 + 4))
}

#[inline]
const fn entry_level(directions: u32, level: u8) -> u32 {
    directions & !0xF | level as u32 & 0xF
}

/// A decrease entry carrying `level` in every direction but `skip`
/// (vanilla `QueueEntry.decreaseSkipOneDirection`).
pub(super) const fn decrease_skip_one_direction(level: u8, skip: Direction) -> u32 {
    entry_level(without_direction(ALL_DIRECTIONS, skip), level)
}

/// A decrease entry carrying `level` in every direction
/// (vanilla `QueueEntry.decreaseAllDirections`).
pub(super) const fn decrease_all_directions(level: u8) -> u32 {
    entry_level(ALL_DIRECTIONS, level)
}

/// An increase that raises the node itself to `level` before spreading
/// (vanilla `QueueEntry.increaseLightFromEmission`).
pub(super) const fn increase_light_from_emission(level: u8, from_empty_shape: bool) -> u32 {
    entry_level(ALL_DIRECTIONS, level) | FROM_EMISSION
        | if from_empty_shape {
            FROM_EMPTY_SHAPE
        } else {
            0
        }
}

/// An increase of `level` in every direction but `skip`
/// (vanilla `QueueEntry.increaseSkipOneDirection`).
pub(super) const fn increase_skip_one_direction(
    level: u8,
    from_empty_shape: bool,
    skip: Direction,
) -> u32 {
    entry_level(
        without_direction(ALL_DIRECTIONS, skip)
            | if from_empty_shape {
                FROM_EMPTY_SHAPE
            } else {
                0
            },
        level,
    )
}

/// An increase of `level` in only `direction`
/// (vanilla `QueueEntry.increaseOnlyOneDirection`).
pub(super) const fn increase_only_one_direction(
    level: u8,
    from_empty_shape: bool,
    direction: Direction,
) -> u32 {
    entry_level(
        with_direction(
            if from_empty_shape {
                FROM_EMPTY_SHAPE
            } else {
                0
            },
            direction,
        ),
        level,
    )
}

#[inline]
pub(super) fn entry_of(entry: u32) -> u8 {
    (entry & 0xF) as u8
}

#[inline]
fn is_increase_from_emission(entry: u32) -> bool {
    entry & FROM_EMISSION != 0
}

#[inline]
fn is_from_empty_shape(entry: u32) -> bool {
    entry & FROM_EMPTY_SHAPE != 0
}

#[inline]
fn should_propagate_in_direction(entry: u32, d: Direction) -> bool {
    entry & (1 << (d as u8 + 4)) != 0
}

/// Packs a block position (vanilla `BlockPos.asLong`).
#[inline]
pub(super) fn pack(x: i32, y: i32, z: i32) -> u64 {
    ((x as u64 & 0x3FF_FFFF) << 38) | ((z as u64 & 0x3FF_FFFF) << 12) | (y as u64 & 0xFFF)
}

#[inline]
pub(super) fn unpack_x(p: u64) -> i32 {
    sign_extend(p >> 38, 26)
}

#[inline]
pub(super) fn unpack_y(p: u64) -> i32 {
    sign_extend(p & 0xFFF, 12)
}

#[inline]
pub(super) fn unpack_z(p: u64) -> i32 {
    sign_extend((p >> 12) & 0x3FF_FFFF, 26)
}

#[inline]
fn sign_extend(v: u64, bits: u32) -> i32 {
    (((v << (64 - bits)) as i64) >> (64 - bits)) as i32
}

/// `p` shifted one block along `d` (vanilla `BlockPos.offset`).
#[inline]
pub(super) fn offset(p: u64, d: Direction) -> u64 {
    let o = d.offset();
    pack(unpack_x(p) + o.x, unpack_y(p) + o.y, unpack_z(p) + o.z)
}

/// The increase and decrease queues of one layer, vanilla `LightEngine`'s two
/// `LongArrayFIFOQueue`s. Entries are `(node, entry)` pairs.
pub(super) struct Queues {
    decrease: std::collections::VecDeque<(u64, u32)>,
    increase: std::collections::VecDeque<(u64, u32)>,
}

impl Queues {
    pub(super) fn new() -> Queues {
        Queues {
            decrease: std::collections::VecDeque::new(),
            increase: std::collections::VecDeque::new(),
        }
    }

    pub(super) fn enqueue_decrease(&mut self, node: u64, entry: u32) {
        self.decrease.push_back((node, entry));
    }

    pub(super) fn enqueue_increase(&mut self, node: u64, entry: u32) {
        self.increase.push_back((node, entry));
    }
}

/// Vanilla `LightEngine.runLightUpdates`'s queue phase (the checked nodes were already
/// queued by the layer's `check_node`): all decreases run to completion, then all
/// increases — so every removal happens before any re-seeded light spreads.
pub(super) fn run(store: &mut Storage<'_>, queues: &mut Queues, layer: Layer) {
    while let Some((from, entry)) = queues.decrease.pop_front() {
        propagate_decrease(store, queues, layer, from, entry);
    }
    while let Some((from, entry)) = queues.increase.pop_front() {
        let mut from_level = store.stored_level(layer, from);
        let target = entry_of(entry);
        if is_increase_from_emission(entry) && from_level < target {
            store.set_stored_level(layer, from, target);
            from_level = target;
        }
        if from_level == target {
            propagate_increase(store, queues, layer, from, entry, from_level);
        }
    }
}

/// Vanilla `LightEngine.propagateIncrease` (both layers share it), with vanilla's
/// `shapeOccludes` test: light crossing from a shape-occluding state's face is blocked
/// when the two face occlusion shapes cover the shared plane (a slab's bottom face, a
/// stair's solid side). False for every state on dumps without the flags, which is the
/// pre-shape behaviour.
fn propagate_increase(
    store: &mut Storage<'_>,
    queues: &mut Queues,
    layer: Layer,
    from: u64,
    entry: u32,
    from_level: u8,
) {
    let from_empty_shape = is_from_empty_shape(entry);
    for d in Direction::ALL {
        if !should_propagate_in_direction(entry, d) {
            continue;
        }
        let to = offset(from, d);
        if !store.storing(layer, to) {
            continue;
        }
        let to_level = store.stored_level(layer, to);
        if from_level.saturating_sub(1) > to_level {
            let to_state = store.state_at(to);
            let opacity = match to_state {
                Some(s) => light_dampening(store.game_data(), s).max(1),
                None => 15,
            };
            let new_to_level = from_level.saturating_sub(opacity);
            if new_to_level > to_level
                && !shape_occludes(store, from, from_empty_shape, to_state, d)
            {
                store.set_stored_level(layer, to, new_to_level);
                if new_to_level > 1 {
                    queues.enqueue_increase(
                        to,
                        increase_skip_one_direction(
                            new_to_level,
                            to_state.map_or(true, |s| store.game_data().shapes.is_empty_shape(s)),
                            d.opposite(),
                        ),
                    );
                }
            }
        }
    }
}

/// Vanilla `LightEngine.shapeOccludes(fromState, toState, direction)`: the spreading state
/// is only read back when the empty-shape flag does not already say its face shape is
/// empty (vanilla substitutes air, whose face shape is empty likewise; an unloaded chunk
/// reads bedrock there, an empty-shape state too).
fn shape_occludes(
    store: &Storage<'_>,
    from: u64,
    from_empty_shape: bool,
    to_state: Option<BlockStateId>,
    d: Direction,
) -> bool {
    let Some(to_state) = to_state else {
        return false;
    };
    let from_state = if from_empty_shape {
        BlockStateId::AIR
    } else {
        let (x, y, z) = (unpack_x(from), unpack_y(from), unpack_z(from));
        store
            .block_state(haste_core::BlockPos::new(x, y, z))
            .unwrap_or(BlockStateId::AIR)
    };
    store
        .game_data()
        .shapes
        .shape_occludes(from_state, to_state, d)
}

/// Vanilla `LightEngine.propagateDecrease`; the block layer additionally re-seeds block
/// light sources it darkens (vanilla `BlockLightEngine.propagateDecrease`' `getEmission`).
fn propagate_decrease(
    store: &mut Storage<'_>,
    queues: &mut Queues,
    layer: Layer,
    from: u64,
    entry: u32,
) {
    let old_from_level = entry_of(entry);
    for d in Direction::ALL {
        if !should_propagate_in_direction(entry, d) {
            continue;
        }
        let to = offset(from, d);
        if !store.storing(layer, to) {
            continue;
        }
        let to_level = store.stored_level(layer, to);
        if to_level == 0 {
            continue;
        }
        if to_level < old_from_level {
            // darker than what the removed light could have reached: remove it too
            store.set_stored_level(layer, to, 0);
            if layer == Layer::Block {
                let to_state = store.state_at(to);
                let emission = to_state.map_or(0, |s| store.game_data().light_emission(s));
                if emission < to_level {
                    queues
                        .enqueue_decrease(to, decrease_skip_one_direction(to_level, d.opposite()));
                }
                if emission > 0 {
                    queues.enqueue_increase(
                        to,
                        increase_light_from_emission(
                            emission,
                            to_state.map_or(true, |s| store.game_data().shapes.is_empty_shape(s)),
                        ),
                    );
                }
            } else {
                queues.enqueue_decrease(to, decrease_skip_one_direction(to_level, d.opposite()));
            }
        } else {
            // brighter: it has another source; re-seed it from here (vanilla passes an
            // empty-shape flag of false: the state is read back when the test needs it)
            queues.enqueue_increase(to, increase_only_one_direction(to_level, false, d.opposite()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_entry_round_trip() {
        // level and direction masks survive the packing; the emission and empty-shape flags
        // are readable
        let e = decrease_skip_one_direction(7, Direction::Up);
        assert_eq!(entry_of(e), 7);
        assert!(!should_propagate_in_direction(e, Direction::Up));
        assert!(should_propagate_in_direction(e, Direction::Down));
        let e = increase_only_one_direction(14, false, Direction::East);
        assert_eq!(entry_of(e), 14);
        assert!(should_propagate_in_direction(e, Direction::East));
        assert!(!should_propagate_in_direction(e, Direction::West));
        let e = increase_light_from_emission(15, false);
        assert!(is_increase_from_emission(e));
        assert!(!is_from_empty_shape(e));
        let e = increase_skip_one_direction(9, true, Direction::North);
        assert!(is_from_empty_shape(e));
        assert!(!should_propagate_in_direction(e, Direction::North));
        assert!(should_propagate_in_direction(e, Direction::South));
        assert!(!is_increase_from_emission(decrease_all_directions(15)));
        assert_eq!(entry_of(PULL_LIGHT_IN), 1);
        // all directions but the skipped one
        assert_eq!(
            (decrease_skip_one_direction(3, Direction::North) & ALL_DIRECTIONS).count_ones(),
            ALL_DIRECTIONS.count_ones() - 1
        );
    }

    #[test]
    fn positions_round_trip() {
        for &(x, y, z) in &[
            (0i32, 0i32, 0i32),
            (1, 64, 1),
            (-1, -64, -1),
            (30_000_000, 319, 30_000_000),
            (-30_000_000, -64, -30_000_000),
            (441, 147, 5),
        ] {
            let p = pack(x, y, z);
            assert_eq!(
                (unpack_x(p), unpack_y(p), unpack_z(p)),
                (x, y, z),
                "{x} {y} {z}"
            );
        }
        // offsetting moves one block along the direction
        let p = pack(10, 20, 30);
        assert_eq!(unpack_x(offset(p, Direction::West)), 9);
        assert_eq!(unpack_y(offset(p, Direction::Up)), 21);
        assert_eq!(unpack_z(offset(p, Direction::North)), 29);
    }
}
