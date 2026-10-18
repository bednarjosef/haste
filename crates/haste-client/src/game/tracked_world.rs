//! The world sink wrapper the packet drain goes through: it forwards to the world and
//! records the sections the world itself does not mark dirty but the renderer must drop.

use haste_protocol::haste_core::chunk::{BlockEntityData, ChunkData, LightData};
use haste_protocol::haste_core::game::DimensionInfo;
use haste_protocol::haste_core::{BlockPos, BlockStateId, ChunkPos, SectionPos};
use haste_world::World;

use crate::net::WorldSink;

/// Forwards to the world and records sections the world itself does not mark dirty but the
/// renderer must drop: every section of an unloaded chunk and of every chunk on a respawn.
pub(super) struct TrackedWorld<'a> {
    pub(super) world: &'a mut World,
    pub(super) gone: &'a mut Vec<SectionPos>,
}

impl TrackedWorld<'_> {
    fn chunk_sections(&mut self, pos: ChunkPos) {
        let dim = self.world.dimension();
        let min = dim.min_section();
        for y in min..min + dim.section_count() as i32 {
            self.gone.push(SectionPos::new(pos.x, y, pos.z));
        }
    }
}

impl WorldSink for TrackedWorld<'_> {
    fn load_chunk(&mut self, data: &ChunkData) -> anyhow::Result<()> {
        WorldSink::load_chunk(self.world, data)
    }
    fn unload_chunk(&mut self, pos: ChunkPos) {
        if self.world.chunk(pos).is_some() {
            self.chunk_sections(pos);
        }
        WorldSink::unload_chunk(self.world, pos)
    }
    fn set_block(&mut self, pos: BlockPos, state: BlockStateId) {
        WorldSink::set_block(self.world, pos, state)
    }
    fn apply_light(&mut self, pos: ChunkPos, light: &LightData) {
        WorldSink::apply_light(self.world, pos, light)
    }
    fn replace_biomes(&mut self, pos: ChunkPos, data: &[u8]) {
        WorldSink::replace_biomes(self.world, pos, data)
    }
    fn set_block_entity(&mut self, pos: BlockPos, data: BlockEntityData) {
        WorldSink::set_block_entity(self.world, pos, data)
    }
    fn block_state_at(&self, pos: BlockPos) -> BlockStateId {
        WorldSink::block_state_at(self.world, pos)
    }
    fn reset(&mut self, dim: &DimensionInfo) {
        let loaded: Vec<ChunkPos> = self.world.loaded_chunks().collect();
        for c in loaded {
            self.chunk_sections(c);
        }
        WorldSink::reset(self.world, dim)
    }
    fn chunk_count(&self) -> usize {
        WorldSink::chunk_count(self.world)
    }
    fn non_empty_sections(&self) -> usize {
        WorldSink::non_empty_sections(self.world)
    }
    fn physics(
        &self,
    ) -> Option<(
        &dyn haste_protocol::haste_core::view::BlockView,
        &haste_data::GameData,
    )> {
        WorldSink::physics(self.world)
    }
}
