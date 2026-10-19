// The particle and level-event packets: `level_particles` queued for the game thread's
// particle pool, and `level_event`'s block-particle and block-sound halves.

use haste_protocol::haste_core::{BlockStateId, Identifier};
use haste_protocol::packets::play::clientbound as cb;
use haste_protocol::{ClientboundPlay, SyncedRegistries};

use super::{Session, WorldSink};

impl Session {
    /// The `level_particles` and `level_event` packets; returns the packet when it is neither.
    pub(super) fn apply_particles<W: WorldSink + ?Sized>(
        &mut self,
        p: ClientboundPlay,
        world: &mut W,
        regs: &SyncedRegistries,
    ) -> Option<ClientboundPlay> {
        use ClientboundPlay as P;
        match p {
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
            _ => return Some(p),
        }
        None
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
        let color = color_option(&kind, &p.particle.data);
        self.particle_spawns.push(haste_render::ParticleSpawn {
            kind,
            pos: p.pos,
            offset: [p.offset_x, p.offset_y, p.offset_z],
            max_speed: [p.max_speed_x, p.max_speed_y, p.max_speed_z],
            count: p.count,
            randomization: p.randomization.max(0) as u32,
            override_limiter: p.override_limiter,
            dust,
            color,
            block_state,
        });
    }
}

/// `DustParticleOptions` from a particle's raw data (the ARGB colour int, then the size
/// scale f32); `None` for every other type and for a malformed payload.
pub(super) fn dust_option(kind: &Identifier, data: &[u8]) -> Option<haste_render::DustOption> {
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
pub(super) fn block_state_option(
    kind: &Identifier,
    data: &[u8],
) -> Option<haste_core::BlockStateId> {
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

/// `entity_effect`'s `ColorParticleOption` from a particle's raw data (one ARGB int;
/// `SpellParticle.MobEffectProvider` tints the particle with its colour); `None` for every
/// other type and for a malformed payload.
fn color_option(kind: &Identifier, data: &[u8]) -> Option<[f32; 3]> {
    if kind.path() != "entity_effect" || data.len() < 4 {
        return None;
    }
    let argb = i32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    Some([
        ((argb >> 16) & 0xFF) as f32 / 255.0,
        ((argb >> 8) & 0xFF) as f32 / 255.0,
        (argb & 0xFF) as f32 / 255.0,
    ])
}
