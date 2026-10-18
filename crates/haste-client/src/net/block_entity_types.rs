// The session's block-entity half of block updates: vanilla `LevelChunk.setBlockState` creates
// the entity when a state whose block carries one is placed (`EntityBlock.newBlockEntity`), and
// the server sends no `block_entity_data` for a plain placement — only when there is NBT to
// load — so without this the placed chest, sign or spawner never draws (its block has no model
// of its own). Which block carries which type comes from the `block_entity_types.json` oracle
// dump, resolved per state at join (`app::block_entity_setup`).

use std::collections::HashMap;

use haste_protocol::haste_core::chunk::BlockEntityData;
use haste_protocol::haste_core::{BlockPos, BlockStateId};

use super::{Session, WorldSink};

impl Session {
    /// The block states that carry a block entity, with their `minecraft:block_entity_type`
    /// registry id (from the game data and the names the server synced); call once at join so
    /// the block updates can create block entities.
    pub fn set_block_entity_state_kinds(&mut self, kinds: HashMap<BlockStateId, u32>) {
        self.block_entity_state_kinds = Some(kinds);
    }

    /// `World::set_block` plus vanilla `LevelChunk.setBlockState`'s block entity half: a state
    /// whose block carries a block entity gets one (fresh, with empty NBT — `EntityBlock
    /// .newBlockEntity`; the server's `block_entity_data` loads into it when there is anything
    /// to send), a mismatched existing one is replaced, and the same type survives a state
    /// change (a chest turned by a neighbour keeps its items). Without this the entities of
    /// blocks placed while the client is connected never exist: the server sends no
    /// `block_entity_data` for a plain placement.
    pub(super) fn set_block_with_block_entities<W: WorldSink + ?Sized>(
        &mut self,
        world: &mut W,
        pos: BlockPos,
        state: BlockStateId,
    ) {
        world.set_block(pos, state);
        let Some(kind) = self
            .block_entity_state_kinds
            .as_ref()
            .and_then(|k| k.get(&state).copied())
        else {
            return;
        };
        if world.block_entity_at(pos).is_some_and(|be| be.kind == kind) {
            return;
        }
        world.set_block_entity(
            pos,
            BlockEntityData {
                pos,
                kind,
                nbt: Default::default(),
            },
        );
    }
}

#[cfg(test)]
mod tests;
