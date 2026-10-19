// `handleExplosion`'s halves as separate checks: the sound request, the explosion
// particle, the knockback and the block particles the level tick spawns. The fakes are
// the two sinks' minimal no-op implementations (most methods have defaults).

use bytes::Bytes;
use glam::DVec3;
use haste_core::game::GameMode;
use haste_protocol::haste_core::chunk::{BlockEntityData, ChunkData, LightData};
use haste_protocol::haste_core::entity::{EntitySpawn, ItemStackData, MetadataEntry, PlayerInfo};
use haste_protocol::haste_core::game::DimensionInfo;
use haste_protocol::haste_core::{BlockPos, BlockStateId, ChunkPos, EntityId};
use haste_protocol::packets::play::clientbound as cb;
use haste_protocol::wire::particle::Particle;
use haste_protocol::{ClientboundPlay, ServerboundPlay, SyncedRegistries};

use super::super::{EntitySink, Session, WorldSink};
use super::explosion_sound_pitch;

/// A world that answers nothing (all air, no physics data): enough for the packet
/// handlers, and the tracker's air test reads it as an empty world.
#[derive(Default)]
struct EmptyWorld;
impl WorldSink for EmptyWorld {
    fn load_chunk(&mut self, _data: &ChunkData) -> anyhow::Result<()> {
        Ok(())
    }
    fn unload_chunk(&mut self, _pos: ChunkPos) {}
    fn set_block(&mut self, _pos: BlockPos, _state: BlockStateId) {}
    fn apply_light(&mut self, _pos: ChunkPos, _light: &LightData) {}
    fn set_block_entity(&mut self, _pos: BlockPos, _data: BlockEntityData) {}
    fn reset(&mut self, _dim: &DimensionInfo) {}
    fn chunk_count(&self) -> usize {
        0
    }
    fn non_empty_sections(&self) -> usize {
        0
    }
}

/// An entity store that records nothing.
#[derive(Default)]
struct EmptyEntities;
impl EntitySink for EmptyEntities {
    fn spawn(&mut self, _s: &EntitySpawn) {}
    fn remove(&mut self, _ids: &[EntityId]) {}
    fn move_delta(&mut self, _id: EntityId, _delta: DVec3, _on_ground: bool) {}
    fn set_rotation(&mut self, _id: EntityId, _yaw: f32, _pitch: f32, _on_ground: bool) {}
    fn teleport(&mut self, _id: EntityId, _pos: DVec3, _yaw: f32, _pitch: f32, _g: bool) {}
    fn set_head_yaw(&mut self, _id: EntityId, _yaw: f32) {}
    fn set_velocity(&mut self, _id: EntityId, _v: DVec3) {}
    fn mark_hurt(&mut self, _id: EntityId) {}
    fn mark_dead(&mut self, _id: EntityId) {}
    fn set_metadata(&mut self, _id: EntityId, _entries: &[MetadataEntry]) {
        let _ = _entries;
    }
    fn set_equipment(&mut self, _id: EntityId, _slot: u8, _item: ItemStackData) {}
    fn player_info(&mut self, _info: &[PlayerInfo]) {
        let _ = _info;
    }
    fn set_passengers(&mut self, _vehicle: EntityId, _passengers: &[EntityId]) {}
    fn pose(&self, _id: EntityId) -> Option<(DVec3, f32, f32)> {
        None
    }
    fn clear(&mut self) {}
    fn tick(&mut self) {}
    fn len(&self) -> usize {
        0
    }
}

/// A session over the fakes, with a three-name particle table (0 `poof`, 1 `explosion`,
/// 2 `explosion_emitter`).
fn session() -> Session {
    use haste_core::Identifier;
    Session::new(
        EntityId(7),
        GameMode::Creative,
        DimensionInfo::overworld(),
        std::sync::Arc::new(vec![
            Identifier::minecraft("poof"),
            Identifier::minecraft("explosion"),
            Identifier::minecraft("explosion_emitter"),
        ]),
    )
}

fn apply(session: &mut Session, p: ClientboundPlay) {
    let mut out = Vec::new();
    session.apply(
        p,
        &mut EmptyWorld,
        &mut EmptyEntities,
        &SyncedRegistries::default(),
        &mut out,
    );
}

/// One `explode` packet with the given particle kind and knockback.
fn explode(kind: i32, knockback: Option<DVec3>, play_sound: bool) -> ClientboundPlay {
    ClientboundPlay::Explode(cb::Explode {
        center: DVec3::new(10.0, 65.0, -5.0),
        radius: 4.0,
        block_count: 8,
        player_knockback: knockback,
        particle: Particle {
            kind,
            data: Bytes::new(),
        },
        sound: haste_protocol::wire::IdOr::Id(0),
        block_particles: Vec::new(),
        play_sound,
    })
}

/// The sound: a local BLOCKS sound at the centre, volume 4, the pitch inside vanilla's
/// `(1 + (r - r) * 0.2) * 0.7` range.
#[test]
fn explosion_plays_the_local_sound() {
    let mut s = session();
    apply(&mut s, explode(2, None, true));
    assert_eq!(s.sound_requests.len(), 1);
    let crate::sound::SoundRequest::Play(p) = &s.sound_requests[0] else {
        panic!("a play request");
    };
    assert_eq!(p.category, 4, "SoundSource.BLOCKS");
    assert_eq!(p.volume, 4.0);
    assert_eq!(
        p.pos,
        crate::sound::SoundPos::Fixed(DVec3::new(10.0, 65.0, -5.0))
    );
    assert!((0.56..=0.84).contains(&p.pitch), "pitch {}", p.pitch);
    // No sound request when the packet's flag is off.
    let mut s = session();
    apply(&mut s, explode(2, None, false));
    assert!(s.sound_requests.is_empty());
}

/// The pitch formula itself, at its ends and centre.
#[test]
fn explosion_sound_pitch_formula() {
    assert!((explosion_sound_pitch(0.0, 0.0) - 0.7).abs() < 1e-6);
    assert!((explosion_sound_pitch(1.0, 0.0) - 0.84).abs() < 1e-6);
    assert!((explosion_sound_pitch(0.0, 1.0) - 0.56).abs() < 1e-6);
    assert!((explosion_sound_pitch(0.5, 0.5) - 0.7).abs() < 1e-6);
}

/// The explosion particle: one spawn at the centre with speed (1, 0, 0), the type
/// resolved against the registry names; an unknown id spawns nothing and does not
/// panic.
#[test]
fn explosion_queues_the_explosion_particle() {
    let mut s = session();
    apply(&mut s, explode(2, None, false));
    assert_eq!(s.particle_spawns.len(), 1);
    let spawn = &s.particle_spawns[0];
    assert_eq!(
        spawn.kind,
        haste_core::Identifier::minecraft("explosion_emitter")
    );
    assert_eq!(spawn.pos, DVec3::new(10.0, 65.0, -5.0));
    assert_eq!(spawn.offset, [1.0, 0.0, 0.0]);
    assert_eq!(spawn.max_speed, [1.0, 0.0, 0.0]);
    assert_eq!(spawn.count, 0);
    let mut s = session();
    apply(&mut s, explode(99, None, false));
    assert!(s.particle_spawns.is_empty());
}

/// The knockback: added to the velocity (`Entity.push`), only when finite, absent when
/// the packet carries none.
#[test]
fn explosion_pushes_the_player() {
    let mut s = session();
    s.player.state.velocity = DVec3::new(0.25, -0.1, 0.0);
    apply(
        &mut s,
        explode(2, Some(DVec3::new(0.5, 0.75, -0.25)), false),
    );
    assert_eq!(s.player.state.velocity, DVec3::new(0.75, 0.65, -0.25));
    // A non-finite impulse changes nothing (vanilla's `push` checks it).
    let mut s = session();
    s.player.state.velocity = DVec3::new(0.25, -0.1, 0.0);
    apply(
        &mut s,
        explode(2, Some(DVec3::new(f64::NAN, 0.0, 0.0)), false),
    );
    assert_eq!(s.player.state.velocity, DVec3::new(0.25, -0.1, 0.0));
    // No knockback field: unchanged.
    let mut s = session();
    apply(&mut s, explode(2, None, false));
    assert_eq!(s.player.state.velocity, DVec3::ZERO);
}

/// The block particles wait for the level tick (`end_tick`), which spawns one per block
/// count into the particle queue; an empty list tracks nothing.
#[test]
fn explosion_block_particles_spawn_on_the_level_tick() {
    let mut s = session();
    // An unknown explosion-particle id, so only the block particles queue.
    let mut p = explode(99, None, false);
    if let ClientboundPlay::Explode(p) = &mut p {
        p.block_particles = vec![cb::ExplosionParticle {
            particle: Particle {
                kind: 0,
                data: Bytes::new(),
            },
            scaling: 1.0,
            speed: 1.0,
            weight: 3,
        }];
    }
    apply(&mut s, p);
    // Nothing spawns before the tick.
    assert!(s.particle_spawns.is_empty());
    let input = haste_core::input::InputState::default();
    let mut out = Vec::<ServerboundPlay>::new();
    s.end_tick(&input, &mut EmptyWorld, &EmptyEntities, None, &mut out);
    assert_eq!(s.particle_spawns.len(), 8, "one per block count");
    for spawn in &s.particle_spawns {
        assert_eq!(spawn.kind, haste_core::Identifier::minecraft("poof"));
        // Inside the explosion's radius of the centre.
        let offset = spawn.pos - DVec3::new(10.0, 65.0, -5.0);
        assert!(offset.length() <= 4.0 + 1e-6, "{}", offset.length());
    }
    // The second tick spawns nothing (the tracker cleared).
    s.particle_spawns.clear();
    s.end_tick(&input, &mut EmptyWorld, &EmptyEntities, None, &mut out);
    assert!(s.particle_spawns.is_empty());
}
