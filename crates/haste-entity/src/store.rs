//! Dense entity storage: `Vec`-backed slots with a free list, `EntityId -> slot` in an ahash map.
//! No per-tick allocation.
use std::collections::HashMap;
use std::sync::Arc;

use ahash::AHashMap;
use haste_core::entity::{
    EntitySpawn, ItemStackData, MetadataEntry, MetadataValue, PlayerInfo, Pose,
};
use haste_core::ids::{BlockStateId, EntityId, RegistryIndex};
use haste_core::DVec3;
use uuid::Uuid;

use crate::dims::EntityDimensions;
use crate::lerp::{LerpState, LerpTarget};

use crate::metadata::Metadata;
use crate::riding::Riding;
use crate::walk_swing::{SwingState, WalkAnimationState};

/// `Entity` construction from a spawn packet (`recreateFromPacket`).
mod recreate;

/// Hurt flash duration in ticks (vanilla's `hurtTime` runs to 10).
pub const HURT_TICKS: u32 = 10;
/// Death animation duration in ticks (vanilla's `deathTime` runs to 20).
pub const DEATH_TICKS: u32 = 20;
/// `Creeper.DEFAULT_MAX_SWELL` (the `Fuse` component can change it; the default is all the client
/// usually sees).
pub const CREEPER_MAX_SWELL: i32 = 30;
/// `ClientPacketListener.handleEntityPositionSync`: a sync further than this (squared blocks,
/// 64 blocks) from the entity's current position snaps instead of interpolating.
pub const TELEPORT_SNAP_DISTANCE_SQ: f64 = 4096.0;
/// `ClientPacketListener.handleRotateMob`: ticks a `rotate_head` packet's yaw is eased over.
pub const HEAD_LERP_STEPS: u8 = 3;
/// `LivingEntity.DATA_HEALTH_ID`: the second of `LivingEntity`'s synched fields after
/// `Entity`'s eight (0-7), so index 9 (a float).
pub const HEALTH_METADATA_INDEX: u8 = 9;

// The angle helpers live in `walk_swing` (the rotation states' home); re-exported for the
// callers that still reach them through this module.
pub(crate) use crate::walk_swing::{rot_lerp, wrap_degrees};
// `hurt_strength` moved to `render.rs` with the draw list that reads it.
pub use crate::render::hurt_strength;

/// One tracked entity.
pub struct Entity {
    pub id: EntityId,
    pub uuid: Uuid,
    pub kind: RegistryIndex,
    pub pos: DVec3,
    pub prev_pos: DVec3,
    /// The `add_entity` data field: hanging entities carry their facing direction here
    /// (`ItemFrame.recreateFromPacket` -> `Direction.from3DDataValue(packet.getData())`).
    pub data: i32,
    /// `Entity.yRot` / `xRot`, stepped toward the last packet's rotation by `lerp`
    /// (`LinearInterpolationHandler.doInterpolate`); read them at a partial tick with
    /// `yaw_at` / `pitch_at`.
    pub yaw: f32,
    pub pitch: f32,
    /// `Entity.yRotO` / `xRotO`: the rotation at the end of the previous tick.
    pub prev_yaw: f32,
    pub prev_pitch: f32,
    pub head_yaw: f32,
    /// Velocity the server sent (blocks/tick); informational until the lead asks for prediction.
    pub velocity: DVec3,
    pub on_ground: bool,
    pub pose: Pose,
    /// Width, height for the current pose.
    pub dimensions: (f32, f32),
    pub metadata: Metadata,
    pub equipment: Vec<(u8, ItemStackData)>,
    pub(crate) lerp: LerpState,
    /// A minecart's `NewMinecartBehavior` (the `minecart_improvements` experiment); `None`
    /// for every other entity and for carts on the old behaviour.
    pub new_minecart: Option<Box<crate::new_minecart_behavior::NewMinecartBehavior>>,
    /// Cached nametag from `player_info`.
    pub nametag: Option<String>,
    /// True when the entity type is player-like (humanoid model). Until world-and-data's registry lands we
    /// use the heuristic: an entity whose UUID matches a player-info entry (other players), plus
    /// the caller can set this for remote players seen before their info arrives.
    pub is_player_type: bool,
    /// Limb swing phase and amplitude (`LivingEntity.walkAnimation`).
    pub walk: WalkAnimationState,
    /// Body yaw (`LivingEntity.yBodyRot`), following movement; the whole model turns with it.
    pub body_yaw: f32,
    /// Body yaw at the end of the previous tick (`LivingEntity.yBodyRotO`), for interpolation.
    pub prev_body_yaw: f32,
    /// Head yaw at the previous tick (`LivingEntity.yHeadRotO`), for interpolation.
    pub(crate) prev_head_yaw: f32,
    /// `LivingEntity.lerpYHeadRot` / `lerpHeadSteps`: the head yaw a `rotate_head` packet asked
    /// for and the ticks left to ease `head_yaw` onto it (`lerpHeadTo(yaw, 3)`).
    pub(crate) lerp_head_yaw: f32,
    pub(crate) lerp_head_steps: u8,
    /// The hand swing state (`LivingEntity.swingState`, started by `swing_animation`).
    pub swing: SwingState,
    /// Smoothed swimming amount (`LivingEntity.swimAmount` / `swimAmountO`), 0..1.
    pub swim_amount: f32,
    pub swim_amount_o: f32,
    /// `LivingEntity.fallFlyTicks`: increments each tick while the fall-flying flag is set
    /// (`LivingEntity.tick`), driving `AvatarRenderState.fallFlyingScale`.
    pub fall_flying_ticks: u32,
    /// Ticks since spawn (`Entity.tickCount`), for `ageInTicks`.
    pub tick_count: u64,
    /// Chicken wing flap state (`Chicken.flap` / `oFlap` / `flapSpeed` / `oFlapSpeed`).
    pub flap: f32,
    pub o_flap: f32,
    pub flap_speed: f32,
    pub o_flap_speed: f32,
    pub(crate) flapping: f32,
    /// Ticks of hurt animation left (10 per hit; `hurt_animation`/`damage_event` restart it).
    pub hurt_ticks: u32,
    /// Set on the death event: holds the hurt tint instead of easing out.
    pub dead: bool,
    /// `Creeper.swell`/`oldSwell`: the client-side swell count advanced by `Creeper.tick` from
    /// the synced swell direction, interpolated for `getSwelling(partial)`.
    pub swell: i32,
    pub old_swell: i32,
    /// `IronGolem.attackAnimationTick`: set to 10 by entity event 4, decremented each tick.
    pub attack_ticks: i32,
    /// `EvokerFangs`' client state: the bite started by entity event 4 and its `lifeTicks`.
    pub fangs: crate::evoker_fangs::FangsState,
    /// The tick count when entity event 32 last arrived (`ArmorStand.handleEntityEvent` sets
    /// `lastHit = gameTime`): the armour stand's hit wobble counts from it. A sentinel far in
    /// the past means "never hit" (vanilla's `lastHit = 0` against a large game time).
    pub last_hit: i64,
    /// A dropped item entity's stack (metadata index 8, `ItemEntity.DATA_ITEM`), if its metadata
    /// named one.
    pub item: Option<ItemStackData>,
    /// True for a `minecraft:item` entity: its client-side motion is simulated
    /// (`ItemEntity.getMoveSimulationType()` is `SERVER_AND_CLIENT`).
    pub is_item: bool,
    /// True for an `AbstractBoat` family entity (vanilla's or a mod's): its client-side
    /// paddle and bubble-column state is ticked (`crate::boat`).
    pub is_boat: bool,
    /// `AbstractBoat`'s client-side state (paddle sweeps, bubble-column rocking, the
    /// under-water flag the water patch reads); only meaningful for boats.
    pub boat: crate::boat::BoatState,
    /// Ticks since spawn: drives the dropped items' spin and bob.
    pub age: u32,
    /// A player's trailing cape point (`ClientAvatarState`'s cloak), ticked for players only.
    pub cloak: crate::avatar::Cloak,
    /// Whether the type is a `LivingEntity` (its metadata index 8 is the living-flags
    /// byte; only such entities track a use).
    pub(crate) is_living: bool,
    /// True for a `minecraft:tnt` entity: the client ticks its own fuse down
    /// (`PrimedTnt.tick` runs on the client as well, between the server's syncs).
    pub(crate) is_tnt: bool,
    /// An item use the server started (`useItem`/`useItemRemaining`; see `using_item`).
    pub using_item: crate::using_item::UsingItem,
    /// `AbstractArrow.shakeTime`: ticks of stuck-in-ground shake left, arrows only.
    pub shake_time: u8,
    /// `AbstractArrow.isInGround` as the client knows it (its own ground detection and the
    /// server's `IN_GROUND` metadata both set it), arrows only.
    pub arrow_in_ground: bool,
    /// `AbstractArrow.inGroundTime`: ticks since the arrow stuck, arrows only.
    pub in_ground_time: u32,
    /// The entity is a `minecraft:spectral_arrow` (its own per-tick particle,
    /// `SpectralArrow.tick`).
    pub spectral: bool,
    /// A display entity's interpolated render state (`crate::display`); `None` otherwise.
    pub display: Option<Box<crate::display::DisplayState>>,
}

/// The hands, equipment layers and custom head item of an entity, resolved through the shared
/// per-slot rules ([`crate::local_player::equipment_of`]).
pub struct EntityStore {
    pub(crate) slots: Vec<Option<Entity>>,
    free: Vec<u32>,
    pub(crate) by_id: AHashMap<EntityId, u32>,
    pub(crate) dims: EntityDimensions,
    players: AHashMap<Uuid, PlayerInfo>,
    /// Who rides whom (`set_passengers`).
    pub(crate) riding: Riding,
    /// The synced `minecraft:painting_variant` registry (id -> size/asset), set at join from
    /// the server's `registry_data`; empty when the server sent none.
    pub(crate) painting_variants: HashMap<u32, crate::decoration::PaintingVariant>,
    /// The synced variant registries (cat/wolf/frog/pig/cow/chicken), set at join from the
    /// server's `registry_data`; empty when the server sent none.
    pub(crate) mob_variants: crate::variants::MobVariants,
    /// The world age of the last `set_time` (`NeutralMob.isAngry` compares anger end times
    /// against it); 0 before one arrives.
    pub(crate) world_age: i64,
    /// Item default components (the `equippable` component armour reads), from the datagen
    /// report; empty when absent (armour then draws nothing).
    pub(crate) item_components: haste_data::ItemComponents,
    /// The TNT block's default state: what a primed TNT without a synced
    /// `DATA_BLOCK_STATE_ID` renders as. `AIR` until set (draws nothing), matching
    /// `PrimedTnt.DEFAULT_BLOCK_STATE`.
    pub(crate) default_tnt_state: BlockStateId,
    /// Each player's resolved skin (`PlayerInfo.getSkin`), by profile id; a player without one
    /// wears its default skin (`DefaultPlayerSkin.get`).
    pub(crate) skins: AHashMap<Uuid, Arc<crate::skin::PlayerSkin>>,
    /// The tick's use steps (`LivingEntity.updateUsingItem`), recorded per entity for the
    /// client's effects; reused, so a tick without a use allocates nothing.
    pub(crate) use_ticks: Vec<crate::using_item::UseTick>,
    /// Who is leashed to whom (`SetEntityLink`); the ropes are drawn from it (`leash_lines`).
    pub(crate) leashes: crate::leash::Leashes,
}

impl EntityStore {
    /// Placeholder constructor pending world-and-data: `docs/INTERFACES.md` specifies
    /// `new(data: Arc<haste_data::GameData>)`, but `haste-data` does not export `GameData` yet.
    /// The only thing the store needs from it is the `minecraft:entity_type` registry (name ->
    /// index) to apply oracle dimensions; until it exists every entity gets the 0.6 x 1.8 default
    /// box (exactly what the brief says to do without the oracle file). Switch this to the
    /// documented signature when world-and-data lands.
    pub fn new(dims: EntityDimensions) -> EntityStore {
        EntityStore {
            slots: Vec::new(),
            free: Vec::new(),
            by_id: AHashMap::new(),
            dims,
            players: AHashMap::new(),
            skins: AHashMap::new(),
            riding: Riding::new(),
            painting_variants: HashMap::new(),
            mob_variants: crate::variants::MobVariants::empty(),
            world_age: 0,
            item_components: haste_data::ItemComponents::default(),
            default_tnt_state: BlockStateId::AIR,
            use_ticks: Vec::new(),
            leashes: crate::leash::Leashes::default(),
        }
    }

    /// Sets the item default-components table (the `equippable` component armour reads), loaded
    /// from the datagen report at startup.
    pub fn set_item_components(&mut self, components: haste_data::ItemComponents) {
        self.item_components = components;
    }

    /// Sets the TNT block's default state (what a primed TNT renders as without a synced
    /// `DATA_BLOCK_STATE_ID`), resolved from the block tables at startup.
    pub fn set_default_tnt_state(&mut self, state: BlockStateId) {
        self.default_tnt_state = state;
    }

    /// Sets the synced `minecraft:painting_variant` table (id -> size/asset), from the server's
    /// `registry_data` at join. Paintings whose variant is absent draw the back only.
    pub fn set_painting_variants(
        &mut self,
        variants: HashMap<u32, crate::decoration::PaintingVariant>,
    ) {
        self.painting_variants = variants;
    }

    /// Sets one synced variant registry's table (id -> assets), from the server's
    /// `registry_data` at join (see `haste-client`'s `net::mob_variants`). Registries no
    /// variant mob reads are ignored.
    pub fn set_mob_variants(
        &mut self,
        registry: &str,
        entries: Vec<Option<crate::variants::VariantAssets>>,
    ) {
        self.mob_variants.insert(registry, entries);
    }

    /// The world age from `set_time` (the bee's anger end time compares against it,
    /// `NeutralMob.isAngry`).
    pub fn set_world_age(&mut self, world_age: i64) {
        self.world_age = world_age;
    }

    pub(crate) fn slot(&self, id: EntityId) -> Option<u32> {
        self.by_id.get(&id).copied()
    }

    pub fn spawn(&mut self, s: &EntitySpawn) {
        if self.by_id.contains_key(&s.id) {
            return; // duplicate add_entity: ignore
        }
        let slot = match self.free.pop() {
            Some(i) => {
                self.slots[i as usize] = Some(Entity::new(s, &self.dims));
                i
            }
            None => {
                self.slots.push(Some(Entity::new(s, &self.dims)));
                (self.slots.len() - 1) as u32
            }
        };
        if let Some(e) = &mut self.slots[slot as usize] {
            // A player who joined before their entity spawned (info arrived first).
            if let Some(p) = self.players.get(&e.uuid) {
                e.nametag = Some(p.name.clone());
                e.is_player_type = true;
            }
        }
        self.by_id.insert(s.id, slot);
    }

    /// Clears every entity but keeps the per-connection configuration (the dimensions oracle,
    /// player info and the painting-variant table): a respawn or reconfiguration does not resend
    /// the registries, and vanilla keeps the tab list across a respawn.
    pub fn clear_entities(&mut self) {
        self.slots.clear();
        self.free.clear();
        self.by_id.clear();
        self.riding = Riding::new();
        self.leashes.clear();
    }

    pub fn remove(&mut self, ids: &[EntityId]) {
        for id in ids {
            if let Some(i) = self.by_id.remove(id) {
                self.slots[i as usize] = None;
                self.free.push(i);
            }
        }
        // A removed entity drops out of riding on both sides: its own passengers, and any
        // vehicle it was riding.
        self.riding.on_removed(ids);
        self.leashes.remove_entities(ids);
    }

    /// `set_passengers` for one vehicle: the list replaces the previous one. Passengers that
    /// dropped out lose their vehicle entry; every listed passenger gains this vehicle.
    pub fn set_passengers(&mut self, vehicle: EntityId, passengers: &[EntityId]) {
        self.riding.set_passengers(vehicle, passengers);
    }

    /// The vehicle an entity rides, if any.
    pub fn vehicle_of(&self, id: EntityId) -> Option<EntityId> {
        self.riding.vehicle_of(id)
    }

    /// The passengers of a vehicle, in seat order.
    pub fn passengers_of(&self, id: EntityId) -> Option<&[EntityId]> {
        self.riding.passengers_of(id)
    }

    /// An entity's type index and current dimensions: what a passenger's seat offset is
    /// computed from (`riding::passenger_seat`).
    pub fn entity_type_info(&self, id: EntityId) -> Option<(RegistryIndex, (f32, f32))> {
        self.get(id).map(|e| (e.kind, e.dimensions))
    }

    /// Short delta move (`delta` already converted from 1/4096-block units by the caller).
    pub fn move_delta(&mut self, id: EntityId, delta: DVec3, on_ground: bool) {
        if let Some(e) = self.get_mut(id) {
            e.move_by_delta(delta);
            e.on_ground = on_ground;
        }
    }

    /// A rotation from a move packet (`ClientPacketListener.handleMoveEntity` ->
    /// `Entity.moveOrInterpolateTo(yRot, xRot)`): eased over 3 ticks with the position.
    pub fn set_rotation(&mut self, id: EntityId, yaw: f32, pitch: f32, on_ground: bool) {
        if let Some(e) = self.get_mut(id) {
            e.move_or_interpolate_to(None, Some((yaw, pitch)));
            e.on_ground = on_ground;
        }
    }

    /// An absolute position (`entity_position_sync`, `teleport_entity`):
    /// `ClientPacketListener.handleEntityPositionSync` interpolates it like a move packet
    /// (`Entity.moveOrInterpolateTo`, 3 steps) unless the entity is more than 64 blocks away
    /// (squared distance over `TELEPORT_SNAP_DISTANCE_SQ`), in which case it `snapTo`s: both
    /// `prev_pos` and `pos` jump, so the renderer never draws a frame travelling the gap.
    pub fn teleport(&mut self, id: EntityId, pos: DVec3, yaw: f32, pitch: f32, on_ground: bool) {
        if let Some(e) = self.get_mut(id) {
            e.on_ground = on_ground;
            if e.pos.distance_squared(pos) > TELEPORT_SNAP_DISTANCE_SQ {
                e.pos = pos;
                e.prev_pos = pos;
                e.yaw = yaw;
                e.prev_yaw = yaw;
                e.pitch = pitch;
                e.prev_pitch = pitch;
                e.lerp.snap(pos, yaw, pitch);
            } else {
                e.move_or_interpolate_to(Some(pos), Some((yaw, pitch)));
            }
        }
    }

    /// `rotate_head`: `ClientPacketListener.handleRotateMob` -> `LivingEntity.lerpHeadTo(yaw, 3)`.
    /// The head does not turn now; `tick` eases it over the next three ticks.
    pub fn set_head_yaw(&mut self, id: EntityId, yaw: f32) {
        if let Some(e) = self.get_mut(id) {
            e.lerp_head_yaw = yaw;
            e.lerp_head_steps = HEAD_LERP_STEPS;
        }
    }

    pub fn set_velocity(&mut self, id: EntityId, v: DVec3) {
        if let Some(e) = self.get_mut(id) {
            e.velocity = v;
            // `Projectile.lerpMotion`: a moving arrow leaves the ground it was stuck in.
            if crate::arrow::is_arrow(e.kind) && v.length_squared() > 0.0 {
                e.arrow_in_ground = false;
            }
        }
    }

    pub fn set_metadata(&mut self, id: EntityId, entries: &[MetadataEntry]) {
        let Some(i) = self.slot(id) else { return };
        let Some(e) = &mut self.slots[i as usize] else {
            return;
        };
        let old_living = e.metadata.living;
        e.metadata.apply(entries);
        // `AbstractArrow.onSyncedDataUpdated`: the in-ground flag flipping to true starts the
        // stuck-in-ground shake (arrows only; other types' index 10 means something else).
        if crate::arrow::is_arrow(e.kind) {
            crate::arrow::on_metadata(e, entries);
        }
        // `Display.onSyncedDataUpdated`: flag what the next tick rebuilds.
        crate::display::on_metadata(e, entries);
        // `onSyncedDataUpdated` for the living-flags byte: a use started or stopped.
        if e.is_living && e.metadata.living != old_living {
            crate::using_item::on_living_flags(e);
        }
        // `LivingEntity.DATA_HEALTH_ID` (index 9): health at or below 0 means `isDeadOrDying`,
        // and the client ticks the death animation from that alone (a `/kill` without entity
        // event 3, a player's death, or the event arriving after the metadata).
        if let Some(entry) = e.metadata.get(HEALTH_METADATA_INDEX) {
            if matches!(entry.value, MetadataValue::Float(h) if h <= 0.0) {
                e.start_death();
            }
        }
        // A dropped item entity's stack: `ItemEntity.DATA_ITEM`, the first slot after
        // Entity's eight synched fields (indices 0-7), so index 8. An emptied stack
        // clears the drop. Thrown projectiles carry their item in the same slot
        // (`ThrowableItemProjectile.DATA_ITEM_STACK`): it feeds their sprite decoration
        // (`crate::thrown_item`), never the dropped-item path.
        let thrown = crate::registry::entity_type_name(e.kind)
            .is_some_and(|n| crate::thrown_item::variant_for_name(n).is_some());
        if let Some(entry) = e.metadata.get(8) {
            if matches!(entry.value, MetadataValue::ItemStack(_)) && !thrown {
                // Only `ItemEntity` defines an item stack at this slot, so it also confirms the
                // type when the registry name was unavailable at spawn (no cache).
                e.is_item = true;
            }
            e.item = if thrown {
                None
            } else {
                match &entry.value {
                    MetadataValue::ItemStack(s) if !s.is_empty() => Some(s.clone()),
                    _ => None,
                }
            };
        }
        if let Some(pose) = e.metadata.pose {
            if pose != e.pose {
                e.pose = pose;
                e.dimensions = self.dims.get(e.kind, pose);
            }
        } else if e.metadata.shared.crouching && e.pose == Pose::Standing {
            // Sneaking via the shared flags without an explicit pose entry.
            e.dimensions = self.dims.crouching(e.dimensions);
        }
    }

    pub fn set_equipment(&mut self, id: EntityId, slot: u8, item: ItemStackData) {
        if let Some(e) = self.get_mut(id) {
            match e.equipment.iter_mut().find(|(s, _)| *s == slot) {
                Some(x) => x.1 = item,
                None => e.equipment.push((slot, item)),
            }
        }
    }

    pub fn player_info(&mut self, info: &[PlayerInfo]) {
        for p in info {
            self.players.insert(p.uuid, p.clone());
        }
        for slot in self.slots.iter_mut().flatten() {
            if let Some(p) = self.players.get(&slot.uuid) {
                slot.nametag = Some(p.name.clone());
                // Player-info entries are only sent for real players.
                slot.is_player_type = true;
            }
        }
    }

    /// Sets a player's resolved skin (the client's skin manager, once its textures loaded).
    pub fn set_player_skin(&mut self, uuid: Uuid, skin: Arc<crate::skin::PlayerSkin>) {
        self.skins.insert(uuid, skin);
    }

    /// `AbstractClientPlayer.getSkin`: the resolved skin, else the profile's default skin.
    pub fn player_skin(&self, uuid: Uuid) -> Arc<crate::skin::PlayerSkin> {
        match self.skins.get(&uuid) {
            Some(s) => s.clone(),
            None => crate::skin::default_skin(uuid),
        }
    }

    /// One 20 Hz tick: advance the lerp windows so `prev_pos`/`pos` bracket this tick's step,
    /// update the walk animation, the body yaw (`LivingEntity.tick` -> `calculateEntityAnimation`
    /// and `tickHeadTurn`) and the swing/swim/chicken timers.
    pub fn tick(&mut self) {
        self.tick_in_world(
            &crate::item_entity::NoWorld,
            &crate::arrow::NoArrowWorld,
            &mut Vec::new(),
        );
    }

    /// [`Self::tick`] with a world view for the dropped items (`ItemEntity.tick`) and the
    /// arrows (`AbstractArrow.tick`); `arrow_particles` collects this tick's particle
    /// requests (the tipped colour and the crit trail), cleared by the caller.
    pub fn tick_in_world(
        &mut self,
        world: &dyn crate::item_entity::ItemWorld,
        arrows: &dyn crate::arrow::ArrowWorld,
        arrow_particles: &mut Vec<crate::arrow::ArrowParticle>,
    ) {
        self.use_ticks.clear();
        arrow_particles.clear();
        let EntityStore { slots, riding, .. } = self;
        for slot in slots.iter_mut().flatten() {
            slot.tick_count += 1;
            slot.prev_body_yaw = slot.body_yaw;
            // `LivingEntity.baseTick` records `yHeadRotO`, then `aiStep` applies one
            // `lerpHeadRotationStep` (rotLerp by 1/steps toward the packet's head yaw).
            slot.prev_head_yaw = slot.head_yaw;
            if slot.lerp_head_steps > 0 {
                slot.head_yaw = rot_lerp(
                    1.0 / f32::from(slot.lerp_head_steps),
                    slot.head_yaw,
                    slot.lerp_head_yaw,
                );
                slot.lerp_head_steps -= 1;
            }
            // `AbstractClientPlayer.tick`: the cloak follows the position before this tick's
            // movement (the interpolation step below).
            if slot.is_player_type {
                slot.cloak.tick(slot.pos);
            }
            slot.lerp.tick(&mut LerpTarget {
                pos: &mut slot.pos,
                prev_pos: &mut slot.prev_pos,
                yaw: &mut slot.yaw,
                prev_yaw: &mut slot.prev_yaw,
                pitch: &mut slot.pitch,
                prev_pitch: &mut slot.prev_pitch,
            });
            // `Display.tick`: the render state rebuilt from the synched data that changed.
            if let Some(display) = slot.display.as_mut() {
                display.tick(&slot.metadata, slot.tick_count);
            }
            // `AbstractMinecart.tick` -> `NewMinecartBehavior.tick`: the track walk.
            crate::new_minecart_behavior::tick(slot);
            // `ItemEntity.tick`: gravity, move, drag, between position packets.
            if slot.is_item {
                crate::item_entity::tick(slot, world);
            }
            // `AbstractArrow.tick`: the client-side flight simulation, between position
            // packets (arrows are `SERVER_AND_CLIENT` simulated like items).
            if crate::arrow::is_arrow(slot.kind) {
                crate::arrow::tick(slot, arrows, arrow_particles);
            }
            // `PrimedTnt.tick` runs on the client too: the fuse counts down locally
            // between the server's entity-data syncs (which overwrite it in lockstep).
            if slot.is_tnt {
                slot.metadata.tnt_fuse = slot.metadata.tnt_fuse.saturating_sub(1);
            }
            // `AbstractBoat.tick`'s client-side state: the paddle sweeps and the
            // bubble-column rocking, from the synced flags and bubble time.
            if slot.is_boat {
                let eye_height = self.dims.eye_height(slot.kind, false);
                crate::boat::tick(slot, world, eye_height);
            }
            // `LivingEntity.tick`'s use step (`updatingUsingItem`), after `super.tick()`
            // like vanilla: the tick is recorded for the client's effects.
            crate::using_item::step(slot, &self.dims, &mut self.use_ticks);
            let d = slot.pos - slot.prev_pos;
            let dist = f32::sqrt((d.x * d.x + d.z * d.z) as f32);
            // `LivingEntity.calculateEntityAnimation`: passengers and the dead stop walking.
            let passenger = riding.vehicle_of(slot.id).is_some();
            if !passenger && !slot.dead {
                let target = (dist * 4.0).min(1.0);
                // `LivingEntity.updateWalkAnimation`: babies swing at three times the position
                // scale (`isBaby() ? 3.0F : 1.0F`).
                let baby = !slot.is_player_type && crate::registry::is_baby(&slot.metadata);
                slot.walk.update(target, 0.4, if baby { 3.0 } else { 1.0 });
            } else {
                slot.walk.stop();
            }
            crate::walk_swing::update_body_yaw(slot, d);
            // `LivingEntity.updateSwimAmount` (smoothed 0.09 per tick).
            slot.swim_amount_o = slot.swim_amount;
            if slot.pose == Pose::Swimming {
                slot.swim_amount = (slot.swim_amount + 0.09).min(1.0);
            } else {
                slot.swim_amount = (slot.swim_amount - 0.09).max(0.0);
            }
            // `Chicken.aiStep`'s flap timers (harmless for every other model).
            slot.o_flap = slot.flap;
            slot.o_flap_speed = slot.flap_speed;
            slot.flap_speed += (if slot.on_ground { -1.0 } else { 4.0 }) * 0.3;
            slot.flap_speed = slot.flap_speed.clamp(0.0, 1.0);
            if !slot.on_ground && slot.flapping < 1.0 {
                slot.flapping = 1.0;
            }
            slot.flapping *= 0.9;
            slot.flap += slot.flapping * 2.0;
            slot.swing.tick();
            slot.hurt_ticks = slot.hurt_ticks.saturating_sub(1);
            // `Creeper.tick`: the client advances its own swell count from the synced direction
            // (an ignited creeper forces it to 1), clamped to [0, maxSwell].
            slot.old_swell = slot.swell;
            if slot.metadata.creeper_ignited {
                slot.metadata.creeper_swell_dir = 1;
            }
            slot.swell = (slot.swell + slot.metadata.creeper_swell_dir).clamp(0, CREEPER_MAX_SWELL);
            // `IronGolem.tick`: the attack animation tick counts down.
            slot.attack_ticks = slot.attack_ticks.saturating_sub(1);
            // `EvokerFangs.tick`: the bite's `lifeTicks` counts down once it has started.
            slot.fangs.tick();
            // `AbstractMinecart.tick`'s countdowns (hurt time, damage, the TNT cart's fuse);
            // read only by minecarts, harmless elsewhere.
            crate::minecart::tick(&mut slot.metadata);
            // `LivingEntity.tick`: fallFlyTicks counts while the fall-flying flag is set.
            if slot.metadata.shared.elytra_flying {
                slot.fall_flying_ticks = slot.fall_flying_ticks.saturating_add(1);
            } else {
                slot.fall_flying_ticks = 0;
            }
            slot.age = slot.age.saturating_add(1);
        }
    }

    /// `swing_animation`: an entity swung a hand.
    pub fn swing(&mut self, id: EntityId, hand: i32, animation: i32, duration: i32) {
        if let Some(e) = self.get_mut(id) {
            e.swing.start(hand, animation, duration);
        }
    }

    pub fn get(&self, id: EntityId) -> Option<&Entity> {
        self.slot(id).and_then(|i| self.slots[i as usize].as_ref())
    }

    /// The entity with this id, mutably: the per-entity packet updates.
    pub(crate) fn get_mut(&mut self, id: EntityId) -> Option<&mut Entity> {
        let i = self.slot(id)?;
        self.slots[i as usize].as_mut()
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}
