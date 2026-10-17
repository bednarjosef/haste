//! Dense entity storage: `Vec`-backed slots with a free list, `EntityId -> slot` in an ahash map.
//! No per-tick allocation.
use std::collections::HashMap;
use std::sync::Arc;

use ahash::AHashMap;
use haste_core::entity::{
    EntitySpawn, ItemStackData, MetadataEntry, MetadataValue, PlayerInfo, Pose,
};
use haste_core::ids::{EntityId, RegistryIndex};
use haste_core::math::Vec3;
use haste_core::model::EntityInstance;
use haste_core::DVec3;
use uuid::Uuid;

use crate::dims::EntityDimensions;
use crate::lerp::{LerpState, LerpTarget};
use crate::metadata::Metadata;
use crate::riding::Riding;

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

/// `Mth.lerp`.
fn lerp(delta: f32, start: f32, end: f32) -> f32 {
    start + delta * (end - start)
}

/// `Mth.rotLerp`.
pub(crate) fn rot_lerp(delta: f32, start: f32, end: f32) -> f32 {
    start + delta * wrap_degrees(end - start)
}

/// `Mth.wrapDegrees`.
pub(crate) fn wrap_degrees(mut v: f32) -> f32 {
    v %= 360.0;
    if v >= 180.0 {
        v -= 360.0;
    }
    if v < -180.0 {
        v += 360.0;
    }
    v
}

/// Vanilla `WalkAnimationState`: the smoothed horizontal speed and the position that accumulates
/// it, which drive the limb swing. `LivingEntity.calculateEntityAnimation` feeds it the per-tick
/// horizontal distance x 4 (capped at 1) with a 0.4 smoothing factor
/// (`LivingEntity.updateWalkAnimation`); the renderer reads `position`/`speed` with the partial
/// tick.
#[derive(Clone, Copy, Debug)]
pub struct WalkAnimationState {
    speed_old: f32,
    speed: f32,
    position: f32,
    position_scale: f32,
}

impl Default for WalkAnimationState {
    fn default() -> Self {
        WalkAnimationState {
            speed_old: 0.0,
            speed: 0.0,
            position: 0.0,
            position_scale: 1.0,
        }
    }
}

impl WalkAnimationState {
    /// `WalkAnimationState.update`.
    pub fn update(&mut self, target_speed: f32, factor: f32, position_scale: f32) {
        self.speed_old = self.speed;
        self.speed += (target_speed - self.speed) * factor;
        self.position += self.speed;
        self.position_scale = position_scale;
    }

    /// `WalkAnimationState.stop`.
    pub fn stop(&mut self) {
        self.speed_old = 0.0;
        self.speed = 0.0;
        self.position = 0.0;
    }

    /// `WalkAnimationState.speed(partialTicks)`.
    pub fn speed(&self, partial: f32) -> f32 {
        lerp(partial, self.speed_old, self.speed).min(1.0)
    }

    /// `WalkAnimationState.position(partialTicks)`.
    pub fn position(&self, partial: f32) -> f32 {
        (self.position - self.speed * (1.0 - partial)) * self.position_scale
    }

    /// `WalkAnimationState.isMoving`.
    pub fn is_moving(&self) -> bool {
        self.speed > 1.0e-5
    }
}

/// `LivingEntity.SwingState`: which hand swings, with which animation type and duration, and how
/// far the swing has progressed. `swing_animation` packets start it
/// (`ClientPacketListener.handleSwingAnimation` -> `LivingEntity.swing`).
#[derive(Clone, Copy, Debug, Default)]
pub struct SwingState {
    active: bool,
    hand: i32,
    animation: i32,
    duration: i32,
    ticks: i32,
    progress: f32,
    old_progress: f32,
}

impl SwingState {
    /// `LivingEntity.swing` -> `SwingState.startIfAble`.
    pub fn start(&mut self, hand: i32, animation: i32, duration: i32) {
        if self.active && self.ticks > 0 && self.ticks <= self.duration / 2 {
            return;
        }
        if !self.active
            || self.hand != hand
            || self.animation != animation
            || self.duration != duration
        {
            self.old_progress = 0.0;
            self.progress = 0.0;
        }
        self.hand = hand;
        self.animation = animation;
        self.duration = duration;
        self.active = true;
        self.ticks = 0;
    }

    /// `SwingState.tick`.
    pub fn tick(&mut self) {
        self.old_progress = self.progress;
        if self.active {
            if self.duration > 0 {
                self.progress = (self.ticks as f32 / self.duration as f32).min(1.0);
            }
            if self.ticks > self.duration {
                self.active = false;
                self.progress = 0.0;
            } else {
                self.ticks += 1;
            }
        } else {
            self.progress = 0.0;
        }
    }

    /// `LivingEntity.getSwingAnimation` -> `SwingState.getAnimation`.
    pub fn animation(&self, partial: f32) -> f32 {
        if self.progress >= self.old_progress {
            lerp(partial, self.old_progress, self.progress)
        } else {
            lerp(partial, self.old_progress, self.progress + 1.0)
        }
    }

    pub fn is_swinging(&self) -> bool {
        self.active
    }

    /// `SwingState.animation` (the raw progress, used by `LivingEntity.tick`'s body yaw).
    pub fn progress(&self) -> f32 {
        self.progress
    }

    /// The swinging hand (0 main, 1 off).
    pub fn hand(&self) -> i32 {
        self.hand
    }

    /// The swing animation type (0 none, 1 whack, 2 stab).
    pub fn animation_type(&self) -> i32 {
        self.animation
    }
}

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
    lerp: LerpState,
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
    prev_head_yaw: f32,
    /// `LivingEntity.lerpYHeadRot` / `lerpHeadSteps`: the head yaw a `rotate_head` packet asked
    /// for and the ticks left to ease `head_yaw` onto it (`lerpHeadTo(yaw, 3)`).
    lerp_head_yaw: f32,
    lerp_head_steps: u8,
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
    flapping: f32,
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
    /// A dropped item entity's stack (metadata index 9, `Item`), if its metadata named one.
    pub item: Option<ItemStackData>,
    /// Ticks since spawn: drives the dropped items' spin and bob.
    pub age: u32,
    /// A player's trailing cape point (`ClientAvatarState`'s cloak), ticked for players only.
    pub cloak: crate::avatar::Cloak,
}

impl Entity {
    fn new(s: &EntitySpawn, dims: &EntityDimensions) -> Entity {
        Entity {
            id: s.id,
            uuid: s.uuid,
            kind: s.kind,
            pos: s.pos,
            prev_pos: s.pos,
            data: s.data,
            yaw: s.yaw,
            pitch: s.pitch,
            prev_yaw: s.yaw,
            prev_pitch: s.pitch,
            head_yaw: s.head_yaw,
            velocity: s.velocity,
            on_ground: false,
            pose: Pose::Standing,
            dimensions: dims.get(s.kind, Pose::Standing),
            metadata: Metadata::default(),
            equipment: Vec::new(),
            lerp: LerpState::new(s.pos, s.yaw, s.pitch),
            nametag: None,
            is_player_type: false,
            walk: WalkAnimationState::default(),
            // `LivingEntity.recreateFromPacket`: the body starts facing the packet's head yaw
            // (`yBodyRot = yHeadRot = getYHeadRot()`, the `O` copies the same), not its yaw.
            body_yaw: s.head_yaw,
            prev_body_yaw: s.head_yaw,
            prev_head_yaw: s.head_yaw,
            lerp_head_yaw: s.head_yaw,
            lerp_head_steps: 0,
            swing: SwingState::default(),
            swim_amount: 0.0,
            swim_amount_o: 0.0,
            fall_flying_ticks: 0,
            tick_count: 0,
            flap: 0.0,
            o_flap: 0.0,
            flap_speed: 0.0,
            o_flap_speed: 0.0,
            flapping: 1.0,
            hurt_ticks: 0,
            dead: false,
            swell: 0,
            old_swell: 0,
            attack_ticks: 0,
            item: None,
            age: 0,
            cloak: crate::avatar::Cloak::default(),
        }
    }

    /// `LivingEntity.isDeadOrDying` became true: `deathTime` starts counting on the next tick
    /// (`hurt_ticks` runs the 20-tick death counter down; see `animation::death_time`). A
    /// second trigger (event 3 after the health metadata, or the reverse) changes nothing.
    fn start_death(&mut self) {
        if !self.dead {
            self.dead = true;
            self.hurt_ticks = DEATH_TICKS;
        }
    }

    /// `Entity.getYRot(partialTicks)`: `Mth.rotLerp(partial, yRotO, yRot)`.
    pub fn yaw_at(&self, partial: f32) -> f32 {
        if partial == 1.0 {
            self.yaw
        } else {
            rot_lerp(partial, self.prev_yaw, self.yaw)
        }
    }

    /// `Entity.getXRot(partialTicks)`: `Mth.lerp(partial, xRotO, xRot)`.
    pub fn pitch_at(&self, partial: f32) -> f32 {
        if partial == 1.0 {
            self.pitch
        } else {
            lerp(partial, self.prev_pitch, self.pitch)
        }
    }

    /// The present (pos, yaw, pitch): what `LerpState` keeps for the part a packet lacks.
    fn current(&self) -> (DVec3, f32, f32) {
        (self.pos, self.yaw, self.pitch)
    }

    /// Body yaw between the last two ticks (`Mth.rotLerp(partial, yBodyRotO, yBodyRot)`).
    pub fn body_yaw_at(&self, partial: f32) -> f32 {
        self.prev_body_yaw + partial * wrap_degrees(self.body_yaw - self.prev_body_yaw)
    }

    /// Head yaw between the last two ticks (`Mth.rotLerp(partial, yHeadRotO, yHeadRot)`).
    pub fn head_yaw_at(&self, partial: f32) -> f32 {
        self.prev_head_yaw + partial * wrap_degrees(self.head_yaw - self.prev_head_yaw)
    }
}

/// `LivingEntity.tick`'s body-rotation step: follow the horizontal movement direction, then
/// `tickHeadTurn` (0.3 lerp, 50 degree head/body split).
fn update_body_yaw(e: &mut Entity, d: DVec3) {
    let mut target = e.body_yaw;
    let side_dist = (d.x * d.x + d.z * d.z) as f32;
    if side_dist > 0.002_500_000_2 {
        let walk_direction = (d.z.atan2(d.x) as f32).to_degrees() - 90.0;
        let diff = (wrap_degrees(e.yaw) - walk_direction).abs();
        target = if 95.0 < diff && diff < 265.0 {
            walk_direction - 180.0
        } else {
            walk_direction
        };
    }
    if e.swing.progress() > 0.0 {
        target = e.yaw;
    }
    let body_delta = wrap_degrees(target - e.body_yaw);
    e.body_yaw += body_delta * 0.3;
    let head_diff = wrap_degrees(e.yaw - e.body_yaw);
    let max_head = 50.0;
    if head_diff.abs() > max_head {
        e.body_yaw += head_diff - head_diff.signum() * max_head;
    }
}

/// The `EntityInstance::item` entry for a tracked entity: the stack of a dropped item entity
/// (count included; the renderer draws 1-5 copies from it), aged by the partial tick.
fn item_of(e: &Entity, partial_tick: f32) -> Option<haste_core::model::DroppedItem> {
    let stack = e.item.as_ref()?;
    let item = stack.item?;
    Some(haste_core::model::DroppedItem {
        item,
        count: stack.count,
        age: e.age as f32 + partial_tick,
    })
}

/// Whether a model has a part with this name (humanoid models carry `right_arm`, which only the
/// armed/armoured layers need).
fn model_has_part(part: &haste_core::model::ModelPart, name: &str) -> bool {
    part.name == name || part.children.iter().any(|c| model_has_part(c, name))
}

/// The hands, equipment layers and custom head item of an entity: the item in each hand (main
/// hand -> right, off hand -> left), every armour piece whose item's `equippable` component names
/// the same slot (vanilla `HumanoidArmorLayer.shouldRender`), and the head-slot item when it is
/// not armour (vanilla `CustomHeadLayer`: pumpkins, heads). The equipment asset id comes from the
/// item's default `equippable` component; the dye from the stack's `dyed_color` patch.
fn equipment_of(
    e: &Entity,
    components: &haste_data::ItemComponents,
) -> (
    [Option<RegistryIndex>; 2],
    Vec<haste_core::model::EquipmentPiece>,
    Option<RegistryIndex>,
) {
    use haste_core::model::{EquipmentPiece, EquipmentSlot};
    let mut hands = [None, None];
    let mut equipment = Vec::new();
    let mut head_item = None;
    for (wire, stack) in &e.equipment {
        if stack.is_empty() {
            continue;
        }
        let Some(item) = stack.item else { continue };
        match EquipmentSlot::from_wire(*wire) {
            Some(EquipmentSlot::MainHand) => hands[0] = Some(item),
            Some(EquipmentSlot::OffHand) => hands[1] = Some(item),
            Some(slot) if slot.is_armor() => {
                let equippable = components.equippable(item);
                let is_armor = equippable.map(|x| x.slot) == Some(slot);
                if slot == EquipmentSlot::Head && !is_armor {
                    // Not a helmet: `CustomHeadLayer` draws it on the head instead.
                    head_item = Some(item);
                    continue;
                }
                if !is_armor {
                    continue;
                }
                equipment.push(EquipmentPiece {
                    slot,
                    item: Some(item),
                    asset: equippable.and_then(|x| x.asset.clone()),
                    dye: components.dyed_color(&stack.components_raw),
                });
            }
            _ => {}
        }
    }
    (hands, equipment, head_item)
}

pub struct EntityStore {
    slots: Vec<Option<Entity>>,
    free: Vec<u32>,
    by_id: AHashMap<EntityId, u32>,
    dims: EntityDimensions,
    players: AHashMap<Uuid, PlayerInfo>,
    /// Who rides whom (`set_passengers`).
    riding: Riding,
    /// The synced `minecraft:painting_variant` registry (id -> size/asset), set at join from
    /// the server's `registry_data`; empty when the server sent none.
    painting_variants: HashMap<u32, crate::decoration::PaintingVariant>,
    /// The synced variant registries (cat/wolf/frog/pig/cow/chicken), set at join from the
    /// server's `registry_data`; empty when the server sent none.
    mob_variants: crate::variants::MobVariants,
    /// The world age of the last `set_time` (`NeutralMob.isAngry` compares anger end times
    /// against it); 0 before one arrives.
    world_age: i64,
    /// Item default components (the `equippable` component armour reads), from the datagen
    /// report; empty when absent (armour then draws nothing).
    item_components: haste_data::ItemComponents,
    /// Each player's resolved skin (`PlayerInfo.getSkin`), by profile id; a player without one
    /// wears its default skin (`DefaultPlayerSkin.get`).
    skins: AHashMap<Uuid, Arc<crate::skin::PlayerSkin>>,
}

/// Hurt flash strength for an entity's remaining hurt ticks, eased out (quadratic, like
/// vanilla's `hurtTime`-driven fade: strong on impact, gone by tick 10). A dead entity keeps
/// the flash until the server removes it (`LivingEntityRenderer.extractRenderState`:
/// `hasRedOverlay = hurtTime > 0 || deathTime > 0`, and `deathTime` never resets).
pub fn hurt_strength(e: &Entity) -> f32 {
    if e.dead {
        return 1.0;
    }
    let left = e.hurt_ticks.min(HURT_TICKS) as f32 / HURT_TICKS as f32;
    left * left
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
        }
    }

    /// Sets the item default-components table (the `equippable` component armour reads), loaded
    /// from the datagen report at startup.
    pub fn set_item_components(&mut self, components: haste_data::ItemComponents) {
        self.item_components = components;
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

    fn slot(&self, id: EntityId) -> Option<u32> {
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
        if let Some(i) = self.slot(id) {
            if let Some(e) = &mut self.slots[i as usize] {
                e.lerp.on_delta(delta, e.current());
                e.on_ground = on_ground;
            }
        }
    }

    /// A rotation from a move packet (`ClientPacketListener.handleMoveEntity` ->
    /// `Entity.moveOrInterpolateTo(yRot, xRot)`): eased over 3 ticks with the position.
    pub fn set_rotation(&mut self, id: EntityId, yaw: f32, pitch: f32, on_ground: bool) {
        if let Some(i) = self.slot(id) {
            if let Some(e) = &mut self.slots[i as usize] {
                e.lerp.interpolate_to(None, Some((yaw, pitch)), e.current());
                e.on_ground = on_ground;
            }
        }
    }

    /// An absolute position (`entity_position_sync`, `teleport_entity`):
    /// `ClientPacketListener.handleEntityPositionSync` interpolates it like a move packet
    /// (`Entity.moveOrInterpolateTo`, 3 steps) unless the entity is more than 64 blocks away
    /// (squared distance over `TELEPORT_SNAP_DISTANCE_SQ`), in which case it `snapTo`s: both
    /// `prev_pos` and `pos` jump, so the renderer never draws a frame travelling the gap.
    pub fn teleport(&mut self, id: EntityId, pos: DVec3, yaw: f32, pitch: f32, on_ground: bool) {
        if let Some(i) = self.slot(id) {
            if let Some(e) = &mut self.slots[i as usize] {
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
                    e.lerp
                        .interpolate_to(Some(pos), Some((yaw, pitch)), e.current());
                }
            }
        }
    }

    /// `rotate_head`: `ClientPacketListener.handleRotateMob` -> `LivingEntity.lerpHeadTo(yaw, 3)`.
    /// The head does not turn now; `tick` eases it over the next three ticks.
    pub fn set_head_yaw(&mut self, id: EntityId, yaw: f32) {
        if let Some(i) = self.slot(id) {
            if let Some(e) = &mut self.slots[i as usize] {
                e.lerp_head_yaw = yaw;
                e.lerp_head_steps = HEAD_LERP_STEPS;
            }
        }
    }

    pub fn set_velocity(&mut self, id: EntityId, v: DVec3) {
        if let Some(i) = self.slot(id) {
            if let Some(e) = &mut self.slots[i as usize] {
                e.velocity = v;
            }
        }
    }

    /// `hurt_animation` / `damage_event`: (re)start the 10-tick red flash.
    pub fn mark_hurt(&mut self, id: EntityId) {
        if let Some(i) = self.slot(id) {
            if let Some(e) = &mut self.slots[i as usize] {
                e.hurt_ticks = HURT_TICKS;
            }
        }
    }

    /// The death event (entity status 3, `LivingEntity.handleEntityEvent`): starts the death
    /// animation (`deathTime` counting from the next tick) and holds the hurt tint until the
    /// server's `remove_entities`. Health reaching 0 in the metadata starts it too
    /// (`LivingEntity.isDeadOrDying` -> `tickDeath`); the first of the two wins.
    pub fn mark_dead(&mut self, id: EntityId) {
        if let Some(i) = self.slot(id) {
            if let Some(e) = &mut self.slots[i as usize] {
                e.start_death();
            }
        }
    }

    /// Entity event 4: a melee attack animation (`IronGolem.handleEntityEvent` sets
    /// `attackAnimationTick = 10`; several mobs share the status).
    pub fn mark_attack(&mut self, id: EntityId) {
        if let Some(i) = self.slot(id) {
            if let Some(e) = &mut self.slots[i as usize] {
                e.attack_ticks = 10;
            }
        }
    }

    pub fn set_metadata(&mut self, id: EntityId, entries: &[MetadataEntry]) {
        let Some(i) = self.slot(id) else { return };
        let Some(e) = &mut self.slots[i as usize] else {
            return;
        };
        e.metadata.apply(entries);
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
        // clears the drop.
        if let Some(entry) = e.metadata.get(8) {
            e.item = match &entry.value {
                MetadataValue::ItemStack(s) if !s.is_empty() => Some(s.clone()),
                _ => None,
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
        if let Some(i) = self.slot(id) {
            if let Some(e) = &mut self.slots[i as usize] {
                match e.equipment.iter_mut().find(|(s, _)| *s == slot) {
                    Some(x) => x.1 = item,
                    None => e.equipment.push((slot, item)),
                }
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
            update_body_yaw(slot, d);
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
        if let Some(i) = self.slot(id) {
            if let Some(e) = &mut self.slots[i as usize] {
                e.swing.start(hand, animation, duration);
            }
        }
    }

    pub fn get(&self, id: EntityId) -> Option<&Entity> {
        self.slot(id).and_then(|i| self.slots[i as usize].as_ref())
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Interpolated draw list for the renderer (renderer). Players get the humanoid model (classic
    /// layout; skins arrive with renderer's texture work), known mob types the model the recorder
    /// dumped for their vanilla layer, everything else a generic box from its current dimensions;
    /// a dropped item entity draws as the item itself. Each model's `setupAnim` (walk swing, head
    /// aim, attack, pose) is applied by part name; models with no "head" part (the box) get no
    /// overrides.
    pub fn render_instances(&self, partial_tick: f32, out: &mut Vec<EntityInstance>) {
        for slot in self.slots.iter().flatten() {
            // Hanging entities (item frames, paintings) are drawn specially, and the client
            // moved them to the bounding-box centre vanilla's `recalculateBoundingBox` computes
            // (the spawn packet carries the block position, not the render origin).
            let decoration = crate::decoration::build(
                slot.kind,
                slot.data,
                &slot.metadata,
                &self.painting_variants,
                slot.id.0,
            );
            let pos = if decoration.is_some() {
                crate::decoration::render_center(
                    slot.kind,
                    slot.data,
                    slot.pos,
                    &slot.metadata,
                    &self.painting_variants,
                )
            } else {
                slot.prev_pos.lerp(slot.pos, partial_tick as f64)
            };
            let (model, has_head, overlays) = if slot.is_player_type {
                (crate::models::humanoid(false), true, Default::default())
            } else {
                // Registry table first (shared per type, head flag precomputed), then the box.
                // Babies use the row's baby layer, like vanilla's AgeableMobRenderer.submit.
                // Variant mobs (cat/wolf/frog/pig/cow/chicken) instead resolve their look from
                // the synced variant registry through the entity's metadata, falling back to
                // the table's default row when the variant is unknown or absent.
                let baby = crate::registry::is_baby(&slot.metadata);
                let variant =
                    crate::variants::model_for(slot.kind, &self.mob_variants, &slot.metadata, baby)
                        .or_else(|| {
                            // Metadata-driven variants (axolotl/rabbit/horse/llama/panda/
                            // parrot/fox/bee/strider/mooshroom): the same layer, the variant's
                            // texture.
                            crate::variants::metadata_model_for(
                                slot.kind,
                                &slot.metadata,
                                baby,
                                slot.nametag.as_deref(),
                                self.world_age,
                            )
                        });
                // The tropical fish's shape follows its packed pattern; the shulker's base
                // texture follows its colour byte. Both are plain model swaps, and both
                // must be gated on the entity's own type: their metadata slots (17, 18)
                // collide with other mobs' fields (e.g. a sheep's wool byte sits where a
                // shulker's colour byte would).
                let type_name = crate::registry::entity_type_name(slot.kind);
                let variant = variant.or_else(|| {
                    if type_name.as_deref() == Some("minecraft:tropical_fish") {
                        crate::variants::tropical_fish_model(&slot.metadata)
                    } else {
                        None
                    }
                });
                let variant = match type_name.as_deref() {
                    Some("minecraft:shulker") => {
                        match crate::variants::shulker_texture(&slot.metadata) {
                            Some(texture) => crate::registry::model_for_name("minecraft:shulker")
                                .map(|mut m| {
                                    if m.texture != texture {
                                        let mut c = (*m).clone();
                                        c.texture = texture.clone();
                                        m = std::sync::Arc::new(c);
                                    }
                                    m
                                }),
                            None => variant,
                        }
                    }
                    _ => variant,
                };
                let entry = crate::registry::model_entry(slot.kind);
                let overlays = crate::variants::overlays_for(slot.kind, &slot.metadata, baby);
                match (variant, entry) {
                    // A variant model (already the baby's when `baby`); all six variant mobs
                    // have a head part, so the head aim stays on.
                    (Some(v), _) => (v, true, overlays),
                    (None, Some((head, m))) => {
                        let baby_model = crate::registry::entity_type_name(slot.kind)
                            .filter(|_| baby)
                            .and_then(|n| crate::registry::baby_model_for_name(&n));
                        (baby_model.unwrap_or(m), head, overlays)
                    }
                    (None, None) => (
                        crate::models::box_for(slot.dimensions.0, slot.dimensions.1),
                        false,
                        overlays,
                    ),
                }
            };
            let is_passenger = self.riding.vehicle_of(slot.id).is_some();
            let pose =
                crate::animation::setup_anim(slot, &model, has_head, partial_tick, is_passenger);
            let texture = model.texture.path();
            let body = crate::animation::body_transform(
                slot,
                partial_tick,
                texture.contains("iron_golem"),
            );
            // `CreeperRenderer.scale`'s swell wobble; every other model is uniformly scaled.
            let body_scale = if texture.contains("creeper") {
                crate::animation::creeper_scale(slot, partial_tick)
            } else {
                Vec3::ONE
            };
            // Living models turn with their body yaw; a non-living box follows its yaw
            // (`Entity.getYRot(partialTicks)`).
            let yaw = if has_head {
                slot.body_yaw_at(partial_tick)
            } else {
                slot.yaw_at(partial_tick)
            };
            let type_name = crate::registry::entity_type_name(slot.kind);
            let is_baby = !slot.is_player_type && crate::registry::is_baby(&slot.metadata);
            // `LivingEntityRenderer.getShadowRadius`/`MobRenderer.getShadowRadius`: the
            // renderer's base radius times the living/age scale (0.5 for babies). The player's
            // `AvatarRenderer` uses 0.5.
            let base_shadow = type_name
                .as_deref()
                .map(crate::registry::shadow_radius_for_name)
                .unwrap_or(0.0);
            let shadow_scale = if is_baby { 0.5 } else { 1.0 };
            let shadow_radius = if slot.is_player_type {
                0.5
            } else {
                base_shadow * shadow_scale
            };
            let shadow_strength = type_name
                .as_deref()
                .map(crate::registry::shadow_strength_for_name)
                .unwrap_or(1.0);
            let invisible = slot.metadata.shared.invisible;
            // `EntityRenderer.extractNameTags`/`shouldShowName`: players always show their name
            // (`Player.shouldShowName`), mobs only a visible custom name (`LivingEntity`), both
            // hidden while invisible (`isVisibleToPlayer`).
            let custom_name = slot.metadata.custom_name();
            let nametag = if invisible {
                None
            } else if slot.is_player_type {
                slot.nametag.clone()
            } else if slot.metadata.custom_name_visible() {
                custom_name.or_else(|| slot.nametag.clone())
            } else {
                None
            };
            let name_tag_y = crate::registry::name_tag_offset_for_name(
                type_name.as_deref().unwrap_or(""),
                slot.dimensions.1,
            );
            // Equipment layers (held items, armour) are only drawn on humanoid models
            // (vanilla adds `ItemInHandLayer`/`HumanoidArmorLayer` per renderer type).
            let humanoid = has_head && model_has_part(&model.root, "right_arm");
            let (hands, equipment, head_item) = if humanoid {
                equipment_of(slot, &self.item_components)
            } else {
                ([None, None], Vec::new(), None)
            };
            let mut inst = EntityInstance {
                id: slot.id,
                model,
                pos,
                yaw,
                pitch: slot.pitch_at(partial_tick),
                head_yaw: slot.head_yaw,
                // Slime/magma cube size (`AbstractCubeMob.ID_SIZE`); 1.0 for everything else.
                scale: crate::variants::scale_for(slot.kind, &slot.metadata),
                pose,
                body,
                body_scale,
                skin: None,
                nametag,
                below_name: None,
                shadow_radius,
                shadow_strength,
                invisible,
                is_discrete: slot.metadata.shared.crouching,
                name_tag_y,
                hurt: hurt_strength(slot),
                // Overlay layers (sheep wool, horse markings, collars, the fish pattern):
                // drawn with the same transform and pose after the base model.
                overlays,
                item: item_of(slot, partial_tick),
                decoration,
                hands,
                equipment,
                head_item,
            };
            if slot.is_player_type {
                crate::avatar::dress_tracked(
                    &self.player_skin(slot.uuid),
                    slot,
                    &mut inst,
                    partial_tick,
                );
            }
            out.push(inst);
        }
    }
}
