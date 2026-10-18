//! The client's own item particles (vanilla `LivingEntity.spawnItemParticles`): the
//! `minecraft:item` particles (`BreakingItemParticle`) a consuming entity spawns in front
//! of its mouth, queued here for the game thread's particle tick the same way the block
//! particles are (`crate::block_particles`); the renderer draws them
//! (`haste_render::Particles::spawn_item`, whose sprite table carries the item's model
//! sprite).

use haste_audio::random::LegacyRandom;
use haste_core::{DVec3, RegistryIndex};

/// An item particle the client spawns itself, queued for the game thread's tick.
#[derive(Clone, Copy, Debug)]
pub struct ItemParticleRequest {
    /// Spawn position (vanilla `p`: the entity's eye plus the rotated offset).
    pub pos: DVec3,
    /// The velocity vanilla hands `addParticle` (`d`, the `+0.05` on y applied).
    pub vel: [f32; 3],
    /// The item whose sprite the particle shows (`ItemParticleOption`'s
    /// `ItemStackTemplate`).
    pub item: RegistryIndex,
}

/// Where an entity's item particles spawn from (vanilla reads `getX`/`getZ`, `getEyeY`
/// and `getYRot`/`getXRot`): the feet position, the eye height and the rotations.
#[derive(Clone, Copy, Debug)]
pub struct EntityPose {
    /// The entity's feet position (`getX`/`getZ`).
    pub pos: DVec3,
    /// `getEyeY()` (the feet's y plus the pose's eye height).
    pub eye_y: f64,
    /// (`getYRot`, `getXRot`) in degrees.
    pub look: (f32, f32),
}

/// Vanilla `LivingEntity.spawnItemParticles(stack, count)`: `count` item particles in
/// front of the mouth. Their positions and velocities are drawn from the entity's own
/// random — the same source the consume sound's volume and pitch come from
/// (`Consumable.emitParticlesAndSounds` passes `LivingEntity.getRandom()` to both), so
/// the draws interleave in vanilla's order.
pub fn spawn_item_particles(
    random: &mut LegacyRandom,
    item: RegistryIndex,
    count: usize,
    pose: EntityPose,
    out: &mut Vec<ItemParticleRequest>,
) {
    for _ in 0..count {
        // The velocity offset `d` and the position offset `p`, each rotated by -pitch
        // then -yaw (vanilla `Vec3.xRot`/`yRot`); the four draws per particle run in
        // vanilla's order (d.x, d.y, p.y, p.x).
        let d = rotated(
            (random.next_float() as f64 - 0.5) * 0.1,
            random.next_float() as f64 * 0.1 + 0.1,
            0.0,
            pose.look,
        );
        let y1 = -(random.next_float() as f64) * 0.6 - 0.3;
        let p = rotated((random.next_float() as f64 - 0.5) * 0.3, y1, 0.6, pose.look);
        out.push(ItemParticleRequest {
            pos: DVec3::new(pose.pos.x + p.0, pose.eye_y + p.1, pose.pos.z + p.2),
            vel: [d.0 as f32, d.1 as f32 + 0.05, d.2 as f32],
            item,
        });
    }
}

/// Vanilla `Vec3.xRot(-pitch)` then `Vec3.yRot(-yaw)` (degrees, the entity's rotations;
/// the trig itself vanilla does in f32).
fn rotated(x: f64, y: f64, z: f64, look: (f32, f32)) -> (f64, f64, f64) {
    let a = -look.1.to_radians();
    let (cos, sin) = (a.cos() as f64, a.sin() as f64);
    let y1 = y * cos + z * sin;
    let z1 = z * cos - y * sin;
    let b = -look.0.to_radians();
    let (cos, sin) = (b.cos() as f64, b.sin() as f64);
    (x * cos + z1 * sin, y1, z1 * cos - x * sin)
}
