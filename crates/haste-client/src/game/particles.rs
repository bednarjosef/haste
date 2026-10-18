//! The tick's particle step (vanilla `ParticleManager.tick` with `ClientLevel.animateTick`
//! before it): spawn what the packets brought, one 20 Hz simulation step, then publish the
//! render snapshot.

use haste_core::view::BlockView;

use super::shared::Shared;
use crate::net::NetDriver;

/// Particles: spawn what the packets brought, then one 20 Hz simulation step. The camera
/// for `ClientLevel.doAddParticle`'s 32-block limiter is the player's eye.
pub(super) fn tick(driver: &mut NetDriver, shared: &Shared) {
    let mut particles = shared.particles.lock();
    let camera = driver.session.player.camera(1.0).0;
    let spawns = std::mem::take(&mut driver.session.particle_spawns);
    // Block particles: our own break/mining/sprint and the server's level events.
    // The world (state, shapes, biomes) is read here, where the game thread holds it.
    let requests = std::mem::take(&mut driver.session.block_particle_requests);
    // The ambient block particles (`ClientLevel.animateTick`, vanilla runs it
    // right before the particle tick): torch flames, fire smoke, campfires, lava,
    // portals. Rain-gated parts (leaf drip) stay dormant until weather is decoded.
    let mut ambient = std::mem::take(&mut driver.session.ambient_spawns);
    {
        let world = shared.world.read();
        let data = shared.data.clone();
        let biomes = crate::app::biomes::WorldBiomes(&world);
        for s in &spawns {
            particles.spawn(s, camera, Some(&biomes));
        }
        driver.session.particle_spawned = spawns.len();
        let state_count = data.blocks.len();
        for r in &requests {
            match *r {
                crate::block_particles::BlockParticleRequest::Destroy { pos, state } => {
                    // Vanilla's `Block.stateById` answers air (nothing spawns) for
                    // ids the state table does not know.
                    if (state.0 as usize) < state_count {
                        particles.spawn_destroy(pos, state, &data.shapes, &biomes);
                    }
                }
                crate::block_particles::BlockParticleRequest::Crack { pos, face } => {
                    let state = world.block_state(pos);
                    if !haste_physics::BlockPhysics::is_air(data.as_ref(), state) {
                        particles.spawn_crack(pos, face, state, &data.shapes, &biomes);
                    }
                }
                crate::block_particles::BlockParticleRequest::Sprint { feet, velocity } => {
                    // `getOnPosLegacy`: the support block 0.2 below the feet.
                    let support = haste_core::BlockPos::new(
                        feet.x.floor() as i32,
                        (feet.y - 0.2).floor() as i32,
                        feet.z.floor() as i32,
                    );
                    let state = world.block_state(support);
                    if !haste_physics::BlockPhysics::is_air(data.as_ref(), state) {
                        particles.spawn_sprint(feet, velocity, state, &data.shapes, &biomes);
                    }
                }
            }
        }
        if driver.session.player.has_position {
            let feet = haste_core::BlockPos::from_dvec3(driver.session.player.state.pos);
            crate::ambient_particles::animate_tick(
                &*world,
                data.as_ref(),
                feet,
                false,
                &mut ambient,
            );
        }
    }
    for s in &ambient {
        particles.spawn(s, camera, None);
    }
    driver.session.ambient_spawns = ambient;
    particles.tick();
    // The render snapshot for the coming frames (vanilla's per-frame
    // `ParticleEngine.extract` folded into the tick: light sampled at the tick
    // position, see `build_snapshot`), published while the locks are still held —
    // the render thread only ever takes an `Arc` copy.
    let world = shared.world.read();
    particles.publish_snapshot(&*world, &shared.particle_snapshots);
}
