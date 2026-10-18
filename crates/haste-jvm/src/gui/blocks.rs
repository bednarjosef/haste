//! Block states and block entities the Java-side screens see.

use super::GuiPipeline;
use crate::proxies::with_state;
use haste_core::BlockPos;

impl GuiPipeline {
    /// Records the world's block state at a position (Java's global block state id): the block
    /// entities the screens edit are built from the real state, like vanilla's chunks build them
    /// (`LevelChunk.setBlockState`). The client records every position whose entity packets it
    /// forwards, before forwarding them (the native must not take the world's lock: it can run
    /// while the game thread holds it).
    pub fn set_block_state(&self, pos: BlockPos, id: i32) {
        with_state(|s| {
            s.block_states.insert([pos.x, pos.y, pos.z], id);
        });
    }

    /// A chunk was forgotten (vanilla `handleForgetLevelChunk` drops the chunk's block entities,
    /// which the forwarded packet does for the Java-side store): the block states recorded for
    /// its positions go with them, so the map does not grow without bound.
    pub fn forget_recorded_block_states(&self, chunk_x: i32, chunk_z: i32) {
        with_state(|s| {
            s.block_states
                .retain(|[x, _, z], _| (*x >> 4) != chunk_x || (*z >> 4) != chunk_z);
        });
    }

    /// Number of block entities in the Java-side store (tests).
    pub fn block_entity_count(&self) -> usize {
        self.call_int("blockEntityCount", self.m.block_entity_count, &[]) as usize
    }
}
