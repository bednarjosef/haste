//! Network driver (client-app): owns the `Connection`, drains packets each tick and applies them
//! to the world, the entity store and the local player, and sends the per-tick packets.
//!
//! The driver does not own the world or the entities: each tick borrows them (`&mut`), so the
//! client can keep them behind its own locks and the render thread can read them between ticks.
//! [`WorldSink`] and [`EntitySink`] are the narrow surface the driver needs; they are implemented
//! for `haste_world::World` and `haste_entity::EntityStore` (one-line forwards) and for the
//! recording fakes in the tests.

use std::collections::{HashMap, HashSet};

use anyhow::Context;
use glam::DVec3;
use haste_core::input::InputState;
use haste_core::Direction;
use haste_data::GameData;
use haste_entity::riding::SeatPoint;
use haste_protocol::haste_core::chunk::BlockEntityData;
use haste_protocol::haste_core::entity::MetadataEntry;
use haste_protocol::haste_core::game::{DimensionInfo, GameMode};
use haste_protocol::haste_core::{BlockPos, BlockStateId, EntityId, Identifier, RegistryIndex};
use haste_protocol::packets::play::clientbound as cb;
use haste_protocol::packets::play::clientbound::teleport_flags;
use haste_protocol::packets::play::serverbound as sb;
use haste_protocol::{
    ClientboundPlay, ConnectOptions, Connection, Login, ServerboundPlay, StatsSnapshot,
    SyncedRegistries,
};

use crate::player::{LocalPlayer, VehiclePose};

/// Hurt-tilt camera roll duration in ticks (vanilla's `hurtTime` runs to 10).
pub const HURT_TILT_TICKS: u32 = 10;
/// The server's correction of the vehicle we steer is only taken when it moved the boat this far
/// (vanilla `ClientPacketListener.handleMoveVehicle` compares the distance to 1.0E-5).
const VEHICLE_CORRECTION_EPSILON: f64 = 1.0e-5;
/// The head equipment slot of `set_equipment` (`EquipmentSlot.values()`: 0 main hand, 1 off
/// hand, 2 boots, 3 leggings, 4 chestplate, 5 helmet, 6 body, 7 saddle). The item worn there
/// can hide the screen behind a camera overlay (the carved pumpkin's blur).
pub const HEAD_SLOT: u8 = 5;
/// The saddle equipment slot of `set_equipment` (see `HEAD_SLOT`'s order): a mount wearing it
/// is the one its passenger steers (vanilla `Mob.isSaddled`).
pub const SADDLE_SLOT: u8 = 7;

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
    /// Profiles added by `player_info_update` since the game thread last took them, with their
    /// `textures` property, for the skin manager (`game::skins`).
    pub profiles: Vec<crate::game::skins::ProfileRequest>,
    /// Lid animations by block position, driven by `block_event` action 1 (chest and shulker
    /// box lids; entries only exist while the block is one of `lid_states`).
    pub lids: HashMap<BlockPos, LidAnim>,
    /// The block states whose `block_event` action 1 belongs to a lid (chest family, shulker
    /// boxes; copper chests are not drawn yet), filled from the game data at join.
    lid_states: Option<HashSet<BlockStateId>>,
    /// The bell block states, for `block_event` action 1's ring events.
    bell_states: Option<HashSet<BlockStateId>>,
    /// The decorated pot block states, for `block_event` action 1's wobble events.
    pot_states: Option<HashSet<BlockStateId>>,
    /// Bell rings waiting to be drawn, by block position: the click direction vanilla
    /// `BellBlockEntity.onHit` sends as the event's parameter.
    pub bells: HashMap<BlockPos, Option<Direction>>,
    /// Decorated pot wobbles waiting to be drawn, by block position: the game tick the wobble
    /// started (`DecoratedPotBlockEntity.wobbleStartedAtTick`) and the style's duration.
    pub pots: HashMap<BlockPos, (i64, u32)>,
    /// Other players' break progress (`block_destruction`): breaker id -> (block, stage 0..=9).
    pub breaking: HashMap<i32, (BlockPos, u8)>,
    /// Entities the server spawned and has not removed (crosshair targets).
    entity_ids: HashSet<EntityId>,
    entity_boxes: Vec<(EntityId, haste_core::Aabb)>,
    /// `minecraft:entity_type` registry index per `entity_boxes` entry (same order), for the
    /// crosshair's pickability filter. A sentinel (`u32::MAX`) means unknown (mod) type.
    entity_types: Vec<haste_core::RegistryIndex>,
    /// The block tags the server synced (`update_tags`), shared with `player.int` (which holds
    /// its own copy for mining): the strider's warm test reads them during ticks that also
    /// mutate the player, so the session keeps a copy the env can borrow.
    block_tags: Option<haste_physics::mining::BlockTags>,
    /// Firework rockets the server spawned and has not removed (their metadata index 9 is the
    /// attached-to entity id, `FireworkRocketEntity.DATA_ATTACHED_TO_TARGET`).
    fireworks: HashSet<EntityId>,
    /// The firework rocket currently attached to us: vanilla `FireworkRocketEntity.tick` pushes a
    /// fall-flying player it is attached to every tick (see [`Session::end_tick`]).
    attached_firework: Option<EntityId>,
    /// Time of day in ticks from the overworld clock of the last `set_time` (`clocks` arrives
    /// in HashMap order, so the overworld clock is looked up by registry id).
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
    /// Block particles queued since the last tick (our own break and mining start, and the
    /// server's `level_event`s), drained by the game thread into the shared pool.
    pub block_particle_requests: Vec<crate::block_particles::BlockParticleRequest>,
    /// Ambient block particles of this tick's `animateTick` (reused buffer), spawned into
    /// the pool by the game thread.
    pub ambient_spawns: Vec<haste_render::ParticleSpawn>,
    /// Particle spawns the game thread took over the whole session (counters).
    pub particle_spawned: usize,
    /// `minecraft:particle_type` names by protocol id (the static registry the packet's
    /// numeric ids index; the synced registry is not authoritative for it).
    particle_names: std::sync::Arc<Vec<Identifier>>,
    /// The per-tick lightmap state (flicker, effects, boss darkening, lightning).
    pub lightmap: LightmapState,
    /// The weather state (rain/thunder levels, the rain sound and splash particles).
    pub weather: WeatherState,
    /// `ClientLevel.seaLevel` (the spawn info's; biome precipitation's snow line).
    pub sea_level: i32,
    /// `ClientLevelData.isFlat` (the spawn info's superflat flag; the horizon height).
    pub is_flat: bool,
    /// Server view distance in chunks (from `login`, then `set_chunk_cache_radius`).
    pub view_distance: i32,
    /// The chunk batch flow control (vanilla `ChunkBatchSizeCalculator`): the server sends
    /// chunk batches and waits for our `chunk_batch_received` rate before the next, so a
    /// client that never acknowledges gets one batch and then silence — a teleport's new
    /// area never streams. `batch_started_at` is `onBatchStart`'s timestamp.
    batch_started_at: Option<std::time::Instant>,
    /// `aggregatedNanosPerChunk` (2 ms initially).
    aggregated_nanos_per_chunk: f64,
    /// `oldSamplesWeight` (capped at 49).
    old_samples_weight: i32,
    /// `update_attributes` values of living entities that are not us, by entity id (the syncable
    /// attributes the server sends with an entity's spawn): what a steered mount's own
    /// simulation reads (`movement_speed`, `jump_strength`, `gravity`). Entries live as long as
    /// the entity does (`remove_entities` drops them).
    entity_attributes: HashMap<EntityId, Vec<cb::AttributeProperty>>,
    /// `minecraft:attribute` registry entries in id order: resolves an `update_attributes`
    /// property's index to its name. The synced registry when the server sent one, else the
    /// static one from the game data.
    attribute_names: Option<Vec<Identifier>>,
    /// The player started riding with the last `set_passengers` (vanilla
    /// `ClientPacketListener.handleSetEntityPassengersPacket` shows the "mount.onboard"
    /// overlay message on exactly that transition): taken by the driver, which tells the UI.
    mounted_message: bool,
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
            profiles: Vec::new(),
            counters: NetCounters {
                got_login: true,
                ..Default::default()
            },
            entity_id,
            dimension,
            breaking: HashMap::new(),
            lids: HashMap::new(),
            lid_states: None,
            bell_states: None,
            pot_states: None,
            bells: HashMap::new(),
            pots: HashMap::new(),
            entity_ids: HashSet::new(),
            entity_boxes: Vec::new(),
            entity_types: Vec::new(),
            block_tags: None,
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
            block_particle_requests: Vec::new(),
            ambient_spawns: Vec::new(),
            particle_spawned: 0,
            particle_names,
            lightmap: LightmapState::default(),
            weather: WeatherState::default(),
            sea_level: 63,
            is_flat: false,
            view_distance: 0,
            batch_started_at: None,
            aggregated_nanos_per_chunk: 2_000_000.0,
            old_samples_weight: 1,
            entity_attributes: HashMap::new(),
            attribute_names: None,
            mounted_message: false,
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

    /// Sets the `minecraft:attribute` registry entries in id order (see [`attribute_names`]).
    pub fn set_attribute_names(&mut self, names: Vec<Identifier>) {
        self.attribute_names = Some(names);
    }

    /// Whether the player started riding since the last call (the "mount.onboard" overlay
    /// message, vanilla `ClientPacketListener.handleSetEntityPassengersPacket`).
    pub fn take_mounted_message(&mut self) -> bool {
        std::mem::take(&mut self.mounted_message)
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

impl Session {
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
                self.bells.clear();
                self.pots.clear();
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
                let tags = haste_physics::mining::BlockTags::new(
                    t.tags.iter().map(|t| (t.name.clone(), t.entries.clone())),
                );
                self.player.int.set_block_tags(tags.clone());
                self.block_tags = Some(tags);
            }
        }
        if self.player.int.item_tags_missing() {
            if let Some(t) = regs
                .tags
                .iter()
                .find(|t| t.registry == Identifier::minecraft("item"))
            {
                self.player
                    .int
                    .set_item_tags(haste_physics::mining::BlockTags::new(
                        t.tags.iter().map(|t| (t.name.clone(), t.entries.clone())),
                    ));
            }
        }
        let player_packet = self.player.handle(&p, out);
        if player_packet {
            for (pos, state) in self.player.take_world_edits() {
                world.set_block(pos, state);
            }
            // The boat we steer is this side's authoritative instance of it (vanilla
            // `Entity.isLocalInstanceAuthoritative`): the entity the world draws sits where our
            // simulation put it, not where the server's move packets for it still have it.
            if let (Some(vehicle), Some(boat)) = (self.player.vehicle(), self.player.boat()) {
                entities.teleport(vehicle, boat.pos, boat.yaw, 0.0, boat.on_ground);
            }
        }
        match p {
            P::StartConfiguration(_) | P::Transfer(_) => self.new_world_pending = true,
            P::Login(l) => {
                self.counters.got_login = true;
                self.player.game_mode =
                    GameMode::from_id(l.spawn.game_mode).unwrap_or(self.player.game_mode);
                // `ClientLevel.seaLevel` comes with the spawn info (biome precipitation's
                // snow line hangs off it).
                self.sea_level = l.spawn.sea_level;
                // The spawn info's hashed seed is `BiomeManager`'s zoom seed.
                world.set_biome_zoom_seed(l.spawn.hashed_seed);
                self.is_flat = l.spawn.is_flat;
                self.view_distance = l.view_distance;
            }
            P::Respawn(r) => {
                self.counters.respawns += 1;
                let dim = regs
                    .dimension_info(r.spawn.dimension_type, &r.spawn.dimension)
                    .unwrap_or_else(|| DimensionInfo {
                        name: r.spawn.dimension.clone(),
                        ..DimensionInfo::overworld()
                    });
                self.sea_level = r.spawn.sea_level;
                self.is_flat = r.spawn.is_flat;
                world.reset(&dim);
                // The spawn info's hashed seed is `BiomeManager`'s zoom seed (after the reset).
                world.set_biome_zoom_seed(r.spawn.hashed_seed);
                entities.clear();
                self.entity_ids.clear();
                self.breaking.clear();
                self.lids.clear();
                self.bells.clear();
                self.pots.clear();
                self.fireworks.clear();
                self.attached_firework = None;
                self.dimension = dim;
                // A respawn is a new body: no vehicle, no riding.
                self.entity_attributes.clear();
                self.player
                    .set_vehicle(None, SeatPoint::ZERO, None, None, None);
            }
            P::PlayerPosition(p) => {
                tracing::debug!(
                    "player_position: teleport to ({:.1}, {:.1}, {:.1})",
                    p.pos.x,
                    p.pos.y,
                    p.pos.z
                );
                self.counters.teleports += 1;
            }
            // Vanilla `handleSetChunkCacheCenter` moves the ClientChunkCache's window; our
            // world keeps every chunk it is given, so there is nothing to move. Debug-logged:
            // it fires on every chunk border crossing, which would flood long sessions.
            P::SetChunkCacheCenter(c) => {
                tracing::debug!("set_chunk_cache_center ({}, {})", c.x, c.z);
            }
            // Vanilla `ClientPacketListener.handleSetChunkCacheRadius`: the new server view
            // distance (the game thread caps the meshed radius with it every drain).
            P::SetChunkCacheRadius(r) => {
                self.view_distance = r.radius;
            }
            // Vanilla `ClientPacketListener.handleChunksBiomes`: new biome containers for
            // loaded chunks (`/fillbiome`); the world re-tints the 3x3 columns.
            P::ChunksBiomes(b) => {
                for c in &b.chunks {
                    world.replace_biomes(c.pos, &c.data);
                }
            }
            // The chunk batch flow control (vanilla `ClientPacketListener.handleChunkBatchStart`/
            // `handleChunkBatchFinished` + `ChunkBatchSizeCalculator`): time the batch, fold the
            // per-chunk duration into the running average, and tell the server the rate we want.
            // Without the reply the server never sends the next batch, so a teleport's new area
            // never arrives.
            P::ChunkBatchStart(_) => {
                tracing::debug!("chunk batch start");
                self.batch_started_at = Some(std::time::Instant::now());
            }
            P::ChunkBatchFinished(b) => {
                tracing::debug!("chunk batch finished: {} chunks", b.batch_size);
                if b.batch_size > 0 {
                    if let Some(start) = self.batch_started_at.take() {
                        let nanos_per_chunk =
                            start.elapsed().as_secs_f64() * 1e9 / b.batch_size as f64;
                        // Clamp to a third..three times the running average, then the weighted
                        // mean (vanilla `onBatchFinished`).
                        let clamped = nanos_per_chunk.clamp(
                            self.aggregated_nanos_per_chunk / 3.0,
                            self.aggregated_nanos_per_chunk * 3.0,
                        );
                        self.aggregated_nanos_per_chunk = (self.aggregated_nanos_per_chunk
                            * self.old_samples_weight as f64
                            + clamped)
                            / (self.old_samples_weight as f64 + 1.0);
                        self.old_samples_weight = (self.old_samples_weight + 1).min(49);
                    }
                }
                out.push(ServerboundPlay::ChunkBatchReceived(
                    sb::ChunkBatchReceived {
                        // Vanilla `getDesiredChunksPerTick`: 7 ms over the per-chunk duration.
                        chunks_per_tick: (7_000_000.0 / self.aggregated_nanos_per_chunk) as f32,
                    },
                ));
            }
            P::LevelParticles(p) => self.push_particles(&p, regs),
            P::LevelEvent(e) => {
                // The block-particle events (2001 break, 2019/2020 mining ticks); the
                // rest of the level events are sounds and effects we do not decode.
                if let Some(r) =
                    crate::block_particles::request_from_level_event(e.event, e.pos, e.data)
                {
                    self.block_particle_requests.push(r);
                }
                // The sound half of the two events that play one (vanilla
                // `LevelEventHandler` cases 2001/2020). 2001's payload names the broken
                // state; for 2020 the block is still in the world, so its state is read
                // here (`ClientLevel.addBreakingBlockEffects` reads it at the same
                // point). The server excludes the breaker from its 2001 broadcast
                // (`Block.spawnDestroyByEntityParticles` with the player), so the local
                // player's own break sound is not doubled.
                if e.event == 2001 || e.event == 2020 {
                    if let Some((_, data)) = world.physics() {
                        let state = if e.event == 2001 {
                            BlockStateId(e.data as u32)
                        } else {
                            world.block_state_at(e.pos)
                        };
                        if let Some(t) = data.block_sound_type(state) {
                            crate::sound::level_event_sound(
                                e.event,
                                e.pos,
                                t,
                                &mut self.sound_requests,
                            );
                        }
                    }
                }
            }
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
            // The weather game events (`ClientPacketListener.handleGameEvent`):
            // start_raining resets the rain level to 0 (the server follows with
            // rain_level_change steps that ramp it up), stop_raining sets it to 1 (the
            // steps ramp it down), and the two level changes set rain/thunder directly.
            P::GameEvent(g) if g.event == 1 => self.weather.set_rain_level(0.0),
            P::GameEvent(g) if g.event == 2 => self.weather.set_rain_level(1.0),
            P::GameEvent(g) if g.event == 7 => self.weather.set_rain_level(g.value),
            P::GameEvent(g) if g.event == 8 => self.weather.set_thunder_level(g.value),
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
                self.bells.retain(|pos, _| pos.chunk() != f.pos);
                self.pots.retain(|pos, _| pos.chunk() != f.pos);
                world.unload_chunk(f.pos);
                self.counters.chunks_unloaded += 1;
            }
            P::BlockUpdate(b) => {
                let c = b.change();
                // The block entity, and with it its lid/bell/pot animation, only goes when the
                // block changes (`LevelChunk.setBlockState`), not on a state change of it.
                let block_changed = world.physics().is_none_or(|(_, data)| {
                    data.blocks.block_of(world.block_state_at(c.pos))
                        != data.blocks.block_of(c.state)
                });
                if block_changed {
                    self.lids.remove(&c.pos);
                    self.bells.remove(&c.pos);
                    self.pots.remove(&c.pos);
                }
                if !self.player.defer_block_update(c.pos, c.state) {
                    world.set_block(c.pos, c.state);
                }
            }
            // A block entity's `block_event` action 1: the opener count for a lid (chests use
            // `count > 0`, shulker boxes 0 to close and 1 to open, which `> 0` matches
            // exactly), the click direction for a bell (`BellBlockEntity.onHit`) and the wobble
            // style for a decorated pot (`DecoratedPotBlockEntity.wobble`).
            P::BlockEvent(b) if b.action == 1 => {
                let state = world.block_state_at(b.pos);
                if self.lid_states.as_ref().is_some_and(|s| s.contains(&state)) {
                    let lid = self.lids.entry(b.pos).or_default();
                    lid.open = b.param > 0;
                } else if self
                    .bell_states
                    .as_ref()
                    .is_some_and(|s| s.contains(&state))
                {
                    // `Direction.from3DDataValue(b1)`; vertical values leave the shake
                    // direction unset, which `BellModel.setupAnim` reads as no shake.
                    self.bells.insert(b.pos, Direction::from_ordinal(b.param));
                } else if self.pot_states.as_ref().is_some_and(|s| s.contains(&state)) {
                    // `WobbleStyle.values()[data]`: POSITIVE 7 ticks, NEGATIVE 10.
                    let duration = if b.param == 0 { 7 } else { 10 };
                    self.pots.insert(b.pos, (self.world_age, duration));
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
                    self.player
                        .set_vehicle(None, SeatPoint::ZERO, None, None, None);
                }
                for id in &ids {
                    self.entity_ids.remove(id);
                    self.fireworks.remove(id);
                    self.entity_attributes.remove(id);
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
                // A boat, horse or pig we steer is simulated on our side from the mount on (see
                // [`crate::boat`], [`crate::horse`], [`crate::pig`]); the correction below
                // clears it on any dismount.
                let first = passengers.first() == Some(&self.entity_id);
                let boat = first
                    .then(|| self.boat_at(vehicle, entities, world))
                    .flatten();
                let horse = first.then(|| self.horse_at(vehicle, entities)).flatten();
                let pig = first.then(|| self.pig_at(vehicle, entities)).flatten();
                // Mount, re-seat, or dismount. The seat comes from the vehicle's type through
                // vanilla's attachment rules (`haste_entity::passenger_seat`), which also names
                // our own seat index: a boat's second passenger sits elsewhere than the first.
                match self.seat_of(vehicle, &passengers, entities) {
                    Some(seat) => {
                        // The mount message only on the foot-to-riding transition (vanilla's
                        // `!wasPlayerMounted`): not on a re-seat, which the same packet does.
                        let was_riding = self.player.vehicle().is_some();
                        self.player
                            .set_vehicle(Some(vehicle), seat, boat, horse, pig);
                        if !was_riding {
                            self.mounted_message = true;
                        }
                    }
                    None if self.player.vehicle() == Some(vehicle) => {
                        // Dismount: our vehicle's new list no longer names us. Sneak's shift
                        // flag in player_input is what made the server send this.
                        self.player
                            .set_vehicle(None, SeatPoint::ZERO, None, None, None);
                    }
                    None => {}
                }
            }
            // The server's correction of the vehicle we steer (vanilla
            // `ClientPacketListener.handleMoveVehicle`): our simulation is snapped to where the
            // server put the vehicle, and the answer carries it straight back as our next
            // `move_vehicle`.
            P::MoveVehicle(m) => {
                if let Some(boat) = self.player.boat_mut() {
                    if (m.pos - boat.pos).length() > VEHICLE_CORRECTION_EPSILON {
                        boat.correct(m.pos, m.yaw);
                    }
                    out.push(ServerboundPlay::MoveVehicle(
                        haste_protocol::packets::play::serverbound::MoveVehicle {
                            pos: boat.pos,
                            yaw: boat.yaw,
                            pitch: 0.0,
                            on_ground: boat.on_ground,
                        },
                    ));
                }
                if let Some(horse) = self.player.horse_mut() {
                    if (m.pos - horse.pos).length() > VEHICLE_CORRECTION_EPSILON {
                        horse.correct(m.pos, m.yaw);
                    }
                    out.push(ServerboundPlay::MoveVehicle(
                        haste_protocol::packets::play::serverbound::MoveVehicle {
                            pos: horse.pos,
                            yaw: horse.yaw,
                            pitch: horse.pitch,
                            on_ground: horse.on_ground,
                        },
                    ));
                }
                // A pig or strider only while we hold its stick (otherwise it is not ours and
                // vanilla does not answer).
                if let Some(animal) = self.player.steered_by_item_mut() {
                    if (m.pos - animal.pose().0).length() > VEHICLE_CORRECTION_EPSILON {
                        animal.correct(m.pos, m.yaw);
                    }
                    let (pos, yaw, pitch, on_ground) = animal.pose();
                    out.push(ServerboundPlay::MoveVehicle(
                        haste_protocol::packets::play::serverbound::MoveVehicle {
                            pos,
                            yaw,
                            pitch,
                            on_ground,
                        },
                    ));
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
                    // The pig or strider we ride: the boost's total (a use click on its stick
                    // made the server write it, vanilla `onSyncedDataUpdated` ->
                    // `ItemBasedSteering.onSynced`) and the strider's suffocating flag. The
                    // horse we steer: the server's rear-up flag (the same flag our own
                    // simulation writes locally).
                    if self.player.vehicle() == Some(id) {
                        if let Some(animal) = self.player.item_steered_mut() {
                            animal.on_metadata(&d.metadata);
                        }
                        if let Some(horse) = self.player.horse_mut() {
                            horse.on_metadata(&d.metadata);
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
            // Another entity's attributes (ours are the player's own, handled above): kept for
            // the mounts we steer, whose simulation reads them.
            P::UpdateAttributes(u) if EntityId(u.entity_id) != own => {
                let id = EntityId(u.entity_id);
                let entry = self.entity_attributes.entry(id).or_default();
                for prop in &u.properties {
                    match entry.iter_mut().find(|a| a.id == prop.id) {
                        Some(a) => *a = prop.clone(),
                        None => entry.push(prop.clone()),
                    }
                }
            }
            P::PlayerInfoUpdate(u) => {
                entities.player_info(&u.infos());
                crate::game::skins::added_profiles(&u, &mut self.profiles);
            }
            P::SetTime(t) => {
                entities.set_world_age(t.world_age);
                self.world_age = t.world_age;
                // The day clock is `minecraft:overworld` of `minecraft:world_clock` (the
                // End has its own); `clocks` comes in HashMap order, so look the id up.
                if let Some(c) = t
                    .clocks
                    .iter()
                    .find(|c| c.clock == overworld_clock_id(regs))
                    .or_else(|| t.clocks.first())
                {
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
        // The player's own block particles this tick (break grid, mining cracks).
        self.block_particle_requests
            .extend(self.player.take_own_block_particles());
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
                let warm_tag = crate::item_based_steering::strider_warm_blocks();
                let warm = self
                    .block_tags
                    .as_ref()
                    .and_then(|tags| tags.members(&warm_tag))
                    .map(|warm| StriderWarm { data, warm });
                let env = crate::player::interact::InteractEnv {
                    data,
                    entities: &self.entity_boxes,
                    entity_types: &self.entity_types,
                    warm_blocks: warm
                        .as_ref()
                        .map(|w| w as &dyn crate::item_based_steering::WarmBlocks),
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
            self.bells.remove(&pos);
            self.pots.remove(&pos);
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
        let block_state = block_state_option(&kind, &p.particle.data);
        self.particle_spawns.push(haste_render::ParticleSpawn {
            kind,
            pos: p.pos,
            offset: [p.offset_x, p.offset_y, p.offset_z],
            max_speed: [p.max_speed_x, p.max_speed_y, p.max_speed_z],
            count: p.count,
            randomization: p.randomization.max(0) as u32,
            override_limiter: p.override_limiter,
            dust,
            block_state,
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

/// `BlockParticleOption` from a particle's raw data (one VarInt block state id,
/// `ByteBufCodecs.idMapper(Block.BLOCK_STATE_REGISTRY)`); `None` for every other
/// type and for a malformed payload. The fall-damage landing particles
/// (`LivingEntity.checkFallDamage` -> `sendParticles`) arrive this way.
fn block_state_option(kind: &Identifier, data: &[u8]) -> Option<haste_core::BlockStateId> {
    const BLOCK_KINDS: [&str; 5] = [
        "block",
        "block_marker",
        "falling_dust",
        "dust_pillar",
        "block_crumble",
    ];
    if !BLOCK_KINDS.contains(&kind.path()) || data.is_empty() {
        return None;
    }
    // The VarInt the encoder wrote (up to 5 bytes, little-endian groups).
    let mut value = 0u32;
    for (i, byte) in data.iter().take(5).enumerate() {
        value |= u32::from(byte & 0x7F) << (7 * i);
        if byte & 0x80 == 0 {
            return Some(haste_core::BlockStateId(value));
        }
    }
    None
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
    /// Signed chat: the session, the last-seen window, signing outgoing `chat`.
    pub chat: chat_session::ClientChat,
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
        let mut session = Session::new(
            login.entity_id,
            login.game_mode,
            login.dimension.clone(),
            particle_names,
        );
        session.view_distance = login.view_distance;
        let chat = chat_session::ClientChat::new(conn.profile().uuid, None);
        Ok(Self {
            conn,
            login,
            session,
            chat,
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
    /// packet in order, after the client's own handling), and sends the replies. Packets queued
    /// since the last drain are applied now (vanilla runs the packet queue every frame, before
    /// the tick, `Minecraft.runTick` -> `packetProcessor.processQueuedPackets`).
    pub fn receive<W: WorldSink + ?Sized, E: EntitySink + ?Sized>(
        &mut self,
        world: &mut W,
        entities: &mut E,
        ui: Option<&crate::ui::Ui>,
    ) -> anyhow::Result<()> {
        while let Some(a) = self.conn.try_recv_arrived() {
            let waited_ms = a.at.elapsed().as_secs_f64() * 1000.0;
            if tracing::enabled!(tracing::Level::DEBUG) {
                if let Some(name) = a.packet.name() {
                    if name == "minecraft:container_set_slot" {
                        tracing::debug!(wait_ms = waited_ms, "container_set_slot applied");
                    }
                }
            }
            let regs = self.conn.registries();
            // The HUD sees every packet after the client's own handling, in order; cloning only
            // when there is a UI to show it (the session takes the original). While the HUD JVM
            // is still starting the UI-bound packets are kept and replayed once it is up.
            let keep = ui.is_some() || crate::ui::ui_handles(&a.packet);
            let for_ui = keep.then(|| a.packet.clone());
            self.chat.handle(&a.packet, &mut self.out);
            self.session
                .apply(a.packet, world, entities, regs, &mut self.out);
            // The mount message at the packet's own moment (vanilla shows it while handling
            // `set_passengers`, not on the next tick).
            if self.session.take_mounted_message() {
                if let Some(ui) = ui {
                    ui.show_mount_message();
                }
            }
            match (ui, for_ui) {
                (Some(ui), Some(p)) => ui.handle(&p, &|pos| world.block_state_at(pos)),
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
        for mut p in self.out.drain(..) {
            self.chat.pack(&mut p);
            self.conn
                .send(p)
                .map_err(|e| anyhow::anyhow!("send: {e}"))?;
        }
        Ok(())
    }
}

pub mod account;
mod block_events;
pub mod chat_session;
mod fov;
mod lightmap;
pub(crate) mod pack_prompt;
mod registries;
mod sinks;
mod vehicle;
mod weather;

pub use fov::{fov_modifier_target, UseItemKind};
pub use lightmap::LightmapState;
pub use registries::{
    attribute_names, entity_type_names, load_game_data, mob_variants, overworld_clock_id,
    painting_variants, synced_biome_count,
};
pub use sinks::{new_entity_store, EntitySink, WorldSink};
use sinks::{NoBlocks, StriderWarm, NO_BLOCKS};
pub use weather::{column_height, precipitation_at_pos, Precipitation, WeatherState};

#[cfg(test)]
mod tests;
