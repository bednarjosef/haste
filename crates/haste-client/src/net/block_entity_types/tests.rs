// The block-update behaviour: a state whose kind is in the session's map (fed from the
// `block_entity_types.json` dump by `app::block_entity_setup`) gets its entity client-side,
// and one whose is not gets none. Which blocks carry which type is tested against the real
// dump in `app::block_entity_setup::tests`.

use std::collections::HashMap;

use haste_protocol::haste_core::chunk::{ChunkData, LightData};
use haste_protocol::haste_core::game::DimensionInfo;
use haste_protocol::haste_core::{BlockPos, BlockStateId, ChunkPos};
use haste_protocol::packets::play::clientbound::BlockUpdate;
use haste_protocol::ClientboundPlay;

use super::*;

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
