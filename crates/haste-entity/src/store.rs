//! Dense entity storage: `Vec`-backed slots with a free list, `EntityId -> slot` in an ahash map.
//! No per-tick allocation.
use ahash::AHashMap;
use haste_core::entity::{EntitySpawn, ItemStackData, MetadataEntry, PlayerInfo, Pose};
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

/// One tracked entity.
pub struct Entity {
    pub id: EntityId,
    pub uuid: Uuid,
    pub kind: RegistryIndex,
    pub pos: DVec3,
    pub prev_pos: DVec3,
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
    /// Limb-swing phase (accumulates horizontal distance travelled).
    pub limb_swing: f32,
    /// Smoothed horizontal speed (amplitude of the limb swing).
    pub limb_speed: f32,
    /// Ticks of hurt animation left (10 per hit; `hurt_animation`/`damage_event` restart it).
    pub hurt_ticks: u32,
    /// Set on the death event: holds the hurt tint instead of easing out.
    pub dead: bool,
}

impl Entity {
    fn new(s: &EntitySpawn, dims: &EntityDimensions) -> Entity {
        Entity {
            id: s.id,
            uuid: s.uuid,
            kind: s.kind,
            pos: s.pos,
            prev_pos: s.pos,
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
            limb_swing: 0.0,
            limb_speed: 0.0,
            hurt_ticks: 0,
            dead: false,
        }
    }
}

pub struct EntityStore {
    slots: Vec<Option<Entity>>,
    free: Vec<u32>,
    by_id: AHashMap<EntityId, u32>,
    dims: EntityDimensions,
    players: AHashMap<Uuid, PlayerInfo>,
    /// Who rides whom (`set_passengers`).
    riding: Riding,
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
        }
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
                e.head_yaw = yaw;
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
    /// and update the limb-swing phase/amplitude from this tick's horizontal distance.
    pub fn tick(&mut self) {
        for slot in self.slots.iter_mut().flatten() {
            slot.lerp.tick(&mut slot.pos, &mut slot.prev_pos);
            let d = slot.pos - slot.prev_pos;
            let dist = f32::sqrt((d.x * d.x + d.z * d.z) as f32);
            slot.limb_swing += dist * 0.5;
            // Vanilla smooths the amplitude towards distance x 4 by 0.4 each tick.
            slot.limb_speed += (dist * 4.0 - slot.limb_speed) * 0.4;
            slot.hurt_ticks = slot.hurt_ticks.saturating_sub(1);
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
    /// dumped for their vanilla layer (rest pose; animation is a later task), everything else a
    /// generic box from its current dimensions. The head yaw/pitch override only applies where a
    /// part named "head" exists (humanoids and dumped models both have one; the box has none).
    pub fn render_instances(&self, partial_tick: f32, out: &mut Vec<EntityInstance>) {
        for slot in self.slots.iter().flatten() {
            let pos = slot.prev_pos.lerp(slot.pos, partial_tick as f64);
            let (model, has_head) = if slot.is_player_type {
                (crate::models::humanoid(false), true)
            } else {
                // Registry table first (shared per type, head flag precomputed), then the box.
                match crate::registry::model_entry(slot.kind) {
                    Some((head, m)) => (m, head),
                    None => (
                        crate::models::box_for(slot.dimensions.0, slot.dimensions.1),
                        false,
                    ),
                }
            };
            let pose = if slot.is_player_type {
                crate::animation::pose_overrides(slot, partial_tick)
            } else if has_head {
                // Aim the head at (head_yaw, pitch) relative to the body; same clamp as players.
                crate::animation::head_only(slot)
            } else {
                haste_core::model::PoseOverrides::default()
            };
            out.push(EntityInstance {
                id: slot.id,
                model,
                pos,
                yaw: slot.yaw,
                pitch: slot.pitch,
                head_yaw: slot.head_yaw,
                scale: 1.0,
                pose,
                skin: None,
                nametag: slot.nametag.clone(),
                hurt: hurt_strength(slot),
            });
        }
    }
}
