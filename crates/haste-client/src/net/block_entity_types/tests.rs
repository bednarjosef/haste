// Vanilla `BlockEntityTypes`' valid-block sets: every family resolves to its type, the names
// the type registry does not share with the block resolve too, and no other block (nor the
// piston's head, nor a mod's) carries one.

use std::collections::HashMap;

use haste_protocol::haste_core::chunk::{ChunkData, LightData};
use haste_protocol::haste_core::game::DimensionInfo;
use haste_protocol::haste_core::{BlockPos, BlockStateId, ChunkPos};
use haste_protocol::packets::play::clientbound::BlockUpdate;
use haste_protocol::ClientboundPlay;

use super::*;

fn path(name: &str) -> Option<&'static str> {
    block_entity_type_path(&Identifier::minecraft(name))
}

/// The single-block types, including the names the type registry does not share with the block
/// (`spawner` -> `mob_spawner`, `suspicious_sand` -> `brushable_block`, ...).
#[test]
fn single_block_types_resolve() {
    assert_eq!(path("chest"), Some("chest"));
    assert_eq!(path("exposed_copper_chest"), Some("chest"));
    assert_eq!(path("waxed_oxidized_copper_chest"), Some("chest"));
    assert_eq!(path("trapped_chest"), Some("trapped_chest"));
    assert_eq!(path("ender_chest"), Some("ender_chest"));
    assert_eq!(path("spawner"), Some("mob_spawner"));
    assert_eq!(path("moving_piston"), Some("piston"));
    assert_eq!(path("suspicious_sand"), Some("brushable_block"));
    assert_eq!(path("suspicious_gravel"), Some("brushable_block"));
    assert_eq!(path("soul_campfire"), Some("campfire"));
    assert_eq!(path("bee_nest"), Some("beehive"));
    assert_eq!(path("chain_command_block"), Some("command_block"));
    assert_eq!(path("bell"), Some("bell"));
    assert_eq!(path("vault"), Some("vault"));
    assert_eq!(path("chiseled_bookshelf"), Some("chiseled_bookshelf"));
    assert_eq!(path("lectern"), Some("lectern"));
    assert_eq!(path("decorated_pot"), Some("decorated_pot"));
}

/// The multi-block families resolve by name suffix, like the lid and bell filters.
#[test]
fn name_suffix_families_resolve() {
    assert_eq!(path("oak_sign"), Some("sign"));
    assert_eq!(path("bamboo_wall_sign"), Some("sign"));
    // A hanging sign also ends in `_sign`: it must win over the plain sign.
    assert_eq!(path("oak_hanging_sign"), Some("hanging_sign"));
    assert_eq!(path("pale_oak_wall_hanging_sign"), Some("hanging_sign"));
    assert_eq!(path("red_wall_banner"), Some("banner"));
    assert_eq!(path("player_head"), Some("skull"));
    assert_eq!(path("wither_skeleton_wall_skull"), Some("skull"));
    assert_eq!(path("cyan_shulker_box"), Some("shulker_box"));
    assert_eq!(path("shulker_box"), Some("shulker_box"));
    assert_eq!(path("poplar_shelf"), Some("shelf"));
}

/// Blocks without an entity resolve to none — including the piston's head, which ends like a
/// skull but is drawn by the moving piston's own entity, and the plain bookshelf beside the
/// chiseled one. A mod's block carries none (its entities arrive as the server sends them).
#[test]
fn blocks_without_entities_resolve_to_none() {
    assert_eq!(path("stone"), None);
    assert_eq!(path("piston"), None);
    assert_eq!(path("piston_head"), None);
    assert_eq!(path("bookshelf"), None);
    assert_eq!(path("repeater"), None);
    assert_eq!(
        block_entity_type_path(&Identifier::new("mod", "chest")),
        None
    );
}

/// A world that records the sink calls, with the block entities it stored (a mini version of
/// `net::tests`' FakeWorld, local so this module's tests stay self-contained).
#[derive(Default)]
struct RecordingWorld {
    log: Vec<String>,
    entities: HashMap<BlockPos, BlockEntityData>,
}

impl WorldSink for RecordingWorld {
    fn load_chunk(&mut self, _data: &ChunkData) -> anyhow::Result<()> {
        unreachable!("no chunk packets in these tests")
    }
    fn unload_chunk(&mut self, _pos: ChunkPos) {
        unreachable!("no chunk packets in these tests")
    }
    fn set_block(&mut self, pos: BlockPos, state: BlockStateId) {
        self.log.push(format!("set {} {} {}", pos.x, pos.y, pos.z));
        let _ = state;
    }
    fn apply_light(&mut self, _pos: ChunkPos, _light: &LightData) {
        unreachable!("no light packets in these tests")
    }
    fn set_block_entity(&mut self, pos: BlockPos, data: BlockEntityData) {
        self.log.push(format!(
            "be {} {} {} {} {:?}",
            pos.x, pos.y, pos.z, data.kind, data.nbt
        ));
        self.entities.insert(pos, data);
    }
    fn block_entity_at(&self, pos: BlockPos) -> Option<&BlockEntityData> {
        self.entities.get(&pos)
    }
    fn reset(&mut self, _dim: &DimensionInfo) {
        unreachable!("no respawn in these tests")
    }
    fn chunk_count(&self) -> usize {
        0
    }
    fn non_empty_sections(&self) -> usize {
        0
    }
}

/// A block update that places a block-entity block creates its block entity client-side
/// (vanilla `LevelChunk.setBlockState` -> `EntityBlock.newBlockEntity`; the server sends no
/// `block_entity_data` for a plain placement, so without this a chest placed while we are
/// connected never draws). A state change keeps the entity of the same type, a state of a
/// different type replaces it, and a block without an entity gets none.
#[test]
fn block_update_creates_the_block_entity() {
    let mut session = Session::new(
        haste_protocol::haste_core::EntityId(1),
        haste_protocol::haste_core::game::GameMode::Creative,
        DimensionInfo::overworld(),
        std::sync::Arc::new(Vec::new()),
    );
    let mut kinds = HashMap::new();
    kinds.insert(BlockStateId(9), 7); // a chest state, type id 7
    kinds.insert(BlockStateId(10), 8); // a trapped chest state, type id 8
    session.set_block_entity_state_kinds(kinds);
    let mut world = RecordingWorld::default();
    let chest = BlockPos::new(2, 64, 3);
    let update = |state| ClientboundPlay::BlockUpdate(BlockUpdate { pos: chest, state });
    session.apply_chunk_updates(update(9), &mut world);
    assert_eq!(
        world.log,
        vec![
            "set 2 64 3".to_string(),
            format!("be 2 64 3 7 {:?}", bytes::Bytes::new())
        ]
    );
    // A state change of the same type: the entity survives (its NBT is kept).
    world.log.clear();
    session.apply_chunk_updates(update(9), &mut world);
    assert_eq!(world.log, ["set 2 64 3"]);
    // A state of a different type: the entity is replaced, with fresh NBT.
    world.log.clear();
    session.apply_chunk_updates(update(10), &mut world);
    assert_eq!(
        world.log,
        vec![
            "set 2 64 3".to_string(),
            format!("be 2 64 3 8 {:?}", bytes::Bytes::new())
        ]
    );
    // A block without an entity: only the state.
    world.log.clear();
    session.apply_chunk_updates(update(0), &mut world);
    assert_eq!(world.log, ["set 2 64 3"]);
}
