// The session's block-entity setup, filled once at join from the game data and the registries
// the server synced: the `minecraft:block_entity_type` names by id, the block states whose
// `block_event` action 1 drives a lid, bell ring or pot wobble, and the block states that
// carry a block entity (so a block update that places one can create it client-side, like
// vanilla `LevelChunk.setBlockState`).

use std::collections::{HashMap, HashSet};

use haste_protocol::haste_core::{BlockStateId, Identifier, RegistryIndex};
use haste_protocol::SyncedRegistries;

/// `minecraft:block_entity_type` names by synced registry id (stored block entities carry that
/// id); the static registry from the data generator when the server did not sync one, whose
/// ids are the same on vanilla servers.
pub(crate) fn block_entity_type_names(
    regs: &SyncedRegistries,
    data: &haste_data::GameData,
) -> Vec<Identifier> {
    let registry = Identifier::minecraft("block_entity_type");
    if let Some(entries) = regs.entries(&registry) {
        return entries.iter().map(|e| e.id.clone()).collect();
    }
    let n = data.registries.len("minecraft:block_entity_type");
    (0..n as u32)
        .map(|i| {
            data.registries
                .name("minecraft:block_entity_type", RegistryIndex(i))
                .cloned()
                .unwrap_or_else(|| Identifier::minecraft("air"))
        })
        .collect()
}

/// The block states whose `block_event` action 1 drives a lid (chests, ender chests, trapped
/// chests, shulker boxes; copper chests are not drawn yet), for the session's event filter.
pub(crate) fn lid_block_states(data: &haste_data::GameData) -> HashSet<BlockStateId> {
    block_states_where(data, haste_render::blockentity::is_lid_block)
}

/// Every block state whose block matches `f`, for the session's `block_event` filters.
pub(crate) fn block_states_where(
    data: &haste_data::GameData,
    f: impl Fn(&Identifier) -> bool,
) -> HashSet<BlockStateId> {
    (0..data.blocks.len() as u32)
        .map(BlockStateId)
        .filter(|s| f(data.blocks.block_of(*s)))
        .collect()
}

/// Each block state whose block carries a block entity, with the block's
/// `minecraft:block_entity_type` registry id (the `block_entity_types.json` oracle dump —
/// vanilla `EntityBlock.newBlockEntity`'s answer per state — resolved through the names the
/// server synced): the block updates that place such blocks create their block entities
/// client-side, like vanilla `LevelChunk.setBlockState` does — the server sends no
/// `block_entity_data` for a plain placement. Without the dump no state carries one (the
/// server's `block_entity_data` still works).
pub(crate) fn block_entity_state_kinds(
    data: &haste_data::GameData,
    names: &[Identifier],
) -> HashMap<BlockStateId, u32> {
    if !data.block_entity_types.is_loaded() {
        tracing::warn!(
            "no block_entity_types.json oracle dump (tools/recorder-mod/dump-server-oracles.sh): \
             blocks placed by block updates get no client-side block entity"
        );
        return HashMap::new();
    }
    let ids: HashMap<&Identifier, u32> = names
        .iter()
        .enumerate()
        .map(|(id, n)| (n, id as u32))
        .collect();
    (0..data.blocks.len() as u32)
        .map(BlockStateId)
        .filter_map(|s| {
            let ty = data
                .block_entity_types
                .type_of(data.blocks.block_of(s), s)?;
            Some((s, *ids.get(ty)?))
        })
        .collect()
}

#[cfg(test)]
mod tests;
