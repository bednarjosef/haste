//! Server-sent particles (`level_particles`) and the client-spawned terrain particles,
//! ported from vanilla 26.3's particle classes (`net.minecraft.client.particle`): a
//! fixed-capacity pool simulated at 20 Hz on the game thread ([`Particles`]), which
//! publishes a render snapshot each tick ([`ParticleSnapshots`]) that the render thread
//! draws as camera-facing quads textured from the particle atlas in one batched draw
//! inside the main pass ([`ParticleRenderer`], reusing the entity shader: sprite rects
//! by textureLoad, vertex light colour, fog) — interpolated by partial tick and culled
//! per particle like vanilla's `QuadParticleGroup.extractRenderState`, without a lock.
//!
//! Per-type behaviour lives in the [`ParticleBehaviour`] table: gravity, friction, lifetime,
//! quad size and its growth over the particle's life, colour, alpha and spawn-time motion.
//! Each entry names the vanilla class it follows (`FlameParticle` for `flame`,
//! `BaseAshSmokeParticle` subclass `SmokeParticle` for `smoke`, ...). Registry ids without
//! an entry use vanilla's generic `Particle` defaults, which is also what mods and vanilla
//! types we have not ported get.
//!
//! `level_particles` semantics follow `ClientPacketListener.handleParticleEvent`:
//! count 0 spawns one particle at the position with velocity `offset * max_speed`; count > 0
//! spawns that many at `pos + dist * gaussian` with velocity `gaussian * max_speed` (or
//! uniform, when the packet's randomization mode says so). `ClientLevel.doAddParticle`
//! drops spawns beyond `sqrt(1024.0)` = 32 blocks of the camera unless the packet overrides
//! the limiter.
//!
//! The terrain particles (vanilla `TerrainParticle`: block breaking's destroy grid,
//! mining cracks) and the item particles (vanilla `BreakingItemParticle`: eating and
//! drinking's crumbs) are the client's own, spawned by [`Particles::spawn_destroy`],
//! [`Particles::spawn_crack`] and [`Particles::spawn_item`] — their sprites come from
//! the block and item tables ([`crate::block_particles`], `item_sprite`) packed into the
//! same atlas. See those modules for the vanilla paths.
//!
//! Not modelled (each a separate later step): block collisions (`Particle.move` with
//! `hasPhysics`, so no `onGround` drag and no `speedUpWhenYMotionIsBlocked` trigger beyond
//! an exactly zero y step), particle options other than `dust` (block states, trail
//! colors, geyser; a server-sent `minecraft:item` particle's stack option is not decoded,
//! so those spawn nothing), the 16384-particle cap with the distance squash
//! (`ParticleGroup`), roll (billboards keep facing the camera), and the per-type light
//! boosts `FlameParticle`/`PortalParticle` add.

mod engine;
mod item_particle;
mod item_sprite;
mod particle;
mod particle_renderer;
mod providers;
mod resources;
mod rng;
mod single_quad_particle;
mod snapshot;
mod terrain_particle;
#[cfg(test)]
mod tests;
mod totem;

use glam::DVec3;
use haste_core::{BlockStateId, Identifier};

pub use engine::Particles;
pub use particle_renderer::ParticleRenderer;
pub use resources::ParticleTypes;
pub use snapshot::{ParticleSnapshot, ParticleSnapshots, ParticleView};

/// Pool capacity (vanilla caps its engine around 16384 with a distance squash we do not
/// model; extra spawns are dropped).
pub const MAX_PARTICLES: usize = 4096;

/// How far from the camera `level_particles` spawns survive (`ClientLevel.doAddParticle`:
/// `distanceToSqr(x, y, z) > 1024.0`).
const SPAWN_DISTANCE_SQ: f64 = 1024.0;

/// A particle spawn request from `level_particles`: the resolved type name and the packet's
/// spawn parameters. The game thread queues these, the shared pool consumes them.
#[derive(Clone, Debug)]
pub struct ParticleSpawn {
    /// Particle type (`minecraft:flame`).
    pub kind: Identifier,
    /// Spawn position.
    pub pos: DVec3,
    /// Offset per axis (spread box for count > 0, direction for count 0).
    pub offset: [f32; 3],
    /// Max speed per axis.
    pub max_speed: [f32; 3],
    /// Count (0: one particle at `pos` with velocity `offset * max_speed`).
    pub count: i32,
    /// Packet randomization mode (0 gaussian, 1 uniform, 2 uniform with speed).
    pub randomization: u32,
    /// The packet overrides `ClientLevel`'s distance limiter.
    pub override_limiter: bool,
    /// `minecraft:dust` option data: ARGB colour and size scale (vanilla's
    /// `DustParticleOptions`), the one particle option whose payload we read.
    pub dust: Option<DustOption>,
    /// `entity_effect`'s `ColorParticleOption` as the raw ARGB int: the colour (and alpha —
    /// vanilla's option carries one, `SpellParticle.MobEffectProvider`'s `setAlpha`) the
    /// particle is tinted with instead of the type's own. The bytes are vanilla's sRGB
    /// values; the pool decodes them to the linear RGB the sRGB target needs (vanilla
    /// writes them raw into a non-converting target).
    pub color: Option<i32>,
    /// `BlockParticleOption`: the block state a `minecraft:block` (or
    /// `block_marker`/`falling_dust`/`dust_pillar`/`block_crumble`) particle is
    /// made of — its sprite and tint replace the type's (vanilla
    /// `TerrainParticle.createTerrainParticle`).
    pub block_state: Option<BlockStateId>,
}

impl ParticleSpawn {
    /// A client-side spawn of one particle at `pos` moving at `offset` blocks a tick
    /// (`ClientLevel.addParticle`: count 0, no options, the 32-block limiter applies).
    pub fn client(kind: Identifier, pos: DVec3, offset: [f32; 3]) -> ParticleSpawn {
        ParticleSpawn {
            kind,
            pos,
            offset,
            max_speed: [1.0, 1.0, 1.0],
            count: 0,
            randomization: 0,
            override_limiter: false,
            dust: None,
            color: None,
            block_state: None,
        }
    }
}

/// `DustParticleOptions`: the colour `DustParticle` tints its quad with, and the size scale
/// (`DustParticleBase` multiplies `quadSize` and `lifetime` with it).
#[derive(Clone, Copy, Debug)]
pub struct DustOption {
    /// Linear RGB colour (vanilla unpacks the packet's ARGB int, alpha ignored).
    pub color: [f32; 3],
    /// Size scale (`0.2..=4.0` in vanilla's codec, 1.0 in practice).
    pub scale: f32,
}
