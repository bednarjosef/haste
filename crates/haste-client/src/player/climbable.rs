//! Which blocks around the player are climbable (vanilla `LivingEntity.onClimbable`): answered
//! from the server-synced block tags (`update_tags`) and the block-state table, never from a
//! hard-coded block list, so modded climbables climb too. Until the server syncs the tags
//! nothing is climbable, like `haste_physics::NoClimbables`.
use std::collections::HashSet;

use haste_core::{BlockStateId, Identifier};
use haste_data::BlockStates;
use haste_physics::mining::BlockTags;

use haste_physics::ClimbableBlocks;

/// The climbable identity of the world's blocks, from the server's own data.
pub struct SyncedClimbables<'a> {
    /// The block-state table (block ids, state properties); `None` without game data.
    blocks: Option<&'a BlockStates>,
    /// The members of `#minecraft:climbable`, `#minecraft:trapdoors` and
    /// `#minecraft:can_glide_through` (`None` before `update_tags` arrived), resolved once so
    /// the per-block lookups build no identifiers.
    climbable: Option<&'a HashSet<u32>>,
    trapdoors: Option<&'a HashSet<u32>>,
    can_glide_through: Option<&'a HashSet<u32>>,
}

impl<'a> SyncedClimbables<'a> {
    /// From the tags the server synced and the block-state table. Missing either means nothing
    /// is climbable (no tags yet, or a world view without game data).
    pub fn new(tags: Option<&'a BlockTags>, blocks: Option<&'a BlockStates>) -> Self {
        let members = |tag: &str| tags.and_then(|t| t.members(&Identifier::minecraft(tag)));
        Self {
            blocks,
            climbable: members("climbable"),
            trapdoors: members("trapdoors"),
            can_glide_through: members("can_glide_through"),
        }
    }

    /// Whether `state`'s block is in the resolved tag `members`.
    fn in_tag(&self, state: BlockStateId, members: Option<&HashSet<u32>>) -> bool {
        let (Some(members), Some(blocks)) = (members, self.blocks) else {
            return false;
        };
        let block = blocks.block_of(state);
        blocks
            .block_index(block)
            .is_some_and(|index| members.contains(&index.0))
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
        self.in_tag(state, self.climbable)
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
        self.in_tag(state, self.trapdoors)
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
        self.in_tag(state, self.can_glide_through)
    }
}

#[cfg(test)]
mod tests;
