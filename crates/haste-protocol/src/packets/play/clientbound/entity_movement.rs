use super::*;

/// The position part of `entity_position_sync`.
#[derive(Clone, Debug, PartialEq)]
pub enum PositionPath {
    /// One absolute end position.
    Linear(DVec3),
    /// Absolute positions with tick offsets.
    Stepped(Vec<(DVec3, i32)>),
}

/// Synchronises an entity's absolute position and rotation (the packet
/// Mojang renamed from `teleport_entity` in 1.21.2).
#[derive(Clone, Debug, PartialEq)]
pub struct EntityPositionSync {
    /// Entity.
    pub entity_id: i32,
    /// Position path.
    pub path: PositionPath,
    /// Yaw in degrees.
    pub yaw: f32,
    /// Pitch in degrees.
    pub pitch: f32,
    /// On ground.
    pub on_ground: bool,
}
impl EntityPositionSync {
    /// The final position of the path.
    pub fn end_pos(&self) -> Option<DVec3> {
        match &self.path {
            PositionPath::Linear(p) => Some(*p),
            PositionPath::Stepped(s) => s.last().map(|(p, _)| *p),
        }
    }
}
impl Packet for EntityPositionSync {
    const NAME: &'static str = "minecraft:entity_position_sync";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let entity_id = r.var_int()?;
        let path = match r.var_int()? {
            0 => PositionPath::Linear(r.read()?),
            1 => PositionPath::Stepped(r.list(4096, |r| Ok((r.read::<DVec3>()?, r.var_int()?)))?),
            t => return Err(DecodeError::Invalid(format!("position path type {t}"))),
        };
        Ok(Self {
            entity_id,
            path,
            yaw: r.f32()?,
            pitch: r.f32()?,
            on_ground: r.bool()?,
        })
    }
    fn encode(&self, w: &mut Writer) {
        w.var_int(self.entity_id);
        match &self.path {
            PositionPath::Linear(p) => {
                w.var_int(0);
                w.write(p);
            }
            PositionPath::Stepped(s) => {
                w.var_int(1);
                w.list(s, |w, (p, t)| {
                    w.write(p);
                    w.var_int(*t);
                });
            }
        }
        w.f32(self.yaw);
        w.f32(self.pitch);
        w.bool(self.on_ground);
    }
}
/// Entity movement deltas: one linear delta or stepped deltas (26.3).
/// Units are 1/4096 block.
#[derive(Clone, Debug, PartialEq)]
pub enum MoveDeltas {
    /// One delta.
    Linear([i16; 3]),
    /// Deltas with tick offsets.
    Stepped(Vec<(i32, [i16; 3])>),
}
impl MoveDeltas {
    /// Sum of all deltas in blocks. Approximate for `Stepped`: the wiki does
    /// not say whether step deltas are relative to the previous step (assumed).
    pub fn total(&self) -> DVec3 {
        let to = |d: &[i16; 3]| DVec3::new(d[0] as f64, d[1] as f64, d[2] as f64) / 4096.0;
        match self {
            MoveDeltas::Linear(d) => to(d),
            MoveDeltas::Stepped(s) => s.iter().map(|(_, d)| to(d)).sum(),
        }
    }
    fn read(r: &mut Reader<'_>) -> DecodeResult<(Self, bool)> {
        let props = r.var_int()?;
        let on_ground = props & 1 != 0;
        let steps = (props as u32 >> 1) as usize;
        let d3 =
            |r: &mut Reader<'_>| -> DecodeResult<[i16; 3]> { Ok([r.i16()?, r.i16()?, r.i16()?]) };
        let m = if steps == 0 {
            MoveDeltas::Linear(d3(r)?)
        } else {
            if steps > 4096 {
                return Err(DecodeError::TooLarge(format!("{steps} movement steps")));
            }
            let mut v = Vec::with_capacity(steps);
            for _ in 0..steps {
                v.push((r.var_int()?, d3(r)?));
            }
            MoveDeltas::Stepped(v)
        };
        Ok((m, on_ground))
    }
    fn write(&self, on_ground: bool, w: &mut Writer) {
        let d3 = |w: &mut Writer, d: &[i16; 3]| d.iter().for_each(|v| w.i16(*v));
        match self {
            MoveDeltas::Linear(d) => {
                w.var_int(on_ground as i32);
                d3(w, d);
            }
            MoveDeltas::Stepped(s) => {
                w.var_int(((s.len() as i32) << 1) | on_ground as i32);
                for (t, d) in s {
                    w.var_int(*t);
                    d3(w, d);
                }
            }
        }
    }
}

/// Small entity move.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveEntityPos {
    /// Entity.
    pub entity_id: i32,
    /// Deltas.
    pub deltas: MoveDeltas,
    /// On ground.
    pub on_ground: bool,
}
impl Packet for MoveEntityPos {
    const NAME: &'static str = "minecraft:move_entity_pos";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let entity_id = r.var_int()?;
        let (deltas, on_ground) = MoveDeltas::read(r)?;
        Ok(Self {
            entity_id,
            deltas,
            on_ground,
        })
    }
    fn encode(&self, w: &mut Writer) {
        w.var_int(self.entity_id);
        self.deltas.write(self.on_ground, w);
    }
}

/// Small entity move with rotation.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveEntityPosRot {
    /// Entity.
    pub entity_id: i32,
    /// Deltas.
    pub deltas: MoveDeltas,
    /// On ground.
    pub on_ground: bool,
    /// Yaw in degrees.
    pub yaw: f32,
    /// Pitch in degrees.
    pub pitch: f32,
}
impl Packet for MoveEntityPosRot {
    const NAME: &'static str = "minecraft:move_entity_pos_rot";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let entity_id = r.var_int()?;
        let (deltas, on_ground) = MoveDeltas::read(r)?;
        Ok(Self {
            entity_id,
            deltas,
            on_ground,
            yaw: r.angle()?,
            pitch: r.angle()?,
        })
    }
    fn encode(&self, w: &mut Writer) {
        w.var_int(self.entity_id);
        self.deltas.write(self.on_ground, w);
        w.angle(self.yaw);
        w.angle(self.pitch);
    }
}

packet! {
    /// Entity rotation.
    MoveEntityRot = "minecraft:move_entity_rot" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// On ground (before the angles in 26.3 per the wiki).
        on_ground: bool,
        /// Yaw in degrees.
        yaw: f32 as Angle,
        /// Pitch in degrees.
        pitch: f32 as Angle,
    }
}
packet! {
    /// Teleports an entity without resetting its delta base (vehicles).
    TeleportEntity = "minecraft:teleport_entity" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// Position.
        pos: DVec3,
        /// Velocity.
        velocity: DVec3,
        /// Yaw.
        yaw: f32,
        /// Pitch.
        pitch: f32,
        /// [`teleport_flags`] bits.
        flags: i32,
        /// On ground.
        on_ground: bool,
    }
}
/// One step of `move_minecart_along_track`.
#[derive(Clone, Debug, PartialEq)]
pub struct MinecartStep {
    /// Position.
    pub pos: DVec3,
    /// Velocity.
    pub velocity: DVec3,
    /// Yaw.
    pub yaw: f32,
    /// Pitch.
    pub pitch: f32,
    /// Weight.
    pub weight: f32,
}
impl Wire for MinecartStep {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            pos: r.read()?,
            velocity: r.read()?,
            yaw: r.angle()?,
            pitch: r.angle()?,
            weight: r.f32()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.write(&self.pos);
        w.write(&self.velocity);
        w.angle(self.yaw);
        w.angle(self.pitch);
        w.f32(self.weight);
    }
}

packet! {
    /// Minecart movement along a track (new minecart physics).
    MoveMinecartAlongTrack = "minecraft:move_minecart_along_track" {
        /// Minecart.
        entity_id: i32 as VarInt,
        /// Steps.
        steps: Vec<MinecartStep>,
    }
}

packet! {
    /// Teleports the client-controlled vehicle.
    MoveVehicle = "minecraft:move_vehicle" {
        /// Position.
        pos: DVec3,
        /// Yaw.
        yaw: f32,
        /// Pitch.
        pitch: f32,
    }
}
