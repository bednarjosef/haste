use super::*;

packet! {
    /// A sound at a position.
    Sound = "minecraft:sound" {
        /// Sound (registry id or inline).
        sound: IdOr<SoundEvent>,
        /// Category.
        category: i32 as VarInt,
        /// X * 8.
        x: i32,
        /// Y * 8.
        y: i32,
        /// Z * 8.
        z: i32,
        /// Volume.
        volume: f32,
        /// Pitch.
        pitch: f32,
        /// Variant seed.
        seed: i64,
    }
}

packet! {
    /// A sound from an entity.
    SoundEntity = "minecraft:sound_entity" {
        /// Sound.
        sound: IdOr<SoundEvent>,
        /// Category.
        category: i32 as VarInt,
        /// Entity.
        entity_id: i32 as VarInt,
        /// Volume.
        volume: f32,
        /// Pitch.
        pitch: f32,
        /// Variant seed.
        seed: i64,
    }
}
packet! {
    /// Particles.
    LevelParticles = "minecraft:level_particles" {
        /// Particle (type and data).
        particle: Particle,
        /// Override the particle limiter.
        override_limiter: bool,
        /// Always visible.
        always_visible: bool,
        /// Position.
        pos: DVec3,
        /// Offset x.
        offset_x: f32,
        /// Offset y.
        offset_y: f32,
        /// Offset z.
        offset_z: f32,
        /// Max speed x.
        max_speed_x: f32,
        /// Max speed y.
        max_speed_y: f32,
        /// Max speed z.
        max_speed_z: f32,
        /// Count.
        count: i32 as VarInt,
        /// 0 default, 1 alternative, 2 alternative with speed.
        randomization: i32 as VarInt,
    }
}
/// Stops sounds by category and/or name.
#[derive(Clone, Debug, PartialEq)]
pub struct StopSound {
    /// Sound category, or all.
    pub source: Option<i32>,
    /// Sound name, or all.
    pub sound: Option<Identifier>,
}
impl Packet for StopSound {
    const NAME: &'static str = "minecraft:stop_sound";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let flags = r.u8()?;
        let source = if flags & 1 != 0 {
            Some(r.var_int()?)
        } else {
            None
        };
        let sound = if flags & 2 != 0 {
            Some(r.identifier()?)
        } else {
            None
        };
        Ok(Self { source, sound })
    }
    fn encode(&self, w: &mut Writer) {
        w.u8(self.source.is_some() as u8 | ((self.sound.is_some() as u8) << 1));
        if let Some(s) = self.source {
            w.var_int(s);
        }
        if let Some(s) = &self.sound {
            w.identifier(s);
        }
    }
}
