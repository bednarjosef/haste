use super::*;

packet! {
    /// Spawns a non-player-controlled entity (and other players).
    AddEntity = "minecraft:add_entity" {
        /// Network id.
        entity_id: i32 as VarInt,
        /// UUID.
        uuid: Uuid,
        /// Index into `minecraft:entity_type`.
        kind: i32 as VarInt,
        /// Position.
        pos: DVec3,
        /// Velocity in blocks per tick.
        velocity: DVec3 as LpVec3,
        /// Pitch in degrees.
        pitch: f32 as Angle,
        /// Yaw in degrees.
        yaw: f32 as Angle,
        /// Head yaw in degrees.
        head_yaw: f32 as Angle,
        /// Type-specific data (object data).
        data: i32 as VarInt,
    }
}
impl AddEntity {
    /// As the shared `EntitySpawn`.
    pub fn spawn(&self) -> EntitySpawn {
        EntitySpawn {
            id: EntityId(self.entity_id),
            uuid: self.uuid,
            kind: RegistryIndex(self.kind as u32),
            pos: self.pos,
            yaw: self.yaw,
            pitch: self.pitch,
            head_yaw: self.head_yaw,
            data: self.data,
            velocity: self.velocity,
        }
    }
}
packet! {
    /// Removes entities.
    RemoveEntities = "minecraft:remove_entities" {
        /// Entity ids.
        entity_ids: Vec<i32> as List<VarInt>,
    }
}
packet! {
    /// Head yaw.
    RotateHead = "minecraft:rotate_head" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// Head yaw in degrees.
        head_yaw: f32 as Angle,
    }
}
packet! {
    /// Entity metadata update.
    SetEntityData = "minecraft:set_entity_data" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// Changed entries.
        metadata: Vec<MetadataEntry> as Metadata,
    }
}

packet! {
    /// Entity velocity.
    SetEntityMotion = "minecraft:set_entity_motion" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// Velocity in blocks per tick.
        velocity: DVec3 as LpVec3,
    }
}

/// Entity equipment: `(slot, item)` pairs; slot 0 main hand, 1 off hand,
/// 2 boots, 3 leggings, 4 chestplate, 5 helmet, 6 body, 7 saddle.
#[derive(Clone, Debug, PartialEq)]
pub struct SetEquipment {
    /// Entity.
    pub entity_id: i32,
    /// Slots and items.
    pub equipment: Vec<(u8, ItemStackData)>,
}
impl Packet for SetEquipment {
    const NAME: &'static str = "minecraft:set_equipment";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let entity_id = r.var_int()?;
        let mut equipment = Vec::new();
        loop {
            let s = r.u8()?;
            equipment.push((s & 0x7f, ItemStackData::read(r)?));
            if s & 0x80 == 0 {
                break;
            }
            if equipment.len() > 64 {
                return Err(DecodeError::TooLarge("equipment list".into()));
            }
        }
        Ok(Self {
            entity_id,
            equipment,
        })
    }
    fn encode(&self, w: &mut Writer) {
        w.var_int(self.entity_id);
        let n = self.equipment.len();
        for (i, (s, item)) in self.equipment.iter().enumerate() {
            w.u8(if i + 1 < n { s | 0x80 } else { *s });
            w.write(item);
        }
    }
}
packet! {
    /// An entity animation (26.3 numbering: 0 leave bed, 1 critical hit,
    /// 2 magic critical hit; arm swings moved to `swing_animation`).
    Animate = "minecraft:animate" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// Animation id.
        animation: u8,
    }
}
packet! {
    /// An entity took damage.
    DamageEvent = "minecraft:damage_event" {
        /// Damaged entity.
        entity_id: i32 as VarInt,
        /// Index into `minecraft:damage_type`.
        source_type: i32 as VarInt,
        /// Responsible entity id + 1, 0 for none.
        source_cause: i32 as VarInt,
        /// Direct entity id + 1, 0 for none.
        source_direct: i32 as VarInt,
        /// Source position (`/damage`).
        source_pos: Option<DVec3>,
    }
}
packet! {
    /// Entity status (animations, effects; meaning per entity type).
    EntityEvent = "minecraft:entity_event" {
        /// Entity.
        entity_id: i32,
        /// Status id.
        status: i8,
    }
}
/// A weighted block particle alternative of an explosion.
#[derive(Clone, Debug, PartialEq)]
pub struct ExplosionParticle {
    /// Particle.
    pub particle: Particle,
    /// Scaling.
    pub scaling: f32,
    /// Speed.
    pub speed: f32,
    /// Weight.
    pub weight: i32,
}
impl Wire for ExplosionParticle {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            particle: r.read()?,
            scaling: r.f32()?,
            speed: r.f32()?,
            weight: r.var_int()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.write(&self.particle);
        w.f32(self.scaling);
        w.f32(self.speed);
        w.var_int(self.weight);
    }
}

packet! {
    /// An explosion.
    Explode = "minecraft:explode" {
        /// Centre.
        center: DVec3,
        /// Radius.
        radius: f32,
        /// Number of affected blocks.
        block_count: i32,
        /// Velocity added to the local player.
        player_knockback: Option<DVec3>,
        /// Explosion particle.
        particle: Particle,
        /// Sound.
        sound: IdOr<SoundEvent>,
        /// Block particle alternatives.
        block_particles: Vec<ExplosionParticle>,
        /// Play the sound (26.3).
        play_sound: bool,
    }
}
packet! {
    /// Opens a mount (horse) inventory.
    MountScreenOpen = "minecraft:mount_screen_open" {
        /// Window id.
        window_id: i32 as VarInt,
        /// Inventory columns.
        columns: i32 as VarInt,
        /// Owner entity.
        entity_id: i32,
    }
}

packet! {
    /// Hurt bobbing animation.
    HurtAnimation = "minecraft:hurt_animation" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// Damage direction yaw.
        yaw: f32,
    }
}
/// Entity target of `player_look_at`.
#[derive(Clone, Debug, PartialEq)]
pub struct LookAtEntity {
    /// Entity.
    pub entity_id: i32,
    /// 0 feet, 1 eyes.
    pub anchor: i32,
}
impl Wire for LookAtEntity {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            entity_id: r.var_int()?,
            anchor: r.var_int()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.var_int(self.entity_id);
        w.var_int(self.anchor);
    }
}

packet! {
    /// Rotates the player to face a point or entity.
    PlayerLookAt = "minecraft:player_look_at" {
        /// 0 feet, 1 eyes.
        from_anchor: i32 as VarInt,
        /// Target point.
        target: DVec3,
        /// Entity target.
        entity: Option<LookAtEntity>,
    }
}

packet! {
    /// Sets the player's rotation.
    PlayerRotation = "minecraft:player_rotation" {
        /// Yaw.
        yaw: f32,
        /// Yaw is relative.
        relative_yaw: bool,
        /// Pitch.
        pitch: f32,
        /// Pitch is relative.
        relative_pitch: bool,
    }
}
packet! {
    /// Removes a mob effect.
    RemoveMobEffect = "minecraft:remove_mob_effect" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// Index into `minecraft:mob_effect`.
        effect: i32 as VarInt,
    }
}
packet! {
    /// Camera entity (spectating).
    SetCamera = "minecraft:set_camera" {
        /// Entity.
        camera_id: i32 as VarInt,
    }
}
packet! {
    /// Leash link.
    SetEntityLink = "minecraft:set_entity_link" {
        /// Leashed entity.
        attached: i32,
        /// Holder, -1 to detach.
        holder: i32,
    }
}
packet! {
    /// Passengers of a vehicle.
    SetPassengers = "minecraft:set_passengers" {
        /// Vehicle.
        vehicle: i32 as VarInt,
        /// Passenger ids.
        passengers: Vec<i32> as List<VarInt>,
    }
}
packet! {
    /// An entity swings a hand (26.3).
    SwingAnimation = "minecraft:swing_animation" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// 0 main hand, 1 off hand.
        hand: i32 as VarInt,
        /// 0 none, 1 whack, 2 stab.
        animation: i32 as VarInt,
        /// Duration in ticks.
        duration: i32 as VarInt,
    }
}
packet! {
    /// Item pickup animation.
    TakeItemEntity = "minecraft:take_item_entity" {
        /// Picked up entity.
        collected: i32 as VarInt,
        /// Collector.
        collector: i32 as VarInt,
        /// Count.
        count: i32 as VarInt,
    }
}
/// An attribute modifier.
#[derive(Clone, Debug, PartialEq)]
pub struct AttributeModifier {
    /// Modifier id.
    pub id: Identifier,
    /// Amount.
    pub amount: f64,
    /// 0 add, 1 multiply base, 2 multiply total.
    pub operation: u8,
}
impl Wire for AttributeModifier {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            id: r.identifier()?,
            amount: r.f64()?,
            operation: r.u8()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.identifier(&self.id);
        w.f64(self.amount);
        w.u8(self.operation);
    }
}

/// An attribute value with modifiers.
#[derive(Clone, Debug, PartialEq)]
pub struct AttributeProperty {
    /// Index into `minecraft:attribute`.
    pub id: i32,
    /// Base value.
    pub value: f64,
    /// Modifiers.
    pub modifiers: Vec<AttributeModifier>,
}
impl Wire for AttributeProperty {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            id: r.var_int()?,
            value: r.f64()?,
            modifiers: r.read()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.var_int(self.id);
        w.f64(self.value);
        w.write(&self.modifiers);
    }
}

packet! {
    /// Entity attributes.
    UpdateAttributes = "minecraft:update_attributes" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// Attributes.
        properties: Vec<AttributeProperty>,
    }
}

packet! {
    /// Adds or updates a mob effect.
    UpdateMobEffect = "minecraft:update_mob_effect" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// Index into `minecraft:mob_effect`.
        effect: i32 as VarInt,
        /// Amplifier (level - 1).
        amplifier: i32 as VarInt,
        /// Duration in ticks, -1 infinite.
        duration: i32 as VarInt,
        /// 0x01 ambient, 0x02 show particles, 0x04 show icon, 0x08 blend.
        flags: u8,
    }
}

packet! {
    /// Projectile acceleration power.
    ProjectilePower = "minecraft:projectile_power" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// Power.
        power: f64,
    }
}
