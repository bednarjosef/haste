// The narrow surfaces the network driver needs from the world and the entity store
// (`WorldSink`, `EntitySink`): what the packets push into, borrowed per tick so the client can
// keep the real objects behind its own locks. One-line forwards of `haste_world::World`'s and
// `haste_entity::EntityStore`'s methods; the recording fakes in the tests implement them too.

use std::collections::HashSet;

use glam::DVec3;

use haste_core::view::{BlockView, CollisionShapes};
use haste_data::GameData;
use haste_entity::{EntityDimensions, EntityStore};
use haste_protocol::haste_core::chunk::{BlockEntityData, ChunkData, LightData};
use haste_protocol::haste_core::entity::{
    EntitySpawn, ItemStackData, MetadataEntry, MetadataValue, PlayerInfo,
};
use haste_protocol::haste_core::game::DimensionInfo;
use haste_protocol::haste_core::{BlockPos, BlockStateId, ChunkPos, EntityId};
use haste_world::World;

/// What the driver needs from the world (`haste_world::World`).
pub trait WorldSink {
    /// `World::load_chunk`.
    fn load_chunk(&mut self, data: &ChunkData) -> anyhow::Result<()>;
    /// `World::unload_chunk`.
    fn unload_chunk(&mut self, pos: ChunkPos);
    /// `World::set_block`.
    fn set_block(&mut self, pos: BlockPos, state: BlockStateId);
    /// `World::apply_light`.
    fn apply_light(&mut self, pos: ChunkPos, light: &LightData);
    /// `World::replace_biomes` (chunks_biomes packet).
    fn replace_biomes(&mut self, pos: ChunkPos, data: &[u8]) {
        let _ = (pos, data);
    }
    /// `World::set_block_entity`.
    fn set_block_entity(&mut self, pos: BlockPos, data: BlockEntityData);
    /// The block entity at `pos` (`World::block_entity`; `None` by default for fakes). Named
    /// differently from the world's method so `World` (which has both) stays unambiguous.
    fn block_entity_at(&self, pos: BlockPos) -> Option<&BlockEntityData> {
        let _ = pos;
        None
    }
    /// `World::set_biome_zoom_seed` (vanilla `BiomeManager.biomeZoomSeed` from the spawn info).
    fn set_biome_zoom_seed(&mut self, seed: i64) {
        let _ = seed;
    }
    /// The block state at `pos` (`World::block_state`; `AIR` by default for fakes). Named
    /// differently from the `BlockView` method so `World` (which has both) stays unambiguous.
    fn block_state_at(&self, pos: BlockPos) -> BlockStateId {
        let _ = pos;
        BlockStateId::AIR
    }
    /// Drops every chunk and switches to `dim` (respawn / dimension change).
    fn reset(&mut self, dim: &DimensionInfo);
    /// Number of loaded chunks.
    fn chunk_count(&self) -> usize;
    /// Number of loaded sections with at least one non-air block.
    fn non_empty_sections(&self) -> usize;
    /// What the local player's physics reads; `None` (test fakes) freezes the player in place.
    fn physics(&self) -> Option<(&dyn BlockView, &GameData)> {
        None
    }
}

/// A world with no chunks: the player does not move in it.
pub(super) struct NoBlocks;

/// One shared instance of it, for the paths that need a `'static` block view.
pub(super) const NO_BLOCKS: NoBlocks = NoBlocks;

impl BlockView for NoBlocks {
    fn block_state(&self, _pos: BlockPos) -> BlockStateId {
        BlockStateId::AIR
    }
    fn is_chunk_loaded(&self, _pos: ChunkPos) -> bool {
        false
    }
    fn sky_light(&self, _pos: BlockPos) -> u8 {
        15
    }
    fn block_light(&self, _pos: BlockPos) -> u8 {
        0
    }
    fn min_y(&self) -> i32 {
        0
    }
    fn height(&self) -> i32 {
        0
    }
}
impl CollisionShapes for NoBlocks {
    fn collision_boxes(&self, _state: BlockStateId) -> &[haste_core::Aabb] {
        &[]
    }
    fn outline_boxes(&self, _state: BlockStateId) -> &[haste_core::Aabb] {
        &[]
    }
}

/// The `#strider_warm_blocks` test for the strider the player steers, wired from the game
/// data's block table and the block tags the server synced (`update_tags`). The tag's
/// members are looked up once per tick, not per block.
pub(super) struct StriderWarm<'a> {
    pub(super) data: &'a haste_data::GameData,
    pub(super) warm: &'a HashSet<u32>,
}

impl crate::item_based_steering::WarmBlocks for StriderWarm<'_> {
    fn is_strider_warm(&self, state: BlockStateId) -> bool {
        self.data
            .blocks
            .block_registry_index_of_state(state)
            .is_some_and(|i| self.warm.contains(&i.0))
    }
}

/// What the driver needs from the entity store (`haste_entity::EntityStore`).
pub trait EntitySink {
    /// `EntityStore::spawn`.
    fn spawn(&mut self, s: &EntitySpawn);
    /// `EntityStore::remove`.
    fn remove(&mut self, ids: &[EntityId]);
    /// `EntityStore::move_delta`.
    fn move_delta(&mut self, id: EntityId, delta: DVec3, on_ground: bool);
    /// `EntityStore::set_rotation`.
    fn set_rotation(&mut self, id: EntityId, yaw: f32, pitch: f32, on_ground: bool);
    /// `EntityStore::teleport`.
    fn teleport(&mut self, id: EntityId, pos: DVec3, yaw: f32, pitch: f32, on_ground: bool);
    /// `EntityStore::set_head_yaw`.
    fn set_head_yaw(&mut self, id: EntityId, yaw: f32);
    /// `EntityStore::set_velocity`.
    fn set_velocity(&mut self, id: EntityId, v: DVec3);
    /// `EntityStore::swing`: an entity swung a hand (`swing_animation` packet). Default no-op.
    fn swing(&mut self, id: EntityId, hand: i32, animation: i32, duration: i32) {
        let _ = (id, hand, animation, duration);
    }
    /// `EntityStore::mark_hurt`.
    fn mark_hurt(&mut self, id: EntityId);
    /// `EntityStore::mark_dead`.
    fn mark_dead(&mut self, id: EntityId);
    /// `EntityStore::mark_attack`: entity event 4 (a melee attack animation).
    fn mark_attack(&mut self, id: EntityId) {
        let _ = id;
    }
    /// `EntityStore::mark_hit`: entity event 32 (an armour stand was hit, starting its
    /// wobble). Default no-op.
    fn mark_hit(&mut self, id: EntityId) {
        let _ = id;
    }
    /// `EntityStore::prime_tnt_minecart`: entity event 70 (a TNT cart's fuse starts).
    fn prime_tnt_minecart(&mut self, id: EntityId) {
        let _ = id;
    }
    /// `EntityStore::set_metadata`.
    fn set_metadata(&mut self, id: EntityId, entries: &[MetadataEntry]);
    /// `EntityStore::set_equipment`.
    fn set_equipment(&mut self, id: EntityId, slot: u8, item: ItemStackData);
    /// `EntityStore::player_info`.
    fn player_info(&mut self, info: &[PlayerInfo]);
    /// `EntityStore::set_passengers`: the vehicle's passenger list is replaced by `passengers`.
    fn set_passengers(&mut self, vehicle: EntityId, passengers: &[EntityId]);
    /// `EntityStore::set_world_age` (from `set_time`); the bee's anger end time compares
    /// against it. Default no-op.
    fn set_world_age(&mut self, world_age: i64) {
        let _ = world_age;
    }
    /// `EntityStore::entity_type_info`: registry index and dimensions, for the seat offset.
    fn entity_type_info(&self, _id: EntityId) -> Option<(haste_core::RegistryIndex, (f32, f32))> {
        None
    }
    /// The entity's equipment slots (vanilla `LivingEntity` equipment): (wire slot, stack)
    /// pairs, for the saddle a steered mount must wear. None in fakes.
    fn equipment(&self, _id: EntityId) -> Option<&[(u8, ItemStackData)]> {
        None
    }
    /// The entity's last synced data value at `index` (vanilla `SynchedEntityData`). None in
    /// fakes.
    fn metadata_value(&self, _id: EntityId, _index: u8) -> Option<&MetadataValue> {
        None
    }
    /// The entity's age in ticks (`Entity.tickCount`), for the pickup particle's frozen item
    /// spin and bob. None in fakes that do not track it.
    fn entity_age(&self, _id: EntityId) -> Option<u32> {
        None
    }
    /// Position and rotation of an entity.
    fn pose(&self, id: EntityId) -> Option<(DVec3, f32, f32)>;
    /// Hit box of an entity, for the crosshair (none in fakes by default).
    fn hitbox(&self, _id: EntityId) -> Option<haste_core::Aabb> {
        None
    }
    /// `minecraft:entity_type` registry index of an entity, for the crosshair's pickability
    /// filter (`Entity.isPickable`); none in fakes, which the picker treats as a mod type.
    fn entity_type(&self, _id: EntityId) -> Option<haste_core::RegistryIndex> {
        None
    }
    /// Forgets every entity (respawn / dimension change).
    fn clear(&mut self);
    /// `EntityStore::tick`.
    fn tick(&mut self);
    /// `Arrow.handleEntityEvent` 0: a stuck tipped arrow's potion puff, 20 colour particles
    /// over its box (queued into `out`). Fakes know no arrows.
    fn arrow_potion_puff(
        &mut self,
        id: EntityId,
        out: &mut Vec<haste_entity::arrow::ArrowParticle>,
    ) {
        let _ = (id, out);
    }
    /// `FireworkRocketEntity.handleEntityEvent` 17: a tracked rocket's burst (its position,
    /// movement and synched stack). Fakes know no rockets.
    fn firework_burst(&self, id: EntityId) -> Option<super::fireworks::FireworkBurst> {
        let _ = id;
        None
    }
    /// `EntityStore::tick_in_world`: the tick with the world the dropped items and the
    /// arrows move in (`ItemEntity.tick`'s and `AbstractArrow.tick`'s collision, friction
    /// and fluids), collecting the arrows' own particle requests. Fakes ignore the world.
    fn tick_in_world(
        &mut self,
        _world: &dyn haste_entity::item_entity::ItemWorld,
        _arrows: &dyn haste_entity::arrow::ArrowWorld,
        arrow_particles: &mut Vec<haste_entity::arrow::ArrowParticle>,
    ) {
        self.tick();
        arrow_particles.clear();
    }
    /// `EntityStore::len`.
    fn len(&self) -> usize;
}

impl WorldSink for World {
    fn load_chunk(&mut self, data: &ChunkData) -> anyhow::Result<()> {
        World::load_chunk(self, data).map_err(anyhow::Error::from)
    }
    fn unload_chunk(&mut self, pos: ChunkPos) {
        World::unload_chunk(self, pos)
    }
    fn set_block(&mut self, pos: BlockPos, state: BlockStateId) {
        World::set_block(self, pos, state)
    }
    fn apply_light(&mut self, pos: ChunkPos, light: &LightData) {
        World::apply_light(self, pos, light)
    }
    fn replace_biomes(&mut self, pos: ChunkPos, data: &[u8]) {
        World::replace_biomes(self, pos, data)
    }
    fn set_block_entity(&mut self, pos: BlockPos, data: BlockEntityData) {
        World::set_block_entity(self, pos, data)
    }
    fn block_entity_at(&self, pos: BlockPos) -> Option<&BlockEntityData> {
        World::block_entity(self, pos)
    }
    fn set_biome_zoom_seed(&mut self, seed: i64) {
        World::set_biome_zoom_seed(self, seed)
    }
    fn block_state_at(&self, pos: BlockPos) -> BlockStateId {
        World::block_state(self, pos)
    }
    fn reset(&mut self, dim: &DimensionInfo) {
        let biomes = 1usize << self.biome_bits();
        *self = World::new_with_biomes(dim.clone(), self.game_data().clone(), biomes);
    }
    fn chunk_count(&self) -> usize {
        self.loaded_chunks().count()
    }
    fn non_empty_sections(&self) -> usize {
        let dim = self.dimension();
        let (min, count) = (dim.min_section(), dim.section_count() as i32);
        self.loaded_chunks()
            .filter_map(|p| self.chunk(p))
            .map(|c| {
                (min..min + count)
                    .filter(|y| c.section(*y).is_some_and(|s| !s.is_empty()))
                    .count()
            })
            .sum()
    }
    fn physics(&self) -> Option<(&dyn BlockView, &GameData)> {
        Some((self, self.game_data()))
    }
}

impl EntitySink for EntityStore {
    fn spawn(&mut self, s: &EntitySpawn) {
        EntityStore::spawn(self, s)
    }
    fn remove(&mut self, ids: &[EntityId]) {
        EntityStore::remove(self, ids)
    }
    fn move_delta(&mut self, id: EntityId, delta: DVec3, on_ground: bool) {
        EntityStore::move_delta(self, id, delta, on_ground)
    }
    fn set_rotation(&mut self, id: EntityId, yaw: f32, pitch: f32, on_ground: bool) {
        EntityStore::set_rotation(self, id, yaw, pitch, on_ground)
    }
    fn teleport(&mut self, id: EntityId, pos: DVec3, yaw: f32, pitch: f32, on_ground: bool) {
        EntityStore::teleport(self, id, pos, yaw, pitch, on_ground)
    }
    fn set_head_yaw(&mut self, id: EntityId, yaw: f32) {
        EntityStore::set_head_yaw(self, id, yaw)
    }
    fn set_velocity(&mut self, id: EntityId, v: DVec3) {
        EntityStore::set_velocity(self, id, v)
    }
    fn swing(&mut self, id: EntityId, hand: i32, animation: i32, duration: i32) {
        EntityStore::swing(self, id, hand, animation, duration)
    }
    fn mark_hurt(&mut self, id: EntityId) {
        EntityStore::mark_hurt(self, id)
    }
    fn mark_dead(&mut self, id: EntityId) {
        EntityStore::mark_dead(self, id)
    }
    fn mark_attack(&mut self, id: EntityId) {
        EntityStore::mark_attack(self, id)
    }
    fn mark_hit(&mut self, id: EntityId) {
        EntityStore::mark_hit(self, id)
    }
    fn set_metadata(&mut self, id: EntityId, entries: &[MetadataEntry]) {
        EntityStore::set_metadata(self, id, entries)
    }
    fn set_equipment(&mut self, id: EntityId, slot: u8, item: ItemStackData) {
        EntityStore::set_equipment(self, id, slot, item)
    }
    fn player_info(&mut self, info: &[PlayerInfo]) {
        EntityStore::player_info(self, info)
    }
    fn set_passengers(&mut self, vehicle: EntityId, passengers: &[EntityId]) {
        EntityStore::set_passengers(self, vehicle, passengers)
    }
    fn set_world_age(&mut self, world_age: i64) {
        EntityStore::set_world_age(self, world_age)
    }
    fn entity_type_info(&self, id: EntityId) -> Option<(haste_core::RegistryIndex, (f32, f32))> {
        EntityStore::entity_type_info(self, id)
    }
    fn equipment(&self, id: EntityId) -> Option<&[(u8, ItemStackData)]> {
        self.get(id).map(|e| e.equipment.as_slice())
    }
    fn metadata_value(&self, id: EntityId, index: u8) -> Option<&MetadataValue> {
        self.get(id)
            .and_then(|e| e.metadata.get(index))
            .map(|e| &e.value)
    }
    fn entity_age(&self, id: EntityId) -> Option<u32> {
        self.get(id).map(|e| e.age)
    }
    fn pose(&self, id: EntityId) -> Option<(DVec3, f32, f32)> {
        self.get(id).map(|e| (e.pos, e.yaw, e.pitch))
    }
    fn hitbox(&self, id: EntityId) -> Option<haste_core::Aabb> {
        self.get(id)
            .map(|e| haste_core::Aabb::entity(e.pos, e.dimensions.0 as f64, e.dimensions.1 as f64))
    }
    fn entity_type(&self, id: EntityId) -> Option<haste_core::RegistryIndex> {
        self.get(id).map(|e| e.kind)
    }
    fn clear(&mut self) {
        // Keeps the dimensions oracle, player info and painting variants (vanilla keeps the tab
        // list across a respawn; the registries are not resent on reconfigure).
        EntityStore::clear_entities(self);
    }
    fn tick(&mut self) {
        EntityStore::tick(self)
    }
    fn arrow_potion_puff(
        &mut self,
        id: EntityId,
        out: &mut Vec<haste_entity::arrow::ArrowParticle>,
    ) {
        if let Some(e) = EntityStore::get(self, id) {
            haste_entity::arrow::potion_puff(e, out);
        }
    }
    fn firework_burst(&self, id: EntityId) -> Option<super::fireworks::FireworkBurst> {
        let e = EntityStore::get(self, id)?;
        // The synched stack (`DATA_ITEM_STACK`, slot 8); a rocket without one bursts as the
        // default rocket, which has no explosions.
        let stack = match e.metadata.get(8).map(|m| &m.value) {
            Some(MetadataValue::ItemStack(s)) => s.clone(),
            _ => ItemStackData::default(),
        };
        Some(super::fireworks::FireworkBurst {
            pos: e.pos,
            velocity: e.velocity,
            stack,
        })
    }
    fn tick_in_world(
        &mut self,
        world: &dyn haste_entity::item_entity::ItemWorld,
        arrows: &dyn haste_entity::arrow::ArrowWorld,
        arrow_particles: &mut Vec<haste_entity::arrow::ArrowParticle>,
    ) {
        EntityStore::tick_in_world(self, world, arrows, arrow_particles)
    }
    fn len(&self) -> usize {
        EntityStore::len(self)
    }
}

/// The entity store (dimensions oracle when present).
pub fn new_entity_store(entity_types: usize) -> EntityStore {
    EntityStore::new(EntityDimensions::from_registry_len(entity_types))
}
