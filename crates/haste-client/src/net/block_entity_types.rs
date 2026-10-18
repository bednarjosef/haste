// The block side of vanilla's block entity types (`BlockEntityTypes`' valid-block sets): which
// blocks carry which `minecraft:block_entity_type`. A block update that places such a block
// creates its block entity on the client (`LevelChunk.setBlockState` -> `EntityBlock
// .newBlockEntity`); the server sends no `block_entity_data` for a plain placement — only when
// there is NBT to load — so without this the placed chest, sign or spawner never draws (its
// block has no model of its own).

use std::collections::HashMap;

use haste_protocol::haste_core::chunk::BlockEntityData;
use haste_protocol::haste_core::{BlockPos, BlockStateId, Identifier};

use super::{Session, WorldSink};

/// The `minecraft:block_entity_type` path of the type `block` carries, if any: each type's
/// valid-block set in vanilla `BlockEntityTypes`. The multi-block families (signs, banners,
/// skulls, shulker boxes, shelves, copper chests) match by block name like the lid and bell
/// filters (`haste_render::blockentity::is_lid_block`); blocks outside `minecraft` (mods)
/// carry none, so their entities only appear through the packets the server sends.
pub fn block_entity_type_path(block: &Identifier) -> Option<&'static str> {
    if block.namespace() != "minecraft" {
        return None;
    }
    let p = block.path();
    // The piston's head block ends like the skull family but belongs to the moving piston's
    // own entity (created from `moving_piston`, not the head).
    if p == "piston_head" {
        return None;
    }
    // The hanging signs first: "..._hanging_sign" also ends in "_sign".
    if p.ends_with("_hanging_sign") {
        return Some("hanging_sign");
    }
    if p.ends_with("_sign") {
        return Some("sign");
    }
    if p.ends_with("_banner") {
        return Some("banner");
    }
    if p.ends_with("_head") || p.ends_with("_skull") {
        return Some("skull");
    }
    if p == "shulker_box" || p.ends_with("_shulker_box") {
        return Some("shulker_box");
    }
    if p.ends_with("_shelf") {
        return Some("shelf");
    }
    // The chest family: the plain chest and every copper chest variant.
    if p == "chest" || p.ends_with("copper_chest") {
        return Some("chest");
    }
    Some(match p {
        "trapped_chest" => "trapped_chest",
        "ender_chest" => "ender_chest",
        "spawner" => "mob_spawner",
        "moving_piston" => "piston",
        "campfire" | "soul_campfire" => "campfire",
        "bee_nest" | "beehive" => "beehive",
        "suspicious_sand" | "suspicious_gravel" => "brushable_block",
        "command_block" | "chain_command_block" | "repeating_command_block" => "command_block",
        "furnace" => "furnace",
        "jukebox" => "jukebox",
        "dispenser" => "dispenser",
        "dropper" => "dropper",
        "creaking_heart" => "creaking_heart",
        "brewing_stand" => "brewing_stand",
        "enchanting_table" => "enchanting_table",
        "end_portal" => "end_portal",
        "beacon" => "beacon",
        "daylight_detector" => "daylight_detector",
        "hopper" => "hopper",
        "comparator" => "comparator",
        "structure_block" => "structure_block",
        "end_gateway" => "end_gateway",
        "conduit" => "conduit",
        "barrel" => "barrel",
        "smoker" => "smoker",
        "blast_furnace" => "blast_furnace",
        "lectern" => "lectern",
        "bell" => "bell",
        "jigsaw" => "jigsaw",
        "sculk_sensor" => "sculk_sensor",
        "calibrated_sculk_sensor" => "calibrated_sculk_sensor",
        "sculk_catalyst" => "sculk_catalyst",
        "sculk_shrieker" => "sculk_shrieker",
        "chiseled_bookshelf" => "chiseled_bookshelf",
        "decorated_pot" => "decorated_pot",
        "crafter" => "crafter",
        "trial_spawner" => "trial_spawner",
        "vault" => "vault",
        "test_block" => "test_block",
        "test_instance_block" => "test_instance_block",
        "copper_golem_statue" => "copper_golem_statue",
        "potent_sulfur" => "potent_sulfur",
        _ => return None,
    })
}

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
