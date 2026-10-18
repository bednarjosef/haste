// The session's side of vanilla's `block_event` routing (`ClientLevel.blockEvent` ->
// `BlockEntity.triggerEvent`): which block states take which events, and the events waiting
// for the game thread's block entity tickers.

use std::collections::HashSet;

use haste_protocol::haste_core::{BlockPos, BlockStateId, Direction};

use super::Session;

impl Session {
    /// The block states that carry a lid (chest family, shulker boxes), from the game data;
    /// call once at join so `block_event` can tell lid events from other blocks' (pistons use
    /// the same action id).
    pub fn set_lid_states(&mut self, states: HashSet<BlockStateId>) {
        self.lid_states = Some(states);
    }

    /// The bell block states, for `block_event` action 1's ring events.
    pub fn set_bell_states(&mut self, states: HashSet<BlockStateId>) {
        self.bell_states = Some(states);
    }

    /// The decorated pot block states, for `block_event` action 1's wobble events.
    pub fn set_pot_states(&mut self, states: HashSet<BlockStateId>) {
        self.pot_states = Some(states);
    }

    /// The bell rings and decorated pot wobbles waiting to be drawn, as (position, click
    /// direction) and (position, start tick, duration) pairs. The game thread turns them into
    /// the animation state the renderer interpolates.
    pub fn block_event_anims(
        &self,
        bells: &mut Vec<(BlockPos, Option<Direction>)>,
        pots: &mut Vec<(BlockPos, i64, u32)>,
    ) {
        bells.extend(self.bells.iter().map(|(p, d)| (*p, *d)));
        pots.extend(self.pots.iter().map(|(p, w)| (*p, w.0, w.1)));
    }
}

/// One block entity's lid animation (vanilla `ChestLidController` for chests and ender chests,
/// `ShulkerBoxBlockEntity.updateAnimation`'s progress for shulker boxes: both step their value
/// by 0.1 per tick towards the open target). `old` is the value at the start of the last tick,
/// for the partial-tick lerp.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LidAnim {
    pub old: f32,
    pub value: f32,
    pub open: bool,
}

impl LidAnim {
    /// One 20 Hz tick (vanilla `ChestLidController.tickLid`: 0.1 steps, clamped at 0 and 1).
    pub(super) fn tick(&mut self) {
        self.old = self.value;
        let target = if self.open { 1.0 } else { 0.0 };
        if self.value < target {
            self.value = (self.value + 0.1).min(target);
        } else if self.value > target {
            self.value = (self.value - 0.1).max(target);
        }
    }

    /// Whether the lid is at rest closed (nothing left to animate, no need to copy it out).
    pub(super) fn at_rest_closed(&self) -> bool {
        !self.open && self.value == 0.0
    }
}
