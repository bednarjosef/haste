//! Network driver (client-app): owns the `Connection`, drains packets each tick and applies them
//! to the world, the entity store and the local player, and sends the per-tick packets.
//!
//! The driver does not own the world or the entities: each tick borrows them (`&mut`), so the
//! client can keep them behind its own locks and the render thread can read them between ticks.
//! [`WorldSink`] and [`EntitySink`] are the narrow surface the driver needs; they are implemented
//! for `haste_world::World` and `haste_entity::EntityStore` (one-line forwards) and for the
//! recording fakes in the tests.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Context;
use glam::DVec3;
use haste_core::input::InputState;
use haste_core::view::{BlockView, CollisionShapes};
use haste_data::GameData;
use haste_entity::riding::{passenger_seat, PassengerKind, SeatPoint, SeatQuery};
use haste_entity::{EntityDimensions, EntityStore};
use haste_protocol::haste_core::chunk::{BlockEntityData, ChunkData, LightData};
use haste_protocol::haste_core::entity::{EntitySpawn, ItemStackData, MetadataEntry, PlayerInfo};
use haste_protocol::haste_core::game::{DimensionInfo, GameMode};
use haste_protocol::haste_core::{
    BlockPos, BlockStateId, ChunkPos, EntityId, Identifier, RegistryIndex,
};
use haste_protocol::nbt::NbtCompound;
use haste_protocol::packets::play::clientbound as cb;
use haste_protocol::packets::play::clientbound::teleport_flags;
use haste_protocol::{
    ClientboundPlay, ConnectOptions, Connection, Login, ServerboundPlay, StatsSnapshot,
    SyncedRegistries,
};
use haste_world::World;

use crate::player::{LocalPlayer, VehiclePose};

/// Hurt-tilt camera roll duration in ticks (vanilla's `hurtTime` runs to 10).
pub const HURT_TILT_TICKS: u32 = 10;
/// The head equipment slot of `set_equipment` (`EquipmentSlot.values()`: 0 main hand, 1 off
/// hand, 2 boots, 3 leggings, 4 chestplate, 5 helmet, 6 body, 7 saddle). The item worn there
/// can hide the screen behind a camera overlay (the carved pumpkin's blur).
pub const HEAD_SLOT: u8 = 5;

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
    /// `World::set_block_entity`.
    fn set_block_entity(&mut self, pos: BlockPos, data: BlockEntityData);
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
struct NoBlocks;
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
    fn set_block_entity(&mut self, pos: BlockPos, data: BlockEntityData) {
        World::set_block_entity(self, pos, data)
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
    fn len(&self) -> usize {
        EntityStore::len(self)
    }
}

/// The entity store (dimensions oracle when present).
pub fn new_entity_store(entity_types: usize) -> EntityStore {
    EntityStore::new(EntityDimensions::from_registry_len(entity_types))
}

/// The `minecraft:entity_type` registry entries in id order: the synced registry's when the
/// server sent one (its entry order defines the ids), else the static one from the game data
/// (`registries.json`, the data generator's own order; the two agree on vanilla servers).
pub fn entity_type_names(regs: &SyncedRegistries, data: &GameData) -> Vec<Identifier> {
    if let Some(entries) = regs.entries(&Identifier::minecraft("entity_type")) {
        return entries.iter().map(|e| e.id.clone()).collect();
    }
    let n = data.registries.len("minecraft:entity_type");
    (0..n)
        .filter_map(|i| {
            data.registries
                .name("minecraft:entity_type", haste_core::RegistryIndex(i as u32))
                .cloned()
        })
        .collect()
}

/// The synced `minecraft:painting_variant` registry (id -> width/height/asset) for the
/// `PaintingRenderer`: the server sends every variant's data in `registry_data`
/// (`RegistryDataLoader.SYNCHRONIZED_REGISTRIES`), so paintings get their size and sprite
/// without a hard-coded table. Empty when the server sent none (unknown -> back texture).
pub fn painting_variants(regs: &SyncedRegistries) -> HashMap<u32, haste_entity::PaintingVariant> {
    let name = Identifier::minecraft("painting_variant");
    let mut out = HashMap::new();
    if let Some(entries) = regs.entries(&name) {
        for i in 0..entries.len() {
            let Some(nbt) = regs.entry_nbt(&name, i) else {
                continue;
            };
            let (Some(width), Some(height), Some(asset)) = (
                nbt.get_int("width"),
                nbt.get_int("height"),
                nbt.get_string("asset_id"),
            ) else {
                continue;
            };
            if let Ok(asset) = Identifier::parse(asset) {
                out.insert(
                    i as u32,
                    haste_entity::PaintingVariant {
                        width: width.clamp(1, 16) as u8,
                        height: height.clamp(1, 16) as u8,
                        asset,
                    },
                );
            }
        }
    }
    out
}

/// The variant registries the variant mobs read (`haste_entity::VARIANT_REGISTRIES`), each
/// parsed into haste-entity's per-id asset table: `asset_id`/`baby_asset_id` (and `model`
/// for pig/cow/chicken; wolf nests `assets`/`baby_assets` with `wild`/`tame`/`angry` — the
/// wild texture is what an untamed, calm mob shows). An entry whose NBT is absent (a known
/// pack) or does not parse stays `None`: the mob keeps the model table's default look.
/// Returns the registry name and its table, one per registry the server sent.
pub fn mob_variants(
    regs: &SyncedRegistries,
) -> Vec<(String, Vec<Option<haste_entity::VariantAssets>>)> {
    let parse = |nbt: &NbtCompound| -> Option<haste_entity::VariantAssets> {
        if nbt.get_string("asset_id").is_some() {
            haste_entity::VariantAssets::from_assets(
                nbt.get_string("asset_id").unwrap(),
                nbt.get_string("baby_asset_id"),
                nbt.get_string("model"),
            )
        } else {
            // Wolf: assets/baby_assets, each with wild/tame/angry; batch 1 reads the wild
            // texture (tame/angry are TamableAnimal flag textures, a later task).
            let wild = nbt.get_compound("assets")?.get_string("wild")?;
            let baby_wild = nbt.get_compound("baby_assets")?.get_string("wild")?;
            haste_entity::VariantAssets::from_assets(wild, Some(baby_wild), None)
        }
    };
    let mut out = Vec::new();
    for wanted in haste_entity::VARIANT_REGISTRIES {
        let Some(registry) = regs.names().find(|n| n.to_string() == *wanted) else {
            continue;
        };
        let Some(entries) = regs.entries(registry) else {
            continue;
        };
        let table = (0..entries.len())
            .map(|i| regs.entry_nbt(registry, i).as_ref().and_then(parse))
            .collect();
        out.push(((*wanted).to_owned(), table));
    }
    out
}

/// Loads the game data (block states, registries, shapes) once for the whole client.
pub fn load_game_data() -> anyhow::Result<Arc<GameData>> {
    let t = std::time::Instant::now();
    let data = GameData::load().context(
        "loading game data from the data generator reports (run scripts/rebuild-cache.sh)",
    )?;
    tracing::info!(
        "game data: {} block states in {:.0} ms",
        data.blocks.len(),
        t.elapsed().as_secs_f64() * 1000.0
    );
    Ok(Arc::new(data))
}

/// Number of entries in the synced `minecraft:worldgen/biome` registry (the direct biome
/// palette's bit width depends on it).
pub fn synced_biome_count(regs: &SyncedRegistries) -> Option<usize> {
    regs.entries(&Identifier::minecraft("worldgen/biome"))
        .map(|e| e.len())
}

/// Per-connection counters for the HUD/headless printout.
#[derive(Clone, Debug, Default)]
pub struct NetCounters {
    /// Play packets drained by the driver.
    pub packets: u64,
    /// Teleports accepted.
    pub teleports: u64,
    /// Chunks loaded.
    pub chunks_loaded: u64,
    /// Chunk unloads.
    pub chunks_unloaded: u64,
    /// Chunks the world refused to load.
    pub chunk_errors: u64,
    /// Last chunk error message.
    pub last_chunk_error: Option<String>,
    /// Whether the play `login` packet was seen.
    pub got_login: bool,
    /// Respawns / dimension changes.
    pub respawns: u64,
    /// Last chat/system lines (most recent last, at most 20).
    pub chat: Vec<String>,
    /// `sound`, `sound_entity` and `stop_sound` packets seen.
    pub sound_packets: u64,
    /// Own block sounds the player produced (steps, mining, placing, landing).
    pub own_sounds: u64,
}

/// One block entity's lid animation (vanilla `ChestLidController` for chests and ender chests,
/// `ShulkerBoxBlockEntity.updateAnimation`'s progress for shulker boxes: both step their value
/// by 0.1 per tick towards the open target). `old` is the value at the start of the last tick,
/// for the partial-tick lerp.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LidAnim {
    pub old: f32,
    pub value: f32,
    pub open: bool,
}

impl LidAnim {
    /// One 20 Hz tick (vanilla `ChestLidController.tickLid`: 0.1 steps, clamped at 0 and 1).
    fn tick(&mut self) {
        self.old = self.value;
        let target = if self.open { 1.0 } else { 0.0 };
        if self.value < target {
            self.value = (self.value + 0.1).min(target);
        } else if self.value > target {
            self.value = (self.value - 0.1).max(target);
        }
    }

    /// Whether the lid is at rest closed (nothing left to animate, no need to copy it out).
    fn at_rest_closed(&self) -> bool {
        !self.open && self.value == 0.0
    }
}

/// The client's per-tick lightmap state: vanilla `LightmapRenderStateExtractor` (the
/// block-light flicker random walk, the effect-driven night vision and darkness) plus
/// `GameRenderer`'s boss-overlay world darkening and `ClientLevel`'s lightning sky flash. The
/// render thread interpolates it by partial tick (`frame_view`).
#[derive(Clone, Debug)]
pub struct LightmapState {
    /// `LightmapRenderStateExtractor.blockLightFlicker`.
    pub flicker: f32,
    rng: u64,
    /// Night-vision effect remaining ticks (`None` without it, `-1` infinite).
    pub night_vision: Option<i32>,
    /// Darkness effect remaining ticks.
    pub darkness: Option<i32>,
    /// `MobEffectInstance.BlendState` for the darkness effect (22 ticks in and out).
    pub darkness_blend: haste_render::lightmap::BlendState,
    /// Boss bars and their `(darkenScreen, createWorldFog)` flags (`BossEvent`), keyed by uuid
    /// bits.
    boss_bars: HashMap<u128, (bool, bool)>,
    /// `GameRenderer.bossOverlayWorldDarkening` and its value one tick ago.
    pub boss_darkening: f32,
    pub boss_darkening_o: f32,
    /// `BossHealthOverlay.shouldCreateWorldFog()`: any bar asks for the thick boss fog.
    pub boss_world_fog: bool,
    /// `ClientLevel.skyFlashTime`: a lightning strike raises the sky for 2 ticks.
    pub sky_flash_time: i32,
    /// The player's tick count (vanilla `LivingEntity.tickCount`), for the darkness pulse.
    pub tick_count: i32,
}

impl Default for LightmapState {
    fn default() -> Self {
        Self {
            flicker: 0.0,
            // Vanilla seeds `RandomSource.create()` from a unique random seed; a fixed seed
            // keeps Haste's runs reproducible and the walk's values are not observable.
            rng: 0x9e37_79b9_7f4a_7c15,
            night_vision: None,
            darkness: None,
            darkness_blend: haste_render::lightmap::BlendState::default(),
            boss_bars: HashMap::new(),
            boss_darkening: 0.0,
            boss_darkening_o: 0.0,
            boss_world_fog: false,
            sky_flash_time: 0,
            tick_count: 0,
        }
    }
}

impl LightmapState {
    /// A uniform f32 in 0..1 (vanilla `RandomSource.nextFloat`; xorshift64*, seed fixed).
    fn next_float(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        ((x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32) / (1u32 << 24) as f32
    }

    /// One game tick: `LightmapRenderStateExtractor.tick` (flicker), the effect durations and
    /// darkness blend (`MobEffectInstance.tickClient`), the boss-overlay darkening
    /// (`GameRenderer.tick`) and the lightning flash timer (`ClientLevel.tick`).
    pub fn tick(&mut self) {
        let (r1, r2, r3, r4) = (
            self.next_float(),
            self.next_float(),
            self.next_float(),
            self.next_float(),
        );
        self.flicker =
            haste_render::lightmap::block_light_flicker_tick(self.flicker, r1, r2, r3, r4);
        // MobEffectInstance.tickClient -> tickDownDuration; a finite effect that reaches 0 is
        // removed (`LivingEntity.tickEffects`), an infinite one (-1) stays.
        for slot in [&mut self.night_vision, &mut self.darkness] {
            if let Some(d) = slot {
                if *d > 0 {
                    *d -= 1;
                    if *d == 0 {
                        *slot = None;
                    }
                }
            }
        }
        // MobEffectInstance.BlendState.tick: the darkness effect blends over 22 ticks and
        // starts blending out 22 ticks before it ends (MobEffects.DARKNESS).
        let has_darkness = matches!(self.darkness, Some(d) if d == -1 || d > 22);
        self.darkness_blend.tick(has_darkness, 22);
        // GameRenderer.tick: while any boss bar asks to darken the screen, rise by 0.05/tick
        // (capped at 1), else fall by 0.0125/tick.
        self.boss_darkening_o = self.boss_darkening;
        if self.boss_bars.values().any(|(darken, _)| *darken) {
            self.boss_darkening = (self.boss_darkening + 0.05).min(1.0);
        } else if self.boss_darkening > 0.0 {
            self.boss_darkening -= 0.0125;
        }
        // `BossHealthOverlay.shouldCreateWorldFog`: any bar asks for the thick world fog.
        self.boss_world_fog = self.boss_bars.values().any(|(_, fog)| *fog);
        if self.sky_flash_time > 0 {
            self.sky_flash_time -= 1;
        }
        self.tick_count = self.tick_count.wrapping_add(1);
    }

    /// An `update_mob_effect` for us: keep the duration of the night-vision or darkness effect.
    fn update_effect(
        &mut self,
        ids: crate::player::interact::LightmapEffectIds,
        effect: i32,
        duration: i32,
    ) {
        if ids.night_vision == Some(effect) {
            self.night_vision = Some(duration);
        }
        if ids.darkness == Some(effect) {
            self.darkness = Some(duration);
        }
    }

    /// A `remove_mob_effect` for us.
    fn remove_effect(&mut self, ids: crate::player::interact::LightmapEffectIds, effect: i32) {
        if ids.night_vision == Some(effect) {
            self.night_vision = None;
        }
        if ids.darkness == Some(effect) {
            self.darkness = None;
        }
    }

    /// A `boss_event`: `BossEvent` flags (bit 0x01 darkenScreen, bit 0x04 createWorldFog) on
    /// add/update flags.
    fn boss_event(&mut self, id: u128, action: &cb::BossEventAction) {
        let flags = |f: u8| (f & 0x01 != 0, f & 0x04 != 0);
        match action {
            cb::BossEventAction::Add { flags: f, .. } => {
                self.boss_bars.insert(id, flags(*f));
            }
            cb::BossEventAction::Remove => {
                self.boss_bars.remove(&id);
            }
            cb::BossEventAction::UpdateFlags(f) => {
                if let Some(entry) = self.boss_bars.get_mut(&id) {
                    *entry = flags(*f);
                }
            }
            _ => {}
        }
    }

    /// `LightningBolt.tick` on the client: `ClientLevel.setSkyFlashTime(2)`.
    fn lightning(&mut self) {
        self.sky_flash_time = 2;
    }
}

/// Client-side session state the packets drive (everything except world and entities).
pub struct Session {
    /// The local player (player.rs): its packets in, its movement packets out.
    pub player: LocalPlayer,
    /// The local player's entity id (from `login`).
    pub entity_id: EntityId,
    /// The current dimension (from `login`, then each `respawn`).
    pub dimension: DimensionInfo,
    /// Counters.
    pub counters: NetCounters,
    /// Lid animations by block position, driven by `block_event` action 1 (chest and shulker
    /// box lids; entries only exist while the block is one of `lid_states`).
    pub lids: HashMap<BlockPos, LidAnim>,
    /// The block states whose `block_event` action 1 belongs to a lid (chest family, shulker
    /// boxes; copper chests are not drawn yet), filled from the game data at join.
    lid_states: Option<HashSet<BlockStateId>>,
    /// Other players' break progress (`block_destruction`): breaker id -> (block, stage 0..=9).
    pub breaking: HashMap<i32, (BlockPos, u8)>,
    /// Entities the server spawned and has not removed (crosshair targets).
    entity_ids: HashSet<EntityId>,
    entity_boxes: Vec<(EntityId, haste_core::Aabb)>,
    /// `minecraft:entity_type` registry index per `entity_boxes` entry (same order), for the
    /// crosshair's pickability filter. A sentinel (`u32::MAX`) means unknown (mod) type.
    entity_types: Vec<haste_core::RegistryIndex>,
    /// Firework rockets the server spawned and has not removed (their metadata index 9 is the
    /// attached-to entity id, `FireworkRocketEntity.DATA_ATTACHED_TO_TARGET`).
    fireworks: HashSet<EntityId>,
    /// The firework rocket currently attached to us: vanilla `FireworkRocketEntity.tick` pushes a
    /// fall-flying player it is attached to every tick (see [`Session::end_tick`]).
    attached_firework: Option<EntityId>,
    /// Time of day in ticks from the first world clock of the last `set_time` (approximate:
    /// that the first clock is the day clock is not verified).
    pub time_of_day: Option<i64>,
    /// World age in ticks (`set_time`'s `world_age`), for animations keyed to game time
    /// (vanilla `Level.getGameTime`, e.g. `BannerRenderer`'s wave phase).
    pub world_age: i64,
    /// Set by `start_configuration` or `transfer`: the next `login` starts a new world.
    pub new_world_pending: bool,
    /// `minecraft:entity_type` registry entries in id order (the synced registry's when the
    /// server sent one, else the static one from the data generator reports): resolves the
    /// vehicle's type index to its name, for the passenger seat offset.
    pub entity_type_names: Vec<Identifier>,
    /// Sound packets recorded this tick ([`crate::sound::record`]), drained by the game
    /// thread's sound output ([`crate::sound::Sounds::apply`]); cleared at `end_tick` when
    /// nobody drains them (headless). Reused, so a tick without sound packets allocates
    /// nothing.
    pub sound_requests: Vec<crate::sound::SoundRequest>,
    /// The portal and spinning screen effects (`LocalPlayer.portalEffectIntensity`, the
    /// spinning effect time and speed), ticked in `end_tick`.
    pub screen: crate::screen_effects::PortalEffects,
    /// This tick's world probe for the screen effects: the block the camera is inside, the
    /// portal under our feet and the eye position's light (vanilla's
    /// `LevelExtractor.getViewBlockingState` and `NetherPortalBlock.entityInside`).
    pub screen_probe: crate::screen_effects::WorldProbe,
    /// The `fovEffectScale` option this run uses (set by the window from its saved options;
    /// the headless path leaves the vanilla default).
    pub fov_effect_scale: f32,
    /// The smoothed FOV modifier and its value one tick ago (vanilla's `Camera.fovModifier`
    /// and `oldFovModifier`; both start at 0, so the FOV widens over the first ticks).
    pub fov_modifier: f32,
    pub old_fov_modifier: f32,
    /// Hurt ticks left (vanilla's `hurtTime`, 10 per hit) and the damage direction in degrees
    /// (vanilla's `hurtDir`, the `hurt_animation` packet's yaw).
    pub hurt_ticks: u32,
    pub hurt_dir: f32,
    /// Ticks since the local player died (vanilla's `deathTime`, 0 while alive).
    pub death_ticks: u32,
    /// Ticks the current use item has been held (vanilla's `getTicksUsingItem`), and the held
    /// stack's item while it is drawn (a change of item restarts the draw).
    pub use_ticks: u32,
    pub use_item: Option<RegistryIndex>,
    /// Particle spawns queued by `level_particles` since the last tick (drained by the game
    /// thread into the shared pool).
    pub particle_spawns: Vec<haste_render::ParticleSpawn>,
    /// Particle spawns the game thread took over the whole session (counters).
    pub particle_spawned: usize,
    /// `minecraft:particle_type` names by protocol id (the static registry the packet's
    /// numeric ids index; the synced registry is not authoritative for it).
    particle_names: std::sync::Arc<Vec<Identifier>>,
    /// The per-tick lightmap state (flicker, effects, boss darkening, lightning).
    pub lightmap: LightmapState,
}

impl Session {
    /// Fresh state after `Connection::connect` (from the `Login` it returned).
    /// `particle_names` is the static `minecraft:particle_type` registry by protocol id
    /// (the connection's `ProtocolTables`).
    pub fn new(
        entity_id: EntityId,
        game_mode: GameMode,
        dimension: DimensionInfo,
        particle_names: std::sync::Arc<Vec<Identifier>>,
    ) -> Self {
        Self {
            player: LocalPlayer::with_entity(entity_id, game_mode),
            counters: NetCounters {
                got_login: true,
                ..Default::default()
            },
            entity_id,
            dimension,
            breaking: HashMap::new(),
            lids: HashMap::new(),
            lid_states: None,
            entity_ids: HashSet::new(),
            entity_boxes: Vec::new(),
            entity_types: Vec::new(),
            fireworks: HashSet::new(),
            attached_firework: None,
            time_of_day: None,
            world_age: 0,
            new_world_pending: false,
            entity_type_names: Vec::new(),
            sound_requests: Vec::new(),
            screen: crate::screen_effects::PortalEffects::default(),
            screen_probe: crate::screen_effects::WorldProbe::default(),
            fov_effect_scale: 1.0,
            fov_modifier: 0.0,
            old_fov_modifier: 0.0,
            hurt_ticks: 0,
            hurt_dir: 0.0,
            death_ticks: 0,
            use_ticks: 0,
            use_item: None,
            particle_spawns: Vec::new(),
            particle_spawned: 0,
            particle_names,
            lightmap: LightmapState::default(),
        }
    }

    /// `hurt_animation` for our own entity id: the hurt tilt runs for 10 ticks, its roll axis
    /// turned by the packet's damage-direction yaw (vanilla's `animateHurt` setting `hurtDir`).
    fn start_hurt_tilt(&mut self, damage_yaw: f32) {
        self.hurt_dir = damage_yaw;
        self.hurt_ticks = crate::net::HURT_TILT_TICKS;
    }

    /// Sets the `minecraft:entity_type` registry entries in id order: the synced registry when
    /// the server sent one, else the static one from the game data (`registries.json`). The
    /// two agree on vanilla servers (both are the registry's own order); mods that add entity
    /// types are unknown here and ride on the default seat, never a panic.
    pub fn set_entity_type_names(&mut self, names: Vec<Identifier>) {
        self.entity_type_names = names;
    }

    /// Whether a spawned entity's registry index names the firework rocket type.
    fn is_firework_type(&self, kind: RegistryIndex) -> bool {
        self.entity_type_names
            .get(kind.0 as usize)
            .is_some_and(|n| n.path() == "firework_rocket")
    }

    /// Whether a spawned entity's registry index names the lightning bolt type (its tick sets
    /// the sky flash).
    fn is_lightning_type(&self, kind: RegistryIndex) -> bool {
        self.entity_type_names
            .get(kind.0 as usize)
            .is_some_and(|n| n.path() == "lightning_bolt")
    }

    /// One 20 Hz tick of the camera effects (vanilla's `Camera.tick`, plus `LivingEntity.tick` /
    /// `tickDeath`'s hurt and death counters): the FOV modifier target and its smoothing, and
    /// the hurt and death counters. The walk distance and bob for the view bobbing are the
    /// player hand's own (vanilla's one `ClientAvatarState` serves the hand and the camera
    /// alike). Called by the game thread after `end_tick` (the headless path has no camera and
    /// skips it).
    pub fn tick_camera(&mut self, data: Option<&GameData>, first_person: bool) {
        let p = &self.player;
        // The drawn item: the held stack while the use button is down. A change of the held
        // stack restarts the draw (vanilla stops the use when the item in hand changes).
        let held = p.int.input.use_item.then(|| p.main_hand().item).flatten();
        if held != self.use_item {
            self.use_ticks = 0;
        }
        self.use_item = held;
        if held.is_some() {
            self.use_ticks += 1;
        }
        let using = if self.use_ticks > 0 {
            held.and_then(|h| UseItemKind::classify(data, h))
        } else {
            None
        };
        let sprinting = if p.state.sprinting {
            // Vanilla adds the hardcoded "sprinting" attribute modifier (x1.3,
            // ADD_MULTIPLIED_TOTAL) while the player sprints and `getFieldOfViewModifier`
            // reads the attribute with it; haste's physics applies sprinting itself and
            // strips that modifier from its own copy of the attribute.
            1.3
        } else {
            1.0
        };
        let movement_speed = p.attrs.movement_speed * sprinting;
        let target = fov_modifier_target(
            p.state.flying,
            movement_speed,
            p.abilities.fov_modifier,
            using,
            self.use_ticks,
            first_person,
            self.fov_effect_scale,
        );
        self.old_fov_modifier = self.fov_modifier;
        self.fov_modifier =
            (self.fov_modifier + (target - self.fov_modifier) * 0.5).clamp(0.1, 1.5);

        // Hurt and death counters (LivingEntity.tick / tickDeath).
        self.hurt_ticks = self.hurt_ticks.saturating_sub(1);
        if p.health <= 0.0 {
            self.death_ticks = self.death_ticks.saturating_add(1);
        } else {
            self.death_ticks = 0;
        }
    }
}

/// An item whose use affects the FOV (vanilla's `getFieldOfViewModifier` only looks at the
/// bow and the spyglass; every other use item leaves the modifier alone).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseItemKind {
    Bow,
    Spyglass,
}

impl UseItemKind {
    /// Classifies the held stack by `minecraft:item` registry id; `Other` for everything
    /// else (and whenever the game data is not at hand, e.g. the recording test fakes).
    pub fn classify(data: Option<&GameData>, item: RegistryIndex) -> Option<Self> {
        let data = data?;
        let id = |name: &str| {
            data.registries
                .index("minecraft:item", &Identifier::minecraft(name))
        };
        if Some(item) == id("bow") {
            Some(Self::Bow)
        } else if Some(item) == id("spyglass") {
            Some(Self::Spyglass)
        } else {
            None
        }
    }
}

/// `AbstractClientPlayer.getFieldOfViewModifier` (vanilla): the per-tick FOV modifier from
/// the movement speed against the walking speed, the flying bonus, the bow draw and the
/// spyglass, scaled by the `fovEffectScale` option. `movement_speed` is the attribute value
/// *with* the sprinting modifier (vanilla reads the attribute with it).
pub fn fov_modifier_target(
    flying: bool,
    movement_speed: f64,
    walking_speed: f32,
    use_item: Option<UseItemKind>,
    use_ticks: u32,
    first_person: bool,
    effect_scale: f32,
) -> f32 {
    let mut modifier: f64 = 1.0;
    if flying {
        modifier *= 1.1;
    }
    if walking_speed != 0.0 {
        let speed_factor = movement_speed / walking_speed as f64;
        modifier *= (speed_factor + 1.0) / 2.0;
    }
    if let Some(kind) = use_item {
        if kind == UseItemKind::Bow {
            let scale = (use_ticks as f32 / 20.0).min(1.0) as f64;
            modifier *= 1.0 - scale * scale * 0.15;
        } else if first_person && kind == UseItemKind::Spyglass {
            // Vanilla returns before the effect-scale lerp: scoping is a full 0.1.
            return 0.1;
        }
    }
    (1.0 + (modifier - 1.0) * effect_scale as f64) as f32
}

impl Session {
    /// Our seat on `vehicle` if this passenger list names us, else `None` (we are not riding
    /// it): vanilla's `Entity.positionRider` for the local player. The seat order comes from
    /// the list, and the vehicle's attachment points from its type in the entity_type registry.
    fn seat_of<E: EntitySink + ?Sized>(
        &self,
        vehicle: EntityId,
        passengers: &[EntityId],
        entities: &E,
    ) -> Option<SeatPoint> {
        let seat = passengers.iter().position(|p| *p == self.entity_id)?;
        let (kind, dims) = entities.entity_type_info(vehicle)?;
        let vehicle_type = self
            .entity_type_names
            .get(kind.0 as usize)
            .map_or("?", |id| id.path());
        Some(passenger_seat(SeatQuery {
            vehicle_type,
            vehicle_dims: dims,
            seat,
            passengers: passengers.len(),
            passenger: PassengerKind::Player,
            // The passenger is us: `Avatar.DEFAULT_VEHICLE_ATTACHMENT` (0.6 below the seat),
            // which is `minecraft:player`'s own vehicle attachment point.
            passenger_type: "player",
        }))
    }

    /// The block states that carry a lid (chest family, shulker boxes), from the game data;
    /// call once at join so `block_event` can tell lid events from other blocks' (pistons use
    /// the same action id).
    pub fn set_lid_states(&mut self, states: HashSet<BlockStateId>) {
        self.lid_states = Some(states);
    }

    /// Applies one clientbound packet; serverbound replies go to `out`.
    pub fn apply<W: WorldSink + ?Sized, E: EntitySink + ?Sized>(
        &mut self,
        p: ClientboundPlay,
        world: &mut W,
        entities: &mut E,
        regs: &SyncedRegistries,
        out: &mut Vec<ServerboundPlay>,
    ) {
        use ClientboundPlay as P;
        self.counters.packets += 1;
        if let P::Login(l) = &p {
            if std::mem::take(&mut self.new_world_pending) {
                // After a reconfiguration or transfer: a new world, a new player entity.
                // `regs` already holds the registries that came with this login.
                let dim = regs
                    .dimension_info(l.spawn.dimension_type, &l.spawn.dimension)
                    .unwrap_or_else(|| DimensionInfo {
                        name: l.spawn.dimension.clone(),
                        ..DimensionInfo::overworld()
                    });
                world.reset(&dim);
                entities.clear();
                self.fireworks.clear();
                self.attached_firework = None;
                self.dimension = dim;
                self.entity_id = EntityId(l.entity_id);
                let mode = GameMode::from_id(l.spawn.game_mode).unwrap_or(self.player.game_mode);
                self.player = LocalPlayer::with_entity(self.entity_id, mode);
                self.breaking.clear();
                self.lids.clear();
                self.counters.respawns += 1;
            }
        }
        let own = self.entity_id;
        if !self.player.int.has_block_tags() {
            if let Some(t) = regs
                .tags
                .iter()
                .find(|t| t.registry == Identifier::minecraft("block"))
            {
                self.player
                    .int
                    .set_block_tags(haste_physics::mining::BlockTags::new(
                        t.tags.iter().map(|t| (t.name.clone(), t.entries.clone())),
                    ));
            }
        }
        let player_packet = self.player.handle(&p, out);
        if player_packet {
            for (pos, state) in self.player.take_world_edits() {
                world.set_block(pos, state);
            }
        }
        match p {
            P::StartConfiguration(_) | P::Transfer(_) => self.new_world_pending = true,
            P::Login(l) => {
                self.counters.got_login = true;
                self.player.game_mode =
                    GameMode::from_id(l.spawn.game_mode).unwrap_or(self.player.game_mode);
            }
            P::Respawn(r) => {
                self.counters.respawns += 1;
                let dim = regs
                    .dimension_info(r.spawn.dimension_type, &r.spawn.dimension)
                    .unwrap_or_else(|| DimensionInfo {
                        name: r.spawn.dimension.clone(),
                        ..DimensionInfo::overworld()
                    });
                world.reset(&dim);
                entities.clear();
                self.entity_ids.clear();
                self.breaking.clear();
                self.lids.clear();
                self.fireworks.clear();
                self.attached_firework = None;
                self.dimension = dim;
                // A respawn is a new body: no vehicle, no riding.
                self.player.set_vehicle(None, SeatPoint::ZERO);
            }
            P::PlayerPosition(_) => self.counters.teleports += 1,
            P::LevelParticles(p) => self.push_particles(&p, regs),
            // The lightmap's effect inputs (night vision and darkness durations), boss-overlay
            // darkening and the lightning sky flash; these also ride the player's own handling.
            P::UpdateMobEffect(e) if e.entity_id == own.0 => {
                let ids = self.player.int.lightmap_effect_ids;
                self.lightmap.update_effect(ids, e.effect, e.duration);
            }
            P::RemoveMobEffect(e) if e.entity_id == own.0 => {
                let ids = self.player.int.lightmap_effect_ids;
                self.lightmap.remove_effect(ids, e.effect);
            }
            P::BossEvent(b) => self.lightmap.boss_event(b.id.as_u128(), &b.action),
            _ if player_packet => {}
            P::LevelChunkWithLight(c) => match world.load_chunk(&c.chunk) {
                Ok(()) => self.counters.chunks_loaded += 1,
                Err(e) => {
                    self.counters.chunk_errors += 1;
                    if self.counters.chunk_errors <= 3 {
                        tracing::warn!("chunk {:?} not loaded: {e:#}", c.chunk.pos);
                    }
                    self.counters.last_chunk_error = Some(format!("{e:#}"));
                }
            },
            P::ForgetLevelChunk(f) => {
                // The chunk's block entities (and their lids) are gone with it.
                self.lids.retain(|pos, _| pos.chunk() != f.pos);
                world.unload_chunk(f.pos);
                self.counters.chunks_unloaded += 1;
            }
            P::BlockUpdate(b) => {
                let c = b.change();
                self.lids.remove(&c.pos);
                if !self.player.defer_block_update(c.pos, c.state) {
                    world.set_block(c.pos, c.state);
                }
            }
            // A lid's open target (vanilla routes `block_event` to the block entity's
            // `triggerEvent`: action 1, the opener count; chests use `count > 0`, shulker
            // boxes 0 to close and 1 to open, which `> 0` matches exactly).
            P::BlockEvent(b) if b.action == 1 => {
                let is_lid = self
                    .lid_states
                    .as_ref()
                    .is_some_and(|s| s.contains(&world.block_state_at(b.pos)));
                if is_lid {
                    let lid = self.lids.entry(b.pos).or_default();
                    lid.open = b.param > 0;
                }
            }
            P::BlockEntityData(b) => {
                let pos = b.pos;
                world.set_block_entity(
                    pos,
                    BlockEntityData {
                        pos,
                        kind: b.kind as u32,
                        nbt: b.nbt.clone(),
                    },
                );
            }
            P::SectionBlocksUpdate(u) => {
                for c in u.changes() {
                    if !self.player.defer_block_update(c.pos, c.state) {
                        world.set_block(c.pos, c.state);
                    }
                }
            }
            P::BlockDestruction(d) => {
                tracing::debug!(
                    "block_destruction {} {:?} stage {}",
                    d.entity_id,
                    d.pos,
                    d.stage
                );
                if d.stage <= 9 {
                    self.breaking.insert(d.entity_id, (d.pos, d.stage));
                } else {
                    self.breaking.remove(&d.entity_id);
                }
            }
            P::LightUpdate(l) => world.apply_light(l.pos, &l.light),
            P::AddEntity(a) => {
                let s = a.spawn();
                self.entity_ids.insert(s.id);
                if self.is_firework_type(s.kind) {
                    self.fireworks.insert(s.id);
                }
                if self.is_lightning_type(s.kind) {
                    self.lightmap.lightning();
                }
                entities.spawn(&s)
            }
            P::RemoveEntities(r) => {
                let ids: Vec<EntityId> = r.entity_ids.iter().map(|i| EntityId(*i)).collect();
                // If our vehicle is removed, we dismount (the store's riding bookkeeping drops
                // us too, as a passenger of the removed vehicle).
                if self.player.vehicle.is_some_and(|v| ids.contains(&v)) {
                    self.player.set_vehicle(None, SeatPoint::ZERO);
                }
                for id in &ids {
                    self.entity_ids.remove(id);
                    self.fireworks.remove(id);
                    if self.attached_firework == Some(*id) {
                        self.attached_firework = None;
                    }
                }
                entities.remove(&ids);
            }
            P::SetPassengers(s) => {
                let vehicle = EntityId(s.vehicle);
                let passengers: Vec<EntityId> = s.passengers.iter().map(|i| EntityId(*i)).collect();
                entities.set_passengers(vehicle, &passengers);
                // Mount, re-seat, or dismount. The seat comes from the vehicle's type through
                // vanilla's attachment rules (`haste_entity::passenger_seat`), which also names
                // our own seat index: a boat's second passenger sits elsewhere than the first.
                match self.seat_of(vehicle, &passengers, entities) {
                    Some(seat) => self.player.set_vehicle(Some(vehicle), seat),
                    None if self.player.vehicle() == Some(vehicle) => {
                        // Dismount: our vehicle's new list no longer names us. Sneak's shift
                        // flag in player_input is what made the server send this.
                        self.player.set_vehicle(None, SeatPoint::ZERO);
                    }
                    None => {}
                }
            }
            P::MoveEntityPos(m) => {
                entities.move_delta(EntityId(m.entity_id), m.deltas.total(), m.on_ground)
            }
            P::MoveEntityPosRot(m) => {
                let id = EntityId(m.entity_id);
                entities.move_delta(id, m.deltas.total(), m.on_ground);
                entities.set_rotation(id, m.yaw, m.pitch, m.on_ground);
            }
            P::MoveEntityRot(m) => {
                entities.set_rotation(EntityId(m.entity_id), m.yaw, m.pitch, m.on_ground)
            }
            P::EntityPositionSync(s) => {
                if let Some(pos) = s.end_pos() {
                    entities.teleport(EntityId(s.entity_id), pos, s.yaw, s.pitch, s.on_ground);
                }
            }
            P::TeleportEntity(t) => {
                let id = EntityId(t.entity_id);
                let (old, oy, op) = entities.pose(id).unwrap_or((DVec3::ZERO, 0.0, 0.0));
                let rel = |bit: i32| t.flags & bit != 0;
                let pick = |bit: i32, old: f64, new: f64| if rel(bit) { old + new } else { new };
                let pos = DVec3::new(
                    pick(teleport_flags::X, old.x, t.pos.x),
                    pick(teleport_flags::Y, old.y, t.pos.y),
                    pick(teleport_flags::Z, old.z, t.pos.z),
                );
                let yaw = if rel(teleport_flags::YAW) {
                    oy + t.yaw
                } else {
                    t.yaw
                };
                let pitch = if rel(teleport_flags::PITCH) {
                    op + t.pitch
                } else {
                    t.pitch
                };
                entities.teleport(id, pos, yaw, pitch, t.on_ground);
            }
            P::RotateHead(h) => entities.set_head_yaw(EntityId(h.entity_id), h.head_yaw),
            P::SwingAnimation(s) => {
                entities.swing(EntityId(s.entity_id), s.hand, s.animation, s.duration)
            }
            P::SetEntityMotion(m) => entities.set_velocity(EntityId(m.entity_id), m.velocity),
            P::HurtAnimation(h) => {
                entities.mark_hurt(EntityId(h.entity_id));
                if EntityId(h.entity_id) == own {
                    self.start_hurt_tilt(h.yaw);
                }
            }
            P::DamageEvent(d) => {
                entities.mark_hurt(EntityId(d.entity_id));
            }
            P::EntityEvent(e) if e.status == 3 => entities.mark_dead(EntityId(e.entity_id)),
            // Entity status 4 is the melee attack animation (IronGolem.handleEntityEvent).
            P::EntityEvent(e) if e.status == 4 => entities.mark_attack(EntityId(e.entity_id)),
            P::SetEntityData(d) => {
                // Our own entity's data is handled by the player (`LocalPlayer::handle`: the air
                // supply and the screen effects' on-fire and freeze ticks), and `_ if
                // player_packet` above already swallowed it; only others go to the store.
                if EntityId(d.entity_id) != own {
                    let id = EntityId(d.entity_id);
                    entities.set_metadata(id, &d.metadata);
                    // A firework rocket's attached-to target (index 9): remember it while it is
                    // attached to us, so `end_tick` can apply its push.
                    if self.fireworks.contains(&id) {
                        if let Some(attached) = firework_attached_target(&d.metadata) {
                            if attached == Some(own) {
                                self.attached_firework = Some(id);
                            } else if self.attached_firework == Some(id) {
                                self.attached_firework = None;
                            }
                        }
                    }
                }
            }
            P::SetEquipment(e) => {
                for (slot, item) in e.equipment {
                    if EntityId(e.entity_id) == own && slot == HEAD_SLOT {
                        // Our own head slot, if the server sends it this way (vanilla sends our
                        // own armor through `container_set_slot`): the worn item's camera
                        // overlay reads it.
                        self.player.int.armor[0] = item.clone();
                    }
                    entities.set_equipment(EntityId(e.entity_id), slot, item);
                }
            }
            P::PlayerInfoUpdate(u) => entities.player_info(&u.infos()),
            P::SetTime(t) => {
                entities.set_world_age(t.world_age);
                self.world_age = t.world_age;
                if let Some(c) = t.clocks.first() {
                    self.time_of_day = Some(c.time);
                }
            }
            P::SystemChat(c) if !c.overlay => self.push_chat(c.content.to_plain(&|_| None)),
            P::DisguisedChat(c) => self.push_chat(format!(
                "<{}> {}",
                c.sender_name.to_plain(&|_| None),
                c.message.to_plain(&|_| None)
            )),
            P::PlayerChat(c) => self.push_chat(format!(
                "<{}> {}",
                c.sender_name.to_plain(&|_| None),
                c.message
            )),
            P::Disconnect(d) => {
                tracing::warn!("server disconnected us: {}", d.reason.to_plain(&|_| None))
            }
            P::Sound(_) | P::SoundEntity(_) | P::StopSound(_) => {
                if let Some(request) = crate::sound::record(&p) {
                    self.counters.sound_packets += 1;
                    self.sound_requests.push(request);
                }
            }
            _ => {}
        }
    }

    /// The local player's tick with `input` (and the interactions set with
    /// `LocalPlayer::set_interact`) and its packets; predicted block changes go into the world.
    /// In a world without physics data (test fakes) the player stays in place.
    pub fn end_tick<W: WorldSink + ?Sized, E: EntitySink + ?Sized>(
        &mut self,
        input: &InputState,
        world: &mut W,
        entities: &E,
        out: &mut Vec<ServerboundPlay>,
    ) {
        // Sound requests nobody drained (headless) are dropped here; the game thread
        // drains them between `receive` and `end_tick`.
        self.sound_requests.clear();
        // The player's own block sounds this tick (footsteps, mining, placing): played
        // through the same request channel the packets use, so the game thread's
        // `Sounds::apply` picks them up; headless only counts them.
        for request in self.player.take_own_sound_requests() {
            self.counters.own_sounds += 1;
            self.sound_requests.push(request);
        }
        self.entity_boxes.clear();
        self.entity_types.clear();
        for id in &self.entity_ids {
            if let Some(b) = entities.hitbox(*id) {
                self.entity_boxes.push((*id, b));
                // Unknown (fakes, mod types) -> sentinel, which the picker treats as pickable.
                self.entity_types.push(
                    entities
                        .entity_type(*id)
                        .unwrap_or(haste_core::RegistryIndex(u32::MAX)),
                );
            }
        }
        // The vehicle's position this tick, for the passenger tick (`ride_tick`). A vehicle we
        // do not track (removed before its set_passengers, spawn not yet arrived) leaves the
        // passenger at its last position. The yaw comes with it: the seat's sideways offset
        // turns with the vehicle (`Entity.positionRider` runs every tick).
        let vehicle = self
            .player
            .vehicle
            .and_then(|v| entities.pose(v))
            .map(|(pos, yaw, _)| VehiclePose { pos, yaw });
        // The attached firework rocket's push, vanilla `FireworkRocketEntity.tick`: while the
        // rocket is attached to us and we are fall flying, each tick our velocity becomes
        // `v + look * 0.1 + (look * 1.5 - v) * 0.5`. The rocket ticked before the player in the
        // client's own order (`NetDriver::end_tick` ticks entities first), so the push lands
        // before this tick's glide update.
        if self.attached_firework.is_some() && self.player.state.fall_flying {
            let s = &mut self.player.state;
            s.velocity = haste_physics::elytra::firework_boost(s.velocity, s.yaw, s.pitch);
        }
        match world.physics() {
            Some((view, data)) => {
                self.player.prepare(data);
                let env = crate::player::interact::InteractEnv {
                    data,
                    entities: &self.entity_boxes,
                    entity_types: &self.entity_types,
                };
                self.player.tick_interact(input, view, &env, vehicle, out)
            }
            None => self.player.tick_with(
                input,
                &NoBlocks,
                &NoBlocks,
                &haste_physics::DefaultBlockPhysics,
                vehicle,
                out,
            ),
        }
        for (pos, state) in self.player.take_world_edits() {
            world.set_block(pos, state);
            self.lids.remove(&pos);
        }
        // Every lid's animation step (vanilla ticks each chest's `ChestLidController` and each
        // shulker box's `updateAnimation` once per client tick).
        for lid in self.lids.values_mut() {
            lid.tick();
        }
        // The screen effects' tick, in vanilla's `LocalPlayer.tick` order: the portal
        // transition effect (`portalProcess.isInsidePortalThisTick`, set by the game thread's
        // world probe through [`Session::screen`]) and the spinning effect, which read the
        // nausea blend factor at the end of the tick.
        let nausea = self.player.int.nausea_factor(1.0);
        self.screen.tick(true, nausea);
    }

    /// The item worn on the head (the helmet armor slot), if any.
    pub fn head_item(&self) -> Option<haste_core::RegistryIndex> {
        self.player.int.armor[0].item
    }

    /// Our own entity's screen-effect bits (on fire, freeze ticks), parsed by `LocalPlayer`.
    pub fn own_flags(&self) -> &crate::screen_effects::OwnFlags {
        &self.player.own_flags
    }

    /// The portal and spinning screen effects, ticked in `end_tick`.
    pub fn screen_effects(&self) -> &crate::screen_effects::PortalEffects {
        &self.screen
    }

    /// Break cracks to draw: other players' (`block_destruction`) and our own.
    pub fn breaking_stages(&self, out: &mut Vec<(BlockPos, u8)>) {
        out.extend(self.breaking.values().copied());
        out.extend(self.player.break_progress());
    }

    /// Lid animations to draw this frame, as (position, value at the last tick's start and
    /// end) pairs for the partial-tick lerp. Only open or animating lids are listed (a closed
    /// chest at rest draws the same as no entry).
    pub fn lid_states(&self, out: &mut Vec<(BlockPos, (f32, f32))>) {
        out.extend(
            self.lids
                .iter()
                .filter(|(_, l)| !l.at_rest_closed())
                .map(|(pos, l)| (*pos, (l.old, l.value))),
        );
    }

    /// `level_particles`: resolve the particle type against the static registry's names and
    /// queue the spawn (see the renderer's particles module for the semantics; unknown types
    /// are skipped there, logged once). `minecraft:dust`'s option data (colour, size) is the
    /// one type-specific payload the renderer reads.
    fn push_particles(&mut self, p: &cb::LevelParticles, _regs: &SyncedRegistries) {
        let Some(kind) = self
            .particle_names
            .get(usize::try_from(p.particle.kind).ok().unwrap_or(usize::MAX))
            .cloned()
        else {
            return;
        };
        let dust = dust_option(&kind, &p.particle.data);
        self.particle_spawns.push(haste_render::ParticleSpawn {
            kind,
            pos: p.pos,
            offset: [p.offset_x, p.offset_y, p.offset_z],
            max_speed: [p.max_speed_x, p.max_speed_y, p.max_speed_z],
            count: p.count,
            randomization: p.randomization.max(0) as u32,
            override_limiter: p.override_limiter,
            dust,
        });
    }

    fn push_chat(&mut self, line: String) {
        tracing::info!("chat: {line}");
        self.counters.chat.push(line);
        if self.counters.chat.len() > 20 {
            self.counters.chat.remove(0);
        }
    }
}

/// `DustParticleOptions` from a particle's raw data (the ARGB colour int, then the size
/// scale f32); `None` for every other type and for a malformed payload.
fn dust_option(kind: &Identifier, data: &[u8]) -> Option<haste_render::DustOption> {
    if kind.path() != "dust" || data.len() < 8 {
        return None;
    }
    let argb = i32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    let scale = f32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    Some(haste_render::DustOption {
        color: [
            ((argb >> 16) & 0xFF) as f32 / 255.0,
            ((argb >> 8) & 0xFF) as f32 / 255.0,
            (argb & 0xFF) as f32 / 255.0,
        ],
        scale,
    })
}

/// Entity metadata serializer type id of `OPTIONAL_UNSIGNED_INT` (the wire type of
/// `FireworkRocketEntity.DATA_ATTACHED_TO_TARGET`; `EntityDataSerializers`' registration order).
const OPTIONAL_UNSIGNED_INT: i32 = 19;

/// The firework rocket's attached-to entity id from a metadata update: index 9
/// (`DATA_ATTACHED_TO_TARGET`), an `OPTIONAL_UNSIGNED_INT` (VarInt, 0 = empty, else id + 1).
/// `None` when this update does not carry the entry (metadata updates are partial).
fn firework_attached_target(metadata: &[MetadataEntry]) -> Option<Option<EntityId>> {
    let e = metadata.iter().find(|e| e.index == 9)?;
    let haste_protocol::haste_core::entity::MetadataValue::Other { type_id, raw } = &e.value else {
        return None;
    };
    if *type_id != OPTIONAL_UNSIGNED_INT {
        return None;
    }
    let v = read_var_int(raw)?;
    Some(if v == 0 { None } else { Some(EntityId(v - 1)) })
}

/// A VarInt as the wire writes it (LEB128, at most 5 bytes for an i32).
fn read_var_int(mut b: &[u8]) -> Option<i32> {
    let mut value = 0u32;
    for i in 0..5 {
        let byte = *b.first()?;
        b = &b[1..];
        value |= u32::from(byte & 0x7F) << (7 * i);
        if byte & 0x80 == 0 {
            return Some(value as i32);
        }
    }
    None
}
/// Cap on the packets kept for a still-starting HUD (a server spamming the UI channel must not
/// grow the buffer without bound; the real backlog during JVM boot is a handful of packets).
const UI_BACKLOG_LIMIT: usize = 8192;

/// Owns the connection and the session state; world and entities are borrowed per tick.
pub struct NetDriver {
    conn: Connection,
    login: Login,
    /// Session state.
    pub session: Session,
    out: Vec<ServerboundPlay>,
    /// Packets the UI would use, seen while the HUD JVM was still starting (replayed by the
    /// game thread once the `Ui` exists; see `crate::ui::ui_handles`).
    ui_backlog: Vec<ClientboundPlay>,
}

impl NetDriver {
    /// Connects (handshake, login, configuration) and returns in the play state.
    pub async fn connect(opts: ConnectOptions) -> anyhow::Result<Self> {
        let (conn, login) = Connection::connect(opts).await.context("connecting")?;
        // The static `minecraft:particle_type` registry by protocol id, from the same
        // tables the packet decoder resolves particle names with.
        let particle_names = std::sync::Arc::new(
            (0..)
                .map_while(|i| conn.tables().particle_name(i).map(|s| s.to_string()))
                .map(|s| Identifier::parse(&s).expect("registry names are valid identifiers"))
                .collect::<Vec<_>>(),
        );
        let session = Session::new(
            login.entity_id,
            login.game_mode,
            login.dimension.clone(),
            particle_names,
        );
        Ok(Self {
            conn,
            login,
            session,
            out: Vec::new(),
            ui_backlog: Vec::new(),
        })
    }

    /// The connection (registries, stats, close reason).
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// What `login` said (the dimension may since have changed: `session.dimension`).
    pub fn login(&self) -> &Login {
        &self.login
    }

    /// UI-bound packets seen before the HUD JVM was up, in order; the game thread replays them
    /// into the new `Ui` once it exists so a fresh join's tab list, boss bars and scoreboard
    /// survive the boot.
    pub fn take_ui_backlog(&mut self) -> Vec<ClientboundPlay> {
        std::mem::take(&mut self.ui_backlog)
    }

    /// Connection counters.
    pub fn stats(&self) -> StatsSnapshot {
        self.conn.stats()
    }

    /// Whether the connection has closed.
    pub fn is_closed(&self) -> bool {
        self.conn.is_closed()
    }

    /// Closes the connection.
    pub fn disconnect(self) {
        self.conn.disconnect();
    }

    /// Drains every queued packet into the session, world, entities and the UI (the HUD: every
    /// packet in order, after the client's own handling), and sends the replies.
    pub fn receive<W: WorldSink + ?Sized, E: EntitySink + ?Sized>(
        &mut self,
        world: &mut W,
        entities: &mut E,
        ui: Option<&crate::ui::Ui>,
    ) -> anyhow::Result<()> {
        while let Some(p) = self.conn.try_recv() {
            let regs = self.conn.registries();
            // The HUD sees every packet after the client's own handling, in order; cloning only
            // when there is a UI to show it (the session takes the original). While the HUD JVM
            // is still starting the UI-bound packets are kept and replayed once it is up.
            let keep = ui.is_some() || crate::ui::ui_handles(&p);
            let for_ui = keep.then(|| p.clone());
            self.session.apply(p, world, entities, regs, &mut self.out);
            match (ui, for_ui) {
                (Some(ui), Some(p)) => ui.handle(&p),
                (None, Some(p)) if self.ui_backlog.len() < UI_BACKLOG_LIMIT => {
                    self.ui_backlog.push(p)
                }
                _ => {}
            }
        }
        self.flush()
    }

    /// Ends the tick: entity interpolation step, the local player's tick with `input` and this
    /// tick's serverbound packets.
    pub fn end_tick<W: WorldSink + ?Sized, E: EntitySink + ?Sized>(
        &mut self,
        input: &InputState,
        world: &mut W,
        entities: &mut E,
    ) -> anyhow::Result<()> {
        entities.tick();
        self.session.end_tick(input, world, entities, &mut self.out);
        self.flush()
    }

    /// Queues a serverbound packet; it goes out with this tick's packets.
    pub fn queue(&mut self, p: ServerboundPlay) {
        self.out.push(p);
    }

    /// One 20 Hz tick: [`Self::receive`] then [`Self::end_tick`], without a HUD.
    pub fn tick<W: WorldSink + ?Sized, E: EntitySink + ?Sized>(
        &mut self,
        input: &InputState,
        world: &mut W,
        entities: &mut E,
    ) -> anyhow::Result<()> {
        self.receive(world, entities, None)?;
        self.end_tick(input, world, entities)
    }

    fn flush(&mut self) -> anyhow::Result<()> {
        for p in self.out.drain(..) {
            self.conn
                .send(p)
                .map_err(|e| anyhow::anyhow!("send: {e}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
