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
            false,
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
        false,
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
    let (pos, yaw, pitch) = third_person_camera(Perspective::FirstPerson, EYE, -90.0, 30.0, &w, &s);
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
    let (pos, yaw, _) = third_person_camera(Perspective::ThirdPersonBehind, EYE, 90.0, 0.0, &w, &s);
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
    let (pos, _, _) = third_person_camera(Perspective::ThirdPersonBehind, EYE, -90.0, 0.0, &w, &s);
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
