//! Dense entity storage: `Vec`-backed slots with a free list, `EntityId -> slot` in an ahash map.
//! No per-tick allocation.
use std::collections::HashMap;

use ahash::AHashMap;
use haste_core::entity::{
    EntitySpawn, ItemStackData, MetadataEntry, MetadataValue, PlayerInfo, Pose,
};
use haste_core::ids::{EntityId, RegistryIndex};
use haste_core::model::EntityInstance;
use haste_core::DVec3;
use uuid::Uuid;

use crate::dims::EntityDimensions;
use crate::lerp::LerpState;
use crate::metadata::Metadata;
use crate::riding::Riding;

/// Hurt flash duration in ticks (vanilla's `hurtTime` runs to 10).
pub const HURT_TICKS: u32 = 10;
/// Death animation duration in ticks (vanilla's `deathTime` runs to 20).
pub const DEATH_TICKS: u32 = 20;

/// `Mth.lerp`.
fn lerp(delta: f32, start: f32, end: f32) -> f32 {
    start + delta * (end - start)
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
    pub yaw: f32,
    pub pitch: f32,
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
    /// Whether a `rotate_head` packet moved `head_yaw` since the last tick.
    head_yaw_dirty: bool,
    /// The hand swing state (`LivingEntity.swingState`, started by `swing_animation`).
    pub swing: SwingState,
    /// Smoothed swimming amount (`LivingEntity.swimAmount` / `swimAmountO`), 0..1.
    pub swim_amount: f32,
    pub swim_amount_o: f32,
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
    /// A dropped item entity's stack (metadata index 9, `Item`), if its metadata named one.
    pub item: Option<ItemStackData>,
    /// Ticks since spawn: drives the dropped items' spin and bob.
    pub age: u32,
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
            head_yaw: s.head_yaw,
            velocity: s.velocity,
            on_ground: false,
            pose: Pose::Standing,
            dimensions: dims.get(s.kind, Pose::Standing),
            metadata: Metadata::default(),
            equipment: Vec::new(),
            lerp: LerpState::new(s.pos),
            nametag: None,
            is_player_type: false,
            walk: WalkAnimationState::default(),
            body_yaw: s.yaw,
            prev_body_yaw: s.yaw,
            prev_head_yaw: s.head_yaw,
            head_yaw_dirty: false,
            swing: SwingState::default(),
            swim_amount: 0.0,
            swim_amount_o: 0.0,
            tick_count: 0,
            flap: 0.0,
            o_flap: 0.0,
            flap_speed: 0.0,
            o_flap_speed: 0.0,
            flapping: 1.0,
            hurt_ticks: 0,
            dead: false,
            item: None,
            age: 0,
        }
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
}

/// Hurt flash strength for an entity's remaining hurt ticks, eased out (quadratic, like
/// vanilla's `hurtTime`-driven fade: strong on impact, gone by tick 10). `dead` holds the
/// flash while the death animation's 20 ticks run.
pub fn hurt_strength(e: &Entity) -> f32 {
    if e.dead {
        return if e.hurt_ticks > 0 { 1.0 } else { 0.0 };
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
            riding: Riding::new(),
            painting_variants: HashMap::new(),
        }
    }

    /// Sets the synced `minecraft:painting_variant` table (id -> size/asset), from the server's
    /// `registry_data` at join. Paintings whose variant is absent draw the back only.
    pub fn set_painting_variants(
        &mut self,
        variants: HashMap<u32, crate::decoration::PaintingVariant>,
    ) {
        self.painting_variants = variants;
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
                e.lerp.on_delta(delta);
                e.on_ground = on_ground;
            }
        }
    }

    pub fn set_rotation(&mut self, id: EntityId, yaw: f32, pitch: f32, on_ground: bool) {
        if let Some(i) = self.slot(id) {
            if let Some(e) = &mut self.slots[i as usize] {
                e.yaw = yaw;
                e.pitch = pitch;
                e.on_ground = on_ground;
            }
        }
    }

    /// A teleport abandons interpolation and snaps both `prev_pos` and `pos` (a teleport is a
    /// discontinuity; the renderer must not draw a frame travelling the gap).
    pub fn teleport(&mut self, id: EntityId, pos: DVec3, yaw: f32, pitch: f32, on_ground: bool) {
        if let Some(i) = self.slot(id) {
            if let Some(e) = &mut self.slots[i as usize] {
                e.yaw = yaw;
                e.pitch = pitch;
                e.on_ground = on_ground;
                e.pos = pos;
                e.prev_pos = pos;
                e.lerp.on_teleport(pos);
            }
        }
    }

    pub fn set_head_yaw(&mut self, id: EntityId, yaw: f32) {
        if let Some(i) = self.slot(id) {
            if let Some(e) = &mut self.slots[i as usize] {
                // `LivingEntity.baseTick` sets `yHeadRotO = yHeadRot` before the packet handler
                // runs; capture the previous value once per tick for the render interpolation.
                if !e.head_yaw_dirty {
                    e.prev_head_yaw = e.head_yaw;
                }
                e.head_yaw = yaw;
                e.head_yaw_dirty = true;
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

    /// The death event (entity status 3): holds the hurt tint while the 20-tick death
    /// animation would play, then nothing.
    pub fn mark_dead(&mut self, id: EntityId) {
        if let Some(i) = self.slot(id) {
            if let Some(e) = &mut self.slots[i as usize] {
                e.dead = true;
                e.hurt_ticks = DEATH_TICKS;
            }
        }
    }

    pub fn set_metadata(&mut self, id: EntityId, entries: &[MetadataEntry]) {
        let Some(i) = self.slot(id) else { return };
        let Some(e) = &mut self.slots[i as usize] else {
            return;
        };
        e.metadata.apply(entries);
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

    /// One 20 Hz tick: advance the lerp windows so `prev_pos`/`pos` bracket this tick's step,
    /// update the walk animation, the body yaw (`LivingEntity.tick` -> `calculateEntityAnimation`
    /// and `tickHeadTurn`) and the swing/swim/chicken timers.
    pub fn tick(&mut self) {
        let EntityStore { slots, riding, .. } = self;
        for slot in slots.iter_mut().flatten() {
            slot.tick_count += 1;
            slot.prev_body_yaw = slot.body_yaw;
            if !slot.head_yaw_dirty {
                slot.prev_head_yaw = slot.head_yaw;
            }
            slot.head_yaw_dirty = false;
            slot.lerp.tick(&mut slot.pos, &mut slot.prev_pos);
            let d = slot.pos - slot.prev_pos;
            let dist = f32::sqrt((d.x * d.x + d.z * d.z) as f32);
            // `LivingEntity.calculateEntityAnimation`: passengers and the dead stop walking.
            let passenger = riding.vehicle_of(slot.id).is_some();
            if !passenger && !slot.dead {
                let target = (dist * 4.0).min(1.0);
                slot.walk.update(target, 0.4, 1.0);
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
            let (model, has_head) = if slot.is_player_type {
                (crate::models::humanoid(false), true)
            } else {
                // Registry table first (shared per type, head flag precomputed), then the box.
                // Babies use the row's baby layer, like vanilla's AgeableMobRenderer.submit.
                let baby = crate::registry::entity_type_name(slot.kind)
                    .filter(|_| crate::registry::is_baby(&slot.metadata))
                    .and_then(|n| crate::registry::baby_model_for_name(&n));
                match crate::registry::model_entry(slot.kind) {
                    Some((head, m)) => (baby.unwrap_or(m), head),
                    None => (
                        crate::models::box_for(slot.dimensions.0, slot.dimensions.1),
                        false,
                    ),
                }
            };
            let is_passenger = self.riding.vehicle_of(slot.id).is_some();
            let pose =
                crate::animation::setup_anim(slot, &model, has_head, partial_tick, is_passenger);
            // Living models turn with their body yaw; a non-living box follows its packet yaw.
            let yaw = if has_head {
                slot.body_yaw_at(partial_tick)
            } else {
                slot.yaw
            };
            out.push(EntityInstance {
                id: slot.id,
                model,
                pos,
                yaw,
                pitch: slot.pitch,
                head_yaw: slot.head_yaw,
                scale: 1.0,
                pose,
                skin: None,
                nametag: slot.nametag.clone(),
                hurt: hurt_strength(slot),
                item: item_of(slot, partial_tick),
                decoration,
            });
        }
    }
}
