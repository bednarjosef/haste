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
    // Item particles: a consumable in use (the eat/drink crumbs in front of the mouth).
    let item_requests = std::mem::take(&mut driver.session.item_particle_requests);
    // The arrows' own particles (`AbstractArrow.tick` / `Arrow.makeParticle`): the tipped
    // colour and the crit trail, spawned like the server's would be.
    let arrow_particles = std::mem::take(&mut driver.session.arrow_particles);
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
        for r in &item_requests {
            particles.spawn_item(r.item, r.pos, r.vel);
        }
        for r in &arrow_particles {
            // The tipped colour is an `entity_effect` `ColorParticleOption` (its colour rides
            // the spawn request); the spectral swirl is a white `effect`; a crit trail
            // particle is a plain `crit` with the packet's speed
            // (`addParticle(type, x, y, z, vx, vy, vz)`).
            use haste_entity::arrow::ArrowParticleKind;
            let (kind, color, speed) = match r.kind {
                ArrowParticleKind::Color(argb) => (
                    haste_core::Identifier::minecraft("entity_effect"),
                    Some(argb),
                    [0.0; 3],
                ),
                ArrowParticleKind::SpectralSwirl => (
                    haste_core::Identifier::minecraft("effect"),
                    // `SpellParticleOption.create(EFFECT, -1, 1.0F)`: opaque white.
                    Some(-1),
                    [0.0; 3],
                ),
                ArrowParticleKind::Crit => (
                    haste_core::Identifier::minecraft("crit"),
                    None,
                    [r.vel.x as f32, r.vel.y as f32, r.vel.z as f32],
                ),
            };
            let mut spawn = haste_render::ParticleSpawn::client(kind, r.pos, speed);
            spawn.color = color;
            particles.spawn(&spawn, camera, None);
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
        // `AbstractHurtingProjectile.createParticleTrail`: the fireballs and the wither
        // skull smoke at their position, half a block up, one particle per tick (the dragon
        // fireball's dragon-breath trail is a separate task).
        for pos in shared.entities.read().smoke_trail_positions() {
            // count 0: one particle at the position, no velocity of its own.
            ambient.push(haste_render::ParticleSpawn::client(
                haste_core::Identifier::minecraft("smoke"),
                pos + haste_core::DVec3::new(0.0, 0.5, 0.0),
                [0.0; 3],
            ));
        }
        // The totem pops (`TrackingEmitter.tick`): each emitter follows its entity's box and
        // is dropped once it has lived its 30 ticks, or its entity is gone.
        let entities = shared.entities.read();
        let own = driver.session.entity_id;
        let own_pos = driver.session.player.state.pos;
        let mut totems = std::mem::take(&mut driver.session.totem_emitters);
        totems.retain_mut(|t| {
            let (pos, size) = if t.entity == own {
                // Our own entity is not in the store: the local player's box.
                (own_pos, (0.6, 1.8))
            } else {
                match (entities.get(t.entity), entities.entity_type_info(t.entity)) {
                    (Some(e), Some((_, size))) => (e.pos, size),
                    _ => return false,
                }
            };
            t.tick(pos, size, &mut driver.session.totem_rng, &mut ambient)
        });
        driver.session.totem_emitters = totems;
    }
    for s in &ambient {
        particles.spawn(s, camera, None);
    }
    driver.session.ambient_spawns = ambient;
    particles.tick(camera);
    // The render snapshot for the coming frames (vanilla's per-frame
    // `ParticleEngine.extract` folded into the tick: light sampled at the tick
    // position, see `build_snapshot`), published while the locks are still held —
    // the render thread only ever takes an `Arc` copy.
    let world = shared.world.read();
    particles.publish_snapshot(&*world, &shared.particle_snapshots);
}
