// `ClientExplosionTracker` (vanilla `net.minecraft.client.multiplayer`): the block
// particles an `explode` packet carries (`ClientLevel.trackExplosionEffects` hands them
// over), held until the level's tick, which spawns up to 512 of them — weighted by each
// explosion's block count — into air blocks around the centre, their position and speed
// scaled by the chosen alternative's `ExplosionParticleInfo` fields.

use glam::DVec3;
use haste_audio::random::LegacyRandom;
use haste_core::Identifier;
use haste_protocol::haste_core::{BlockPos, BlockStateId};
use haste_render::{DustOption, ParticleSpawn};

/// Vanilla `ClientExplosionTracker.MAX_PARTICLES_PER_TICK`.
const MAX_PARTICLES_PER_TICK: i32 = 512;

/// One tracked alternative from the packet's weighted list (`Weighted<ExplosionParticleInfo>`),
/// its particle type already resolved against the registry names.
pub(super) struct TrackedParticle {
    /// Resolved particle type; `None` when the id is not in the registry (the spawn is
    /// skipped, the weight still counts, like vanilla's engine finding no provider).
    pub(super) kind: Option<Identifier>,
    /// `ExplosionParticleInfo.scaling`: the particle's distance from the centre, as a
    /// fraction of the drawn radius.
    pub(super) scaling: f32,
    /// `ExplosionParticleInfo.speed`: the velocity scale.
    pub(super) speed: f32,
    /// The weighted-list entry's weight.
    pub(super) weight: i32,
    /// The type's option data (`minecraft:dust` colour, a block state), read at the same
    /// point `handleParticleEvent` reads it.
    pub(super) dust: Option<DustOption>,
    pub(super) block_state: Option<BlockStateId>,
}

impl TrackedParticle {
    /// The spawn the tracker draws for this alternative: `count 0` semantics with the
    /// velocity in `offset` (`ClientLevel.addParticle`'s direct six-argument form).
    fn spawn(&self, pos: DVec3, velocity: DVec3) -> Option<ParticleSpawn> {
        let kind = self.kind.clone()?;
        let mut spawn = ParticleSpawn::client(
            kind,
            pos,
            [velocity.x as f32, velocity.y as f32, velocity.z as f32],
        );
        spawn.dust = self.dust;
        spawn.block_state = self.block_state;
        Some(spawn)
    }
}

/// One tracked explosion (`ClientExplosionTracker.ExplosionInfo`).
struct ExplosionInfo {
    center: DVec3,
    radius: f32,
    block_count: i32,
    particles: Vec<TrackedParticle>,
}

impl ExplosionInfo {
    /// The sum of the weighted list's weights (vanilla's `WeightedList.totalWeight`).
    fn total_weight(&self) -> i64 {
        self.particles.iter().map(|p| p.weight as i64).sum()
    }
}

/// The tracker itself: `track` collects what the packets brought, `tick` (from the level
/// tick, `ClientLevel.tick`) spawns and clears. A tick without explosions touches
/// nothing but the empty-list check.
#[derive(Default)]
pub(super) struct ExplosionTracker {
    explosions: Vec<ExplosionInfo>,
}

impl ExplosionTracker {
    /// `ClientExplosionTracker.track`: an explosion with no block particles is not
    /// tracked (the vanilla list only takes non-empty weighted lists).
    pub(super) fn track(
        &mut self,
        center: DVec3,
        radius: f32,
        block_count: i32,
        particles: Vec<TrackedParticle>,
    ) {
        if !particles.is_empty() {
            self.explosions.push(ExplosionInfo {
                center,
                radius,
                block_count,
                particles,
            });
        }
    }

    /// `ClientExplosionTracker.tick`: up to 512 particles across every tracked
    /// explosion, each one's explosion picked by block count (`WeightedRandom
    /// .getRandomItem` over the infos) and its alternative by the weighted list
    /// (`WeightedList.getRandomOrThrow`). `is_air` is the world's block test.
    pub(super) fn tick(
        &mut self,
        rng: &mut LegacyRandom,
        is_air: impl Fn(BlockPos) -> bool,
        out: &mut Vec<ParticleSpawn>,
    ) {
        let total_blocks: i64 = self.explosions.iter().map(|e| e.block_count as i64).sum();
        // Vanilla throws on a negative total weight; a hostile server's numbers play
        // nothing instead of taking the client down.
        if total_blocks <= 0 {
            self.explosions.clear();
            return;
        }
        let total_particles = total_blocks.min(MAX_PARTICLES_PER_TICK as i64) as i32;
        for _ in 0..total_particles {
            let selection = rng.next_int(total_blocks.min(i32::MAX as i64) as i32);
            if let Some(info) = weighted_pick(&self.explosions, selection, |e| e.block_count) {
                add_particle(info, rng, &is_air, out);
            }
        }
        self.explosions.clear();
    }
}

/// `WeightedRandom.getWeightedItem`: the scan that subtracts each weight from the
/// selection until it goes below zero.
fn weighted_pick<T>(items: &[T], mut selection: i32, weight: impl Fn(&T) -> i32) -> Option<&T> {
    for item in items {
        selection -= weight(item);
        if selection < 0 {
            return Some(item);
        }
    }
    None
}

/// `ClientExplosionTracker.addParticle`: a uniform direction on the sphere and a
/// cbrt-distributed radius give the block the particle launches from (only air blocks
/// launch one), and the drawn speed and the alternative's scales give its position and
/// velocity.
fn add_particle(
    info: &ExplosionInfo,
    rng: &mut LegacyRandom,
    is_air: &impl Fn(BlockPos) -> bool,
    out: &mut Vec<ParticleSpawn>,
) {
    let direction = normalized_direction(
        rng.next_float() * 2.0 - 1.0,
        rng.next_float() * 2.0 - 1.0,
        rng.next_float() * 2.0 - 1.0,
    );
    let radius = rng.next_float().cbrt() * info.radius;
    let local = direction * f64::from(radius);
    let pos = info.center + local;
    // The block test uses the position before the alternative's scaling (vanilla's
    // `pos` is the centre plus the unscaled local offset).
    if !is_air(BlockPos::from_dvec3(pos)) {
        return;
    }
    let speed = block_speed(radius, info.radius, rng.next_float(), rng.next_float());
    let selection = rng.next_int(i32::try_from(info.total_weight()).unwrap_or(i32::MAX));
    if let Some(entry) = weighted_pick(&info.particles, selection, |p| p.weight) {
        let particle_pos = info.center + local * f64::from(entry.scaling);
        let velocity = direction * f64::from(speed * entry.speed);
        if let Some(spawn) = entry.spawn(particle_pos, velocity) {
            out.push(spawn);
        }
    }
}

/// `Vec3.normalize`: the direction of a random point in the unit cube, the zero vector
/// when its length is below vanilla's `1.0E-4` cutoff.
fn normalized_direction(x: f32, y: f32, z: f32) -> DVec3 {
    let v = DVec3::new(f64::from(x), f64::from(y), f64::from(z));
    let length = v.length();
    if length < 1.0e-4 {
        DVec3::ZERO
    } else {
        v / length
    }
}

/// The block particle's speed: `0.5 / (radius / explosion.radius + 0.1) * r * r + 0.3`
/// (faster the closer to the centre the particle starts).
fn block_speed(radius: f32, explosion_radius: f32, r1: f32, r2: f32) -> f32 {
    0.5 / (radius / explosion_radius + 0.1) * r1 * r2 + 0.3
}

#[cfg(test)]
mod tests;
