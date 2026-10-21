//! Network driver (client-app): owns the `Connection`, drains packets each tick and applies them
//! to the world, the entity store and the local player, and sends the per-tick packets.
//!
//! The driver does not own the world or the entities: each tick borrows them (`&mut`), so the
//! client can keep them behind its own locks and the render thread can read them between ticks.
//! [`WorldSink`] and [`EntitySink`] are the narrow surface the driver needs; they are implemented
//! for `haste_world::World` and `haste_entity::EntityStore` (one-line forwards) and for the
//! recording fakes in the tests.

use std::collections::{HashMap, HashSet};

use haste_core::Direction;
use haste_protocol::haste_core::game::{DimensionInfo, GameMode};
use haste_protocol::haste_core::{BlockPos, BlockStateId, EntityId, Identifier, RegistryIndex};
use haste_protocol::packets::play::clientbound as cb;
use haste_protocol::{ClientboundPlay, ServerboundPlay, SyncedRegistries};

use crate::player::LocalPlayer;

/// Hurt-tilt camera roll duration in ticks (vanilla's `hurtTime` runs to 10).
pub const HURT_TILT_TICKS: u32 = 10;
/// Vanilla `LivingEntity.damageCooldownTime` per hit (the HUD heart blink's window).
const DAMAGE_COOLDOWN_TICKS: u32 = 20;
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

/// Client-side session state the packets drive (everything except world and entities).
pub struct Session {
    /// The local player (player.rs): its packets in, its movement packets out.
    pub player: LocalPlayer,
    /// The local player's entity id (from `login`).
    pub entity_id: EntityId,
    /// The current dimension (from `login`, then each `respawn`).
    pub dimension: DimensionInfo,
    /// The synced biomes' audio attributes (index = biome id, set once at connect — the
    /// registries are not resent), for the music volume the camera's biome lowers
    /// (`biomes::audio` resolves the target each tick).
    pub biome_audio: Vec<crate::app::biomes::audio::BiomeAudio>,
    /// The dimension type's `audio/music_volume` (vanilla's constant dimension layer of
    /// the environment attribute system): the base the camera's biome overrides each tick.
    pub dimension_music_volume: f32,
    /// The `audio/ambient_sounds` attribute's layers for the current level (the
    /// dimension type's value, each synced biome's override), read at the player's
    /// position by the biome ambient sounds handler.
    pub ambient_sounds_layers: crate::app::biomes::ambient_sounds::AmbientSoundsLayers,
    /// The local player's ambient sound handlers (vanilla `LocalPlayer.ambientSoundHandlers`).
    pub ambient_sound_handlers: crate::sound::AmbientSoundHandlers,
    /// The ticking sound instances they start (vanilla `SoundEngine.tickingSounds`).
    pub ticking_sounds: crate::sound::TickingSounds,
    /// The camera's `MUSIC_VOLUME` as of the last tick (vanilla
    /// `Minecraft.getMusicVolume`): `dimension_music_volume` overridden by the camera's
    /// biome, clamped to the attribute's unit range (`biomes::audio` resolves it).
    pub music_volume: f32,
    /// Counters.
    pub counters: NetCounters,
    /// Game ticks since the join (the packet handlers' log lines carry it).
    pub tick: u64,
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
    /// The `minecraft:block_entity_type` registry id per block state whose block carries a
    /// block entity (vanilla `BlockEntityTypes`' valid-block sets), filled from the game data
    /// and the synced registry at join: a block update that places such a block creates its
    /// block entity client-side, like vanilla `LevelChunk.setBlockState`.
    block_entity_state_kinds: Option<HashMap<BlockStateId, u32>>,
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
    /// The maps' data by id (`handleMapItemData`'s `MapItemSavedData` store), as the
    /// renderer's own versioned type; published to the render thread by `game::maps`.
    pub maps: HashMap<i32, haste_render::MapData>,
    /// Bumped whenever a map changed (the publish clones only then).
    pub maps_version: u64,
    /// The pickup particles (`take_item_entity`): the collected item/orb flying to the
    /// collector for three ticks. Ticked here, published to the render thread each tick.
    pub pickup_particles: Vec<crate::net::pickup::PickupParticle>,
    /// The random vanilla's `ClientPacketListener.random` draws the pickup pitch and sound
    /// seed from.
    pickup_rng: haste_audio::random::LegacyRandom,
    /// The tracking emitters (`ParticleEngine.trackingEmitters`) the `animate` packet's critical
    /// hit actions start: 16 particles a tick on an entity for three ticks. Ticked in `end_tick`.
    pub tracking_emitters: Vec<tracking_emitter::TrackingEmitter>,
    /// The random the emitters' particle offsets are drawn from (vanilla's `Particle.random`,
    /// an unseeded `RandomSource`).
    tracking_rng: haste_audio::random::LegacyRandom,
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
    /// Vanilla `LivingEntity.damageCooldownTime` (20 per hit, 10 on a heal): the HUD heart
    /// blink's gate (`Hud.extractPlayerHealth` blinks while it is positive), ticked down in
    /// `end_tick`.
    pub damage_cooldown: u32,
    /// Vanilla `LocalPlayer.flashOnSetHealth`: the first `set_health` of a player (join,
    /// respawn) only stores the health; drops from then on tilt and blink.
    pub health_seen: bool,
    /// The hurt sounds' random (vanilla's per-entity `RandomSource`): hurt pitch and sound
    /// variant seeds.
    hurt_rng: haste_audio::random::LegacyRandom,
    /// Entity types whose hurt sound fell back to the generic one (log once per type).
    unknown_hurt_types: HashSet<String>,
    /// Ticks since the local player died (vanilla's `deathTime`, 0 while alive).
    pub death_ticks: u32,
    /// Ticks the current use item has been held (vanilla's `getTicksUsingItem`), and the held
    /// stack's item while it is drawn (a change of item restarts the draw).
    pub use_ticks: u32,
    pub use_item: Option<RegistryIndex>,
    /// Particle spawns queued by `level_particles` since the last tick (drained by the game
    /// thread into the shared pool).
    pub particle_spawns: Vec<haste_render::ParticleSpawn>,
    /// Firework bursts queued by entity status 17 since the last tick (drained by the game
    /// thread into the particle pool).
    pub firework_bursts: Vec<fireworks::FireworkBurst>,
    /// The totem pops in progress (vanilla `TrackingEmitter`s the status 35 adds), ticked by
    /// the game thread into the particle pool, and vanilla's level random (also the firework sounds' seeds).
    pub totem_emitters: Vec<crate::game::totem_emitter::TotemEmitter>,
    pub totem_rng: haste_audio::random::LegacyRandom,
    /// Block particles queued since the last tick (our own break and mining start, and the
    /// server's `level_event`s), drained by the game thread into the shared pool.
    pub block_particle_requests: Vec<crate::block_particles::BlockParticleRequest>,
    /// Item particles queued since the last tick (a consumable in use: the eat/drink
    /// crumbs), drained by the game thread into the shared pool.
    pub item_particle_requests: Vec<crate::item_particles::ItemParticleRequest>,
    /// The `explode` packets' block particles (vanilla `ClientExplosionTracker`), ticked
    /// in `end_tick` into the particle queue.
    explosions: explosion_tracker::ExplosionTracker,
    /// The explosion sound's pitch and seed random (vanilla draws them from the level's
    /// `RandomSource`).
    explosion_rng: haste_audio::random::LegacyRandom,
    /// Ambient block particles of this tick's `animateTick` (reused buffer), spawned into
    /// the pool by the game thread.
    pub ambient_spawns: Vec<haste_render::ParticleSpawn>,
    /// The arrows' particle requests since the last tick (`Arrow.makeParticle`), drained
    /// into the shared pool.
    pub arrow_particles: Vec<haste_entity::arrow::ArrowParticle>,
    /// Particle spawns the game thread took over the whole session (counters).
    pub particle_spawned: usize,
    /// `minecraft:particle_type` names by protocol id (the static registry the packet's
    /// numeric ids index; the synced registry is not authoritative for it).
    particle_names: std::sync::Arc<Vec<Identifier>>,
    /// The per-tick lightmap state (flicker, effects, boss darkening, lightning).
    pub lightmap: LightmapState,
    /// The weather state (rain/thunder levels, the rain sound and splash particles).
    pub weather: WeatherState,
    /// The world border (`minecraft:initialize_border` and the `set_border_*` packets): the
    /// interaction step gates its packets on it (`useItemOn` fails without a packet outside
    /// it, `startDestroyBlock` returns false).
    pub(crate) world_border: crate::world_border::WorldBorder,
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
    /// A remote entity's consume draws run through its own generator (vanilla's
    /// `RandomSource` is one per entity), by entity id. Entries live as long as the
    /// entity does (`remove_entities` drops them).
    pub(crate) remote_use: HashMap<EntityId, crate::player::interact::ConsumeEffects>,
    /// The remote uses' sound-request scratch (see `net::remote_use`): drawn in vanilla's
    /// order with the particles, then dropped — the server's sound packets carry the sound.
    remote_use_sounds: Vec<crate::sound::SoundRequest>,
    /// The default spawn point the server sent (`set_default_spawn_position`; vanilla
    /// `LevelData.RespawnData`, the compass's target). Starts at the overworld's origin, as
    /// vanilla's `RespawnData.DEFAULT` does.
    pub respawn: respawn::RespawnData,
    /// The level's difficulty and its lock (`handleChangeDifficulty`); a login resets them.
    pub level_difficulty: difficulty::LevelDifficulty,
    /// The server's MOTD (`handleServerData`); None until the server sends one.
    pub server_motd: Option<haste_protocol::haste_core::Component>,
    /// The server's icon, kept only once it passes `ServerData.validateIcon`.
    pub server_icon: Option<Vec<u8>>,
    /// The camera entity (vanilla `Camera.entity`): the player, or the entity a spectator
    /// spectates (`set_camera`, see [`crate::game::camera_entity`]).
    pub camera: crate::game::camera_entity::CameraEntity,
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
            tick: 0,
            entity_id,
            dimension,
            biome_audio: Vec::new(),
            dimension_music_volume: crate::app::biomes::audio::MUSIC_VOLUME_ATTRIBUTE_DEFAULT,
            music_volume: crate::app::biomes::audio::MUSIC_VOLUME_ATTRIBUTE_DEFAULT,
            ambient_sounds_layers: Default::default(),
            ambient_sound_handlers: Default::default(),
            ticking_sounds: Default::default(),
            breaking: HashMap::new(),
            lids: HashMap::new(),
            lid_states: None,
            bell_states: None,
            pot_states: None,
            block_entity_state_kinds: None,
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
            maps: HashMap::new(),
            maps_version: 0,
            pickup_particles: Vec::new(),
            pickup_rng: Session::new_pickup_rng(),
            tracking_emitters: Vec::new(),
            tracking_rng: tracking_emitter::new_random(),
            sound_requests: Vec::new(),
            screen: crate::screen_effects::PortalEffects::default(),
            screen_probe: crate::screen_effects::WorldProbe::default(),
            fov_effect_scale: 1.0,
            fov_modifier: 0.0,
            old_fov_modifier: 0.0,
            hurt_ticks: 0,
            hurt_dir: 0.0,
            damage_cooldown: 0,
            health_seen: false,
            hurt_rng: haste_audio::random::LegacyRandom::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as i64)
                    .unwrap_or(0x5eed),
            ),
            unknown_hurt_types: HashSet::new(),
            death_ticks: 0,
            use_ticks: 0,
            use_item: None,
            particle_spawns: Vec::new(),
            firework_bursts: Vec::new(),
            totem_emitters: Vec::new(),
            totem_rng: haste_audio::random::LegacyRandom::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as i64)
                    .unwrap_or(0x5eed),
            ),
            block_particle_requests: Vec::new(),
            item_particle_requests: Vec::new(),
            explosions: explosion_tracker::ExplosionTracker::default(),
            explosion_rng: haste_audio::random::LegacyRandom::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as i64)
                    .unwrap_or(0x5eed),
            ),
            ambient_spawns: Vec::new(),
            arrow_particles: Vec::new(),
            particle_spawned: 0,
            particle_names,
            lightmap: LightmapState::default(),
            weather: WeatherState::default(),
            world_border: crate::world_border::WorldBorder::default(),
            sea_level: 63,
            is_flat: false,
            view_distance: 0,
            batch_started_at: None,
            aggregated_nanos_per_chunk: 2_000_000.0,
            old_samples_weight: 1,
            entity_attributes: HashMap::new(),
            attribute_names: None,
            mounted_message: false,
            remote_use: HashMap::new(),
            remote_use_sounds: Vec::new(),
            respawn: respawn::RespawnData::default(),
            level_difficulty: difficulty::LevelDifficulty::default(),
            server_motd: None,
            server_icon: None,
            camera: Default::default(),
        }
    }

    /// `hurt_animation` for our own entity id: the hurt tilt runs for 10 ticks, its roll axis
    /// turned by the packet's damage-direction yaw (vanilla's `animateHurt` setting `hurtDir`).
    fn start_hurt_tilt(&mut self, damage_yaw: f32) {
        self.hurt_dir = damage_yaw;
        self.hurt_ticks = crate::net::HURT_TILT_TICKS;
    }

    /// Whether the player started riding since the last call (the "mount.onboard" overlay
    /// message, vanilla `ClientPacketListener.handleSetEntityPassengersPacket`).
    pub fn take_mounted_message(&mut self) -> bool {
        std::mem::take(&mut self.mounted_message)
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
                self.remote_use.clear();
                // A new level and player: the ambient handlers start over and every sound
                // stops (`Minecraft.updateLevelInEngines`); the dimension layer re-resolves.
                self.ambient_sound_handlers
                    .respawn(&mut self.ticking_sounds, true);
                self.ambient_sounds_layers.dimension =
                    crate::app::biomes::ambient_sounds::dimension_ambient_sounds(regs, &dim);
                self.dimension = dim;
                self.world_border = crate::world_border::WorldBorder::default();
                self.entity_id = EntityId(l.entity_id);
                let mode = GameMode::from_id(l.spawn.game_mode).unwrap_or(self.player.game_mode);
                self.player = LocalPlayer::with_entity(self.entity_id, mode);
                // A new player: no hurt tilt, no blink window, and the first
                // `set_health` only stores the health (vanilla `flashOnSetHealth`).
                self.hurt_ticks = 0;
                self.damage_cooldown = 0;
                self.health_seen = false;
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
        // `set_health`'s old value, for `LocalPlayer.hurtTo` after the player stored the
        // new one (the health drop tilts the camera and starts the HUD blink's cooldown).
        let health_before = matches!(p, P::SetHealth(_)).then(|| self.player.health);
        let player_packet = self.player.handle(&p, out);
        if let (Some(old), P::SetHealth(h)) = (health_before, &p) {
            damage::hurt_to(self, old, h.health);
        }
        if player_packet {
            // The edits are taken out first: the loop below borrows the session.
            for (pos, state) in self.player.take_world_edits() {
                self.set_block_with_block_entities(world, pos, state);
            }
            // The predictions the ack in this batch released (`ClientLevel
            // .syncBlockState`: the server's state restored, the player snapped back
            // only when it collides with it).
            self.sync_block_state(world);
            // The boat we steer is this side's authoritative instance of it (vanilla
            // `Entity.isLocalInstanceAuthoritative`): the entity the world draws sits where our
            // simulation put it, not where the server's move packets for it still have it.
            if let (Some(vehicle), Some(boat)) = (self.player.vehicle(), self.player.boat()) {
                entities.teleport(vehicle, boat.pos, boat.yaw, 0.0, boat.on_ground);
            }
        }
        // The packet groups, one module each like vanilla's ClientPacketListener handlers.
        // The first four run even for a packet the player's own handling claimed (the login
        // and respawn bookkeeping, the lightmap's effect inputs, the weather game events);
        // everything after the `player_packet` check only runs when the player did not
        // handle the packet itself. Each group returns the packet when it has no arm for it,
        // so the next group sees it.
        let Some(p) = self.apply_player_state(p, world, entities, regs) else {
            return;
        };
        let Some(p) = self.apply_world(p, world, out) else {
            return;
        };
        let Some(p) = self.apply_particles(p, world, regs) else {
            return;
        };
        let Some(p) = self.apply_explosion(p) else {
            return;
        };
        let Some(p) = self.apply_effects(p, own) else {
            return;
        };
        if player_packet {
            return;
        }
        let Some(p) = self.apply_chunk_updates(p, world) else {
            return;
        };
        let Some(p) = self.apply_entities(p, own, world, entities, regs, out) else {
            return;
        };
        let Some(p) = self.apply_chat(p) else {
            return;
        };
        let Some(p) = self.apply_animate(p, entities) else {
            return;
        };
        let Some(p) = self.apply_look_at(p, entities) else {
            return;
        };
        self.apply_misc(p, entities, regs);
    }
}

pub mod account;
mod animate;
mod arrow_world;
mod block_entity_types;
mod block_events;
mod camera_tick;
mod chat;
pub mod chat_session;
mod counters;
pub(crate) mod damage;
mod difficulty;
mod driver;
mod effects;
mod entities;
mod evoker_fangs;
mod explosion;
mod explosion_tracker;
pub(crate) mod fireworks;
mod fov;
mod item_world;
mod lightmap;
mod look_at;
mod maps;
mod minecart;
mod misc;
pub(crate) mod pack_prompt;
mod particles;
pub mod pickup;
mod player_state;
mod registries;
mod remote_use;
mod respawn;
mod server_data;
mod session_tick;
mod sinks;
mod sync_block_state;
mod totem;
mod tracking_emitter;
mod vehicle;
mod weather;
mod world;

pub use block_events::LidAnim;
pub use counters::NetCounters;

pub use driver::NetDriver;
pub use fov::{fov_modifier_target, UseItemKind};
pub use lightmap::LightmapState;
pub(crate) use maps::map_id_of;
pub use registries::{
    attribute_names, entity_type_names, load_game_data, mob_variants, overworld_clock_id,
    painting_variants, synced_biome_count,
};
use sinks::NO_BLOCKS;
pub use sinks::{new_entity_store, EntitySink, WorldSink};
pub use weather::{column_height, precipitation_at_pos, Precipitation, WeatherState};

#[cfg(test)]
mod tests;
