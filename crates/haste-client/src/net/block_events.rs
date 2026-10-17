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
