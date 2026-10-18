//! Which blocks around the player are climbable (vanilla `LivingEntity.onClimbable`): answered
//! from the server-synced block tags (`update_tags`) and the block-state table, never from a
//! hard-coded block list, so modded climbables climb too. Until the server syncs the tags
//! nothing is climbable, like `haste_physics::NoClimbables`.
use haste_core::{BlockStateId, Identifier};
use haste_data::BlockStates;
use haste_physics::mining::BlockTags;

use haste_physics::ClimbableBlocks;

/// The climbable identity of the world's blocks, from the server's own data.
pub struct SyncedClimbables<'a> {
    /// The server-synced block tags; `None` before `update_tags` arrived.
    tags: Option<&'a BlockTags>,
    /// The block-state table (block ids, state properties); `None` without game data.
    blocks: Option<&'a BlockStates>,
}

impl<'a> SyncedClimbables<'a> {
    /// From the tags the server synced and the block-state table. Missing either means nothing
    /// is climbable (no tags yet, or a world view without game data).
    pub fn new(tags: Option<&'a BlockTags>, blocks: Option<&'a BlockStates>) -> Self {
        Self { tags, blocks }
    }

    /// Whether `state`'s block is in `#minecraft:<tag>`.
    fn in_tag(&self, state: BlockStateId, tag: &str) -> bool {
        let (Some(tags), Some(blocks)) = (self.tags, self.blocks) else {
            return false;
        };
        let block = blocks.block_of(state);
        blocks
            .block_index(block)
            .is_some_and(|index| tags.contains(&Identifier::minecraft(tag), index.0))
    }

    /// Whether `state`'s block is `minecraft:<name>`.
    fn is_block(&self, state: BlockStateId, name: &str) -> bool {
        self.blocks.is_some_and(|blocks| {
            let block = blocks.block_of(state);
            block.namespace() == "minecraft" && block.path() == name
        })
    }
}

impl ClimbableBlocks for SyncedClimbables<'_> {
    /// `BlockTags.CLIMBABLE` (`#minecraft:climbable`).
    fn is_climbable(&self, state: BlockStateId) -> bool {
        self.in_tag(state, "climbable")
    }

    /// `Blocks.SCAFFOLDING` (`minecraft:scaffolding`): sneaking slides down it.
    fn is_scaffolding(&self, state: BlockStateId) -> bool {
        self.is_block(state, "scaffolding")
    }

    /// Vanilla `LivingEntity.trapdoorUsableAsLadder`: an open trapdoor over a ladder facing the
    /// same way. `#minecraft:trapdoors` decides which blocks count as trapdoors (vanilla's
    /// `TrapDoorBlock` check; the tag covers modded trapdoors), the `open` and `facing`
    /// properties come from the state table, and a state without them does not count.
    fn trapdoor_usable_as_ladder(&self, state: BlockStateId, below: BlockStateId) -> bool {
        let Some(blocks) = self.blocks else {
            return false;
        };
        self.in_tag(state, "trapdoors")
            && blocks.property_value(state, "open") == Some("true")
            && self.is_block(below, "ladder")
            && matches!(
                (
                    blocks.property_value(state, "facing"),
                    blocks.property_value(below, "facing"),
                ),
                (Some(a), Some(b)) if a == b
            )
    }

    /// `BlockTags.CAN_GLIDE_THROUGH` (`#minecraft:can_glide_through`): a glider passes through
    /// these climbables instead of being stopped.
    fn can_glide_through(&self, state: BlockStateId) -> bool {
        self.in_tag(state, "can_glide_through")
    }
}

#[cfg(test)]
mod tests;
