//! Game thread (client-app): the 20 Hz tick that drains the connection into the world, the
//! entity store and the session, and sends this tick's packets. The render thread reads the
//! same state through [`Shared`].
//!
//! Locking: the world and the entity store each sit behind a `parking_lot::RwLock`. The game
//! thread takes the write locks once per tick for the packet drain (never per packet); the
//! render thread takes read locks only to capture dirty sections for the mesher and to copy the
//! entity render instances, never across `Renderer::render`. Everything else the render thread
//! needs per frame is a small `Copy` value in [`Shared::view`].

#[cfg(test)]
use std::time::Duration;
use std::time::Instant;

use glam::DVec3;
#[cfg(test)]
use haste_protocol::haste_core::game::DimensionInfo;
use haste_protocol::haste_core::game::TICK_SECONDS;
#[cfg(test)]
use haste_protocol::haste_core::RegistryIndex;
use haste_protocol::haste_core::SectionPos;
#[cfg(test)]
use haste_protocol::haste_core::{BlockPos, BlockStateId, ChunkPos};

mod block_entities;
mod block_entity_ticks;
mod camera;
mod hud;
mod input;
mod level;
mod particles;
mod run;
mod shared;
mod sound;
mod tracked_world;
mod view;
mod weather;

use crate::net::NetDriver;
#[cfg(test)]
use crate::net::WorldSink;

pub mod skins;

pub use camera::{third_person_camera, Perspective};
pub use run::run;
pub use shared::{PackReload, Shared};
#[cfg(test)]
pub use view::death_time;
pub use view::{player_instance, FirstPersonView, ViewState};

#[cfg(test)]
use camera::THIRD_PERSON_DISTANCE;
#[cfg(test)]
use haste_core::view::DirtyTracker;
#[cfg(test)]
use haste_world::World;
#[cfg(test)]
use run::take_slot;
use run::{drain_packets, HudSlot};
#[cfg(test)]
use tracked_world::TrackedWorld;

/// Fraction of the current tick that has elapsed at `now`, in `0..=1`.
pub fn partial_tick(tick_at: Instant, now: Instant) -> f32 {
    let dt = now.saturating_duration_since(tick_at).as_secs_f64();
    (dt / TICK_SECONDS).clamp(0.0, 1.0) as f32
}

/// Position between the last two ticks (vanilla's `lerp(partialTick, old, new)`).
pub fn interpolate(prev: DVec3, cur: DVec3, partial: f32) -> DVec3 {
    prev.lerp(cur, partial as f64)
}

fn tick_once(
    driver: &mut NetDriver,
    shared: &Shared,
    gone: &mut Vec<SectionPos>,
    jvm: &mut HudSlot,
    server_packs_version: &mut u64,
    extractor: &mut crate::app::block_entity_extract::BlockEntityExtractor,
) -> anyhow::Result<()> {
    // Once the JVM arrives (the session joined before the game thread started): build the
    // `Ui` and give the JVM the registries, tags and the local player. Menu mode's UI arrives
    // pre-built (its JVM opened the title screen); only `joined` runs for it.
    hud::take_arrival(driver, shared, jvm);
    // This tick's UI output (packets vanilla code sent, chat and commands typed).
    let mut ui_out = Vec::new();
    drain_packets(driver, shared, gone, server_packs_version)?;
    // This tick's sound packets, played through the output (when the client set one up).
    // This runs again after `end_tick` below to pick up the player's own block sounds
    // (footsteps, mining, placing) that the player's tick just produced.
    sound::play_packet_sounds(driver, shared);
    // This tick's input, sampled from winit by the render thread.
    let input = input::sample(driver, shared);
    // The horizontal distance travelled this tick, for the hand's walk bob: vanilla's
    // `LocalPlayer.move` adds `length(dx, dz) * 0.6` to `ClientAvatarState.walkDist`.
    let tick_moved = level::tick(driver, shared, &input)?;
    // The player's own block sounds this tick (footsteps, mining, placing), played through
    // the same output the packets went through above.
    sound::play_own_sounds(driver, shared);
    {
        let mut breaking = shared.breaking.lock();
        breaking.clear();
        driver.session.breaking_stages(&mut breaking);
    }
    // The lids' open targets (vanilla routes `block_event` to the block entity).
    block_entities::tick_lids(driver, shared);
    // The skulls' animation counters (vanilla ticks each `SkullBlockEntity.animation`:
    // while the block state is powered the counter grows, powering the jaw/ear swing).
    block_entities::tick_skulls_and_model_anims(driver, shared);
    // The vaults' and spawners' spins (vanilla `VaultClientData.updateDisplayItemSpin` and
    // the spawners' client tick): every vault's displayed item turns 10 degrees a tick, a
    // spawner's display mob turns at the rate its state's spinning mob speed and its remaining
    // spawn delay give (a plain mob spawner only while a player is near it).
    block_entities::tick_spins(driver, shared);
    // Particles: spawn what the packets brought, then one 20 Hz simulation step. The camera
    // for `ClientLevel.doAddParticle`'s 32-block limiter is the player's eye.
    particles::tick(driver, shared);
    hud::tick_ui(driver, shared, &mut ui_out);
    // The per-tick camera effects (FOV modifier smoothing and the hurt and death counters;
    // the render thread interpolates them per frame). The view bobbing's walk distance and
    // bob are the hand's own, ticked below (vanilla's one `ClientAvatarState` serves both).
    driver.session.tick_camera(
        Some(shared.data.as_ref()),
        input.perspective == Perspective::FirstPerson,
    );
    // The lightmap's per-tick state (vanilla `GameRenderer.tick` + the effect durations).
    driver.session.lightmap.tick();
    // The first-person hand's tick, in vanilla's order (`LivingEntity.baseTick`, then
    // `Player.tick`, then `FirstPersonHandsAndItems.tick`, with the walk bob in `aiStep` before
    // them); the interaction step above ran the attacks and uses that started the swings. In
    // spectator the hand is not drawn at all (vanilla hides it there), so it does not advance.
    let first_person = view::tick_hand(driver, shared, tick_moved);
    let s = &driver.session;
    let (prev_eye, prev_yaw, prev_pitch) = s.player.camera(0.0);
    let (eye, yaw, pitch) = s.player.camera(1.0);
    // The sound output's tick: listener from the camera (vanilla's
    // `SoundEngine.updateSource(Camera)` — the third-person perspectives move it off the
    // eye), music situation, then 20 Hz tick.
    sound::tick_listener(driver, shared, &input, eye, yaw, pitch);
    // The view update, the tick's last step: everything the render thread needs per frame is
    // copied into the view state here, bracketed around the tick.
    let feet = view::publish(
        driver,
        shared,
        first_person,
        (prev_eye, prev_yaw, prev_pitch),
        (eye, yaw, pitch),
    );
    // The block entities' render data for the next tick's frames (vanilla's per-tick
    // `extractRenderState`): everything that changes only per tick (block states, lights,
    // payloads, the anims' tick ends) is extracted here, off the render thread; the render
    // thread only folds the partial tick into the animatable fields. Runs after the view
    // update so the camera chunk, view radius and world age are this tick's.
    extractor.extract(shared, feet, shared.view.lock().view_distance, s.world_age);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use haste_physics::testing::{states, TestShapes, TestWorld};

    #[test]
    fn first_person_view_change_detection() {
        // With the skin fixed, view equality is the whole "did the hand change?" check:
        // equal views produce equal hands, differing ones differing hands.
        let skin = haste_entity::skin::steve();
        let skin = &*skin;
        let a = FirstPersonView::sample(&lit(), DVec3::ZERO, Some(RegistryIndex(5)), false);
        let mut b = a;
        b.item = None;
        assert_eq!(a, a);
        assert_ne!(a, b);
        let ha = a.with_skin(skin, true);
        let hb = b.with_skin(skin, true);
        assert_eq!(ha, a.with_skin(skin, true));
        assert_ne!(ha, hb);
        assert_eq!(ha.skin, Some(skin.body.clone()));
        assert!(!ha.slim);
        assert!(ha.sleeve);
        assert_eq!(ha.item, Some(RegistryIndex(5)));
        // Sampled at full sky light.
        assert_eq!(ha.light, (15, 0));
        // A differing light also differs (the eye moved into shade).
        let mut c = a;
        c.light = (4, 7);
        assert_ne!(a, c);
        assert_ne!(ha, c.with_skin(skin, true));
    }

    /// A world at full sky light, for sampling the hand's light.
    fn lit() -> LightWorld {
        LightWorld {
            sky: 15,
            block: 0,
            asked: parking_lot::Mutex::new(Vec::new()),
        }
    }

    #[test]
    fn the_view_carries_the_hand_animation_to_the_renderer() {
        // The per-tick pairs go through to the renderer's hand unchanged, so a swinging hand
        // (or a dipping equip progress, or the walk bob) is a different hand each tick.
        let skin = haste_entity::skin::PlayerSkin::new(
            haste_core::Identifier::minecraft("entity/player/slim/steve"),
            None,
            haste_entity::skin::PlayerModelType::Slim,
            true,
        );
        let mut hand = crate::hand::Hand::new();
        hand.swing();
        let mut v = FirstPersonView::sample(&lit(), DVec3::ZERO, None, false);
        // Two ticks of walking 0.2 blocks with the look at (10, 5) and the hand swinging.
        for _ in 0..2 {
            hand.tick(
                0.2,
                (10.0, 5.0),
                true,
                false,
                crate::hand::HeldStack::of(None),
                4.0,
            );
        }
        v.with_hand(&hand);
        let moving = v.with_skin(&skin, true);
        // The skin's slim arm is passed through.
        assert!(moving.slim);
        // The swing is `SwingState.ticks / 6` through: 0 on the attack tick, 1/6 on the next.
        assert_eq!(v.attack, (0.0, 1.0 / 6.0));
        assert_eq!(moving.attack.start, 0.0);
        assert!((moving.attack.end - 1.0 / 6.0).abs() < 1e-7);
        // Walking feeds the walk bob (the camera's, no longer the hand's own fields); the
        // hand only carries the view lag by half the remaining angle each tick.
        let (bob_o, bob) = hand.bob();
        assert!((bob - 0.04).abs() < 1e-6, "{bob_o} -> {bob}");
        let (walk_o, walk) = hand.walk_dist();
        assert!((walk - 0.24).abs() < 1e-6, "{walk_o} -> {walk}");
        assert!((v.y_bob.1 - 7.5).abs() < 1e-6);
        assert!((v.x_bob.1 - 3.75).abs() < 1e-6);
        // The next tick's step is a different hand, so the renderer is told again.
        hand.tick(
            0.2,
            (10.0, 5.0),
            true,
            false,
            crate::hand::HeldStack::of(None),
            4.0,
        );
        let mut next = v;
        next.with_hand(&hand);
        assert_ne!(next, v);
        assert_ne!(next.with_skin(&skin, true), moving);
    }

    #[test]
    fn first_person_sample_reads_the_eye_block_light() {
        // Sky 12, block 7: sampled at the eye's block, in FirstPerson::light's order.
        let world = LightWorld {
            sky: 12,
            block: 7,
            asked: parking_lot::Mutex::new(Vec::new()),
        };
        let v = FirstPersonView::sample(
            &world,
            DVec3::new(3.2, 65.5, -7.8),
            Some(RegistryIndex(1)),
            false,
        );
        assert_eq!(v.item, Some(RegistryIndex(1)));
        assert_eq!(v.light, (12, 7));
        // The eye at y = 65.5 samples the block containing it, y = 65
        // (`Entity.getLightProbePosition` is the eye position, floored by `BlockPos.containing`),
        // not the block above: in a 2-high tunnel the eye at feet + 1.62 is in the lit upper
        // block and the hand must not take the ceiling's darkness.
        assert_eq!(world.asked.lock().clone(), vec![BlockPos::new(3, 65, -8)]);
    }

    #[test]
    fn first_person_sample_on_fire_forces_full_block_light() {
        // `EntityRenderer.getBlockLightLevel`: 15 while on fire, sky light still from the world.
        let world = LightWorld {
            sky: 9,
            block: 2,
            asked: parking_lot::Mutex::new(Vec::new()),
        };
        let v = FirstPersonView::sample(&world, DVec3::new(0.5, 70.9, 0.5), None, true);
        assert_eq!(v.light, (9, 15));
        let v = FirstPersonView::sample(&world, DVec3::new(0.5, 70.9, 0.5), None, false);
        assert_eq!(v.light, (9, 2));
    }

    /// A world that answers one sky/block light pair and records where it was asked.
    #[derive(Default)]
    struct LightWorld {
        sky: u8,
        block: u8,
        asked: parking_lot::Mutex<Vec<BlockPos>>,
    }

    impl haste_protocol::haste_core::view::BlockView for LightWorld {
        fn block_state(&self, _pos: BlockPos) -> BlockStateId {
            BlockStateId::AIR
        }
        fn is_chunk_loaded(&self, _pos: ChunkPos) -> bool {
            true
        }
        fn sky_light(&self, pos: BlockPos) -> u8 {
            self.asked.lock().push(pos);
            self.sky
        }
        fn block_light(&self, _pos: BlockPos) -> u8 {
            self.block
        }
        fn min_y(&self) -> i32 {
            0
        }
        fn height(&self) -> i32 {
            384
        }
    }

    #[test]
    fn partial_tick_runs_from_zero_to_one_over_a_tick() {
        let t = Instant::now();
        assert_eq!(partial_tick(t, t), 0.0);
        let half = partial_tick(t, t + Duration::from_millis(25));
        assert!((half - 0.5).abs() < 1e-6, "{half}");
        assert_eq!(partial_tick(t, t + Duration::from_millis(80)), 1.0);
        // A clock read before the tick finished counts as the tick start.
        assert_eq!(partial_tick(t + Duration::from_millis(5), t), 0.0);
    }

    #[test]
    fn interpolation_between_ticks() {
        let a = DVec3::new(0.0, 64.0, 0.0);
        let b = DVec3::new(0.4, 64.0, -0.2);
        assert_eq!(interpolate(a, b, 0.0), a);
        assert_eq!(interpolate(a, b, 1.0), b);
        let m = interpolate(a, b, 0.25);
        assert!((m - DVec3::new(0.1, 64.0, -0.05)).length() < 1e-9);
    }

    #[test]
    fn jvm_slot_waits_arrives_and_stays_taken() {
        let (tx, rx) = std::sync::mpsc::channel::<Option<u8>>();
        let mut slot = Some(rx);
        // Still starting: nothing, and the slot stays for the next tick.
        assert_eq!(take_slot(&mut slot), None);
        assert!(slot.is_some());
        // It failed (the thread sent None and ended): nothing now, and nothing ever after.
        tx.send(None).unwrap();
        drop(tx);
        assert_eq!(take_slot(&mut slot), None);
        assert!(slot.is_none());
        // A fresh slot that arrives: the item once, then the slot is spent.
        let (tx, rx) = std::sync::mpsc::channel::<Option<u8>>();
        tx.send(Some(7)).unwrap();
        let mut slot = Some(rx);
        assert_eq!(take_slot(&mut slot), Some(7));
        assert_eq!(take_slot(&mut slot), None);
        assert!(slot.is_none());
    }

    /// Eye at (0.5, 0.5, 0.5). Yaw -90 looks east (+x), yaw 90 west, like
    /// `raycast::look_vector`; the movement sine table is not exactly axis-aligned, so position
    /// assertions carry a small tolerance.
    const EYE: DVec3 = DVec3::new(0.5, 0.5, 0.5);
    const EPS: f64 = 1e-6;

    #[test]
    fn perspective_cycle_is_vanillas_order() {
        use Perspective::*;
        assert_eq!(FirstPerson.next(), ThirdPersonBehind);
        assert_eq!(ThirdPersonBehind.next(), ThirdPersonFront);
        assert_eq!(ThirdPersonFront.next(), FirstPerson);
        assert!(!FirstPerson.draws_player());
        assert!(ThirdPersonBehind.draws_player());
        assert!(ThirdPersonFront.draws_player());
    }

    #[test]
    fn first_person_is_the_eye_itself() {
        let w = TestWorld::new();
        let s = TestShapes::default();
        let (pos, yaw, pitch) =
            third_person_camera(Perspective::FirstPerson, EYE, -90.0, 30.0, &w, &s);
        assert_eq!((pos, yaw, pitch), (EYE, -90.0, 30.0));
    }

    #[test]
    fn behind_no_obstacle_sits_exactly_four_blocks_back() {
        let w = TestWorld::new();
        let s = TestShapes::default();
        let (pos, yaw, pitch) =
            third_person_camera(Perspective::ThirdPersonBehind, EYE, -90.0, 0.0, &w, &s);
        // Looking east: the camera is four blocks west, still looking east.
        assert_eq!((yaw, pitch), (-90.0, 0.0));
        let want = EYE + DVec3::new(-THIRD_PERSON_DISTANCE, 0.0, 0.0);
        assert!((pos - want).length() < EPS, "{pos:?} != {want:?}");
    }

    #[test]
    fn behind_wall_distance_is_measured_straight_line_from_the_eye() {
        let mut w = TestWorld::new();
        // Looking west, so the behind camera goes east; stone cell (2, 0, 0) puts its
        // outline's west face at x = 2, 1.5 blocks ahead of the eye.
        w.set(2, 0, 0, states::STONE);
        let s = TestShapes::default();
        let (pos, yaw, _) =
            third_person_camera(Perspective::ThirdPersonBehind, EYE, 90.0, 0.0, &w, &s);
        assert_eq!(yaw, 90.0);
        // All eight corner rays hit the face: the hit is 1.5 blocks along the ray but
        // sqrt(1.5^2 + 0.1^2 + 0.1^2) = sqrt(2.27) from the eye, and no clearance margin is
        // subtracted, so the camera stops epsilon inside the wall (vanilla's
        // `Camera.getMaxZoom`).
        let want = 0.5 + (2.27f64).sqrt();
        assert!((pos.x - want).abs() < EPS, "{pos:?}");
        assert!(pos.x > 2.0, "{pos:?}");
        assert!(
            (pos.y - 0.5).abs() < EPS && (pos.z - 0.5).abs() < EPS,
            "{pos:?}"
        );
    }

    #[test]
    fn diagonal_wall_takes_the_shortest_of_the_eight_corner_rays() {
        let mut w = TestWorld::new();
        // A diagonal wall one cell behind and to the side: every corner ray meets its x = 1
        // face, but at straight-line distances from the eye of sqrt(0.75), sqrt(0.99) and
        // sqrt(1.31) — the shortest wins.
        w.set(1, 0, 1, states::STONE);
        let s = TestShapes::default();
        let eye = DVec3::new(0.3, 0.5, 0.7);
        let (pos, yaw, pitch) =
            third_person_camera(Perspective::ThirdPersonBehind, eye, 135.0, 0.0, &w, &s);
        assert_eq!((yaw, pitch), (135.0, 0.0));
        let back = -haste_physics::raycast::look_vector(135.0, 0.0);
        let want = eye + back * (0.75f64).sqrt();
        assert!((pos - want).length() < 1e-5, "{pos:?} != {want:?}");
    }

    #[test]
    fn front_mode_faces_back_at_the_player() {
        let w = TestWorld::new();
        let s = TestShapes::default();
        let (pos, yaw, pitch) =
            third_person_camera(Perspective::ThirdPersonFront, EYE, -90.0, 30.0, &w, &s);
        // Looking east and 30 degrees down: the camera sits four blocks along the look, facing
        // back (yaw + 180, pitch negated).
        assert_eq!(yaw, 90.0);
        assert_eq!(pitch, -30.0);
        let dir = haste_physics::raycast::look_vector(-90.0, 30.0);
        let want = EYE + dir * THIRD_PERSON_DISTANCE;
        assert!((pos - want).length() < EPS, "{pos:?} != {want:?}");
    }

    #[test]
    fn a_block_the_eye_starts_inside_pulls_the_camera_to_its_corner() {
        let mut w = TestWorld::new();
        // The eye is inside this block: all eight corner rays start inside its outline, and
        // vanilla counts that as a hit at the corner, 0.1 * sqrt(3) from the eye.
        w.set(0, 0, 0, states::STONE);
        let s = TestShapes::default();
        let (pos, _, _) =
            third_person_camera(Perspective::ThirdPersonBehind, EYE, -90.0, 0.0, &w, &s);
        assert!((pos.x - (0.5 - (0.03f64).sqrt())).abs() < EPS, "{pos:?}");
    }

    /// A forgotten chunk lists every section it had, so the dirty set reaches the renderer
    /// and its resident meshes are freed (vanilla drops the render data with the chunk,
    /// `ClientChunkCache.drop`). Needs the data generator reports in the cache.
    #[test]
    #[ignore = "needs cache/datagen/26.3/reports (GameData::load)"]
    fn tracked_world_lists_gone_sections() {
        let data = crate::net::load_game_data().unwrap();
        let dim = DimensionInfo::overworld();
        let mut world = World::new_with_biomes(dim.clone(), data.clone(), 8);
        let c = haste_world::chunk::synthetic_chunk_data(
            ChunkPos::new(3, -2),
            dim.min_section(),
            dim.section_count(),
            |s, _, _, _| {
                if s < 2 {
                    BlockStateId(1)
                } else {
                    BlockStateId::AIR
                }
            },
        );
        WorldSink::load_chunk(&mut world, &c).unwrap();
        let mut gone = Vec::new();
        let mut tracked = TrackedWorld {
            world: &mut world,
            gone: &mut gone,
        };
        tracked.unload_chunk(ChunkPos::new(3, -2));
        assert!(world.chunk(ChunkPos::new(3, -2)).is_none());
        // Every section of the chunk, exactly once, and nothing else.
        let min = dim.min_section();
        let expect: std::collections::HashSet<SectionPos> = (min..min + dim.section_count() as i32)
            .map(|y| SectionPos::new(3, y, -2))
            .collect();
        assert_eq!(gone.len(), expect.len());
        assert!(gone.iter().all(|p| expect.contains(p)));
        // The world itself dirties only the neighbours: without `gone` riding the dirty
        // set, no packet would ever mention these sections again.
        assert!(world
            .take_dirty_sections()
            .into_iter()
            .all(|p| p.chunk() != ChunkPos::new(3, -2)));
    }
}
