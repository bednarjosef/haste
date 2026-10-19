// `ClientExplosionTracker`'s numbers: the weighted picks, the 512 cap, the air-block
// gate and the position/speed formulas, all with a seeded random so the draws are
// reproducible.

use glam::DVec3;
use haste_audio::random::LegacyRandom;
use haste_core::{BlockPos, Identifier};
use haste_protocol::haste_core::BlockStateId;
use haste_render::ParticleSpawn;

use super::{block_speed, normalized_direction, weighted_pick, ExplosionTracker, TrackedParticle};

/// One alternative with the given weight, `minecraft:poof` unless `kind` is set.
fn entry(weight: i32) -> TrackedParticle {
    TrackedParticle {
        kind: Some(Identifier::minecraft("poof")),
        scaling: 1.0,
        speed: 1.0,
        weight,
        dust: None,
        block_state: None,
    }
}

/// One tracked explosion: eight blocks, radius 4, one `poof` alternative.
fn tracker_with(block_count: i32) -> ExplosionTracker {
    let mut t = ExplosionTracker::default();
    t.track(
        DVec3::new(10.0, 65.0, -5.0),
        4.0,
        block_count,
        vec![entry(3)],
    );
    t
}

/// `WeightedRandom.getWeightedItem`'s scan: the selection walks the weights in order.
#[test]
fn weighted_pick_scans_in_order() {
    let weights = [5, 2, 3];
    assert_eq!(weighted_pick(&weights, 0, |w| *w), Some(&5));
    assert_eq!(weighted_pick(&weights, 4, |w| *w), Some(&5));
    assert_eq!(weighted_pick(&weights, 5, |w| *w), Some(&2));
    assert_eq!(weighted_pick(&weights, 6, |w| *w), Some(&2));
    assert_eq!(weighted_pick(&weights, 7, |w| *w), Some(&3));
    assert_eq!(weighted_pick(&weights, 9, |w| *w), Some(&3));
    // Past the end (a hostile weight sum): nothing, never a panic.
    assert_eq!(weighted_pick(&weights, 10, |w| *w), None);
}

/// `Vec3.normalize`: unit length except at vanilla's `1.0E-4` cutoff, where it is zero.
#[test]
fn normalized_direction_is_unit_or_zero() {
    let d = normalized_direction(1.0, 2.0, -3.0);
    assert!((d.length() - 1.0).abs() < 1e-6);
    // The direction is preserved.
    assert!(d.x > 0.0 && d.y > 0.0 && d.z < 0.0);
    assert_eq!(normalized_direction(0.0, 0.0, 0.0), DVec3::ZERO);
    assert_eq!(normalized_direction(1e-5, 0.0, 0.0), DVec3::ZERO);
}

/// The block particle's speed: `0.5 / (r + 0.1) * r1 * r2 + 0.3` with `r` the drawn
/// radius as a fraction of the explosion's.
#[test]
fn block_speed_formula() {
    // At the centre (r = 0): 0.5/0.1 * r1 * r2 + 0.3 = 5 * r1 * r2 + 0.3.
    assert!((block_speed(0.0, 4.0, 1.0, 1.0) - 5.3).abs() < 1e-6);
    assert!((block_speed(0.0, 4.0, 0.0, 0.0) - 0.3).abs() < 1e-6);
    // At the edge (r = radius): 0.5/1.1 * r1 * r2 + 0.3.
    assert!((block_speed(4.0, 4.0, 1.0, 1.0) - (0.5 / 1.1 + 0.3)).abs() < 1e-6);
}

/// The tick spawns one particle per block count, inside the radius, only where the
/// block is air, and clears itself.
#[test]
fn tick_spawns_per_block_count_into_air() {
    let mut t = tracker_with(8);
    let mut rng = LegacyRandom::new(0x5eed);
    let mut out = Vec::new();
    let solid = BlockPos::new(11, 65, -5);
    let is_air = |pos: BlockPos| pos != solid;
    t.tick(&mut rng, is_air, &mut out);
    assert_eq!(out.len(), 8, "one spawn per block count, air permitting");
    for spawn in &out {
        assert_eq!(spawn.kind, Identifier::minecraft("poof"));
        let offset = spawn.pos - DVec3::new(10.0, 65.0, -5.0);
        assert!(offset.length() <= 4.0 + 1e-6, "{}", offset.length());
        // Velocity = direction * speed (both scaling 1 here): the speed formula's
        // 0.3..5.3 window, along the unit direction of the offset.
        let len = spawn_velocity_len(spawn);
        assert!((0.3..=5.3).contains(&len), "speed {len}");
        let dir = offset.normalize_or_zero();
        if dir.length_squared() > 0.0 {
            let vel = DVec3::new(
                f64::from(spawn.offset[0]),
                f64::from(spawn.offset[1]),
                f64::from(spawn.offset[2]),
            );
            assert!((vel - dir * len).length() < 1e-6, "along the direction");
        }
    }
    // The tracker cleared: a second tick spawns nothing.
    t.tick(&mut rng, is_air, &mut out);
    assert_eq!(out.len(), 8);
}

/// The offset carries the velocity with `max_speed` 1 (the `count 0` semantics).
fn spawn_velocity_len(spawn: &ParticleSpawn) -> f64 {
    let v = DVec3::new(
        f64::from(spawn.offset[0]),
        f64::from(spawn.offset[1]),
        f64::from(spawn.offset[2]),
    );
    v.length()
}

/// A solid block at the drawn position cancels that particle only.
#[test]
fn solid_blocks_cancel_their_particles() {
    let mut t = tracker_with(64);
    let mut rng = LegacyRandom::new(0x5eed);
    let mut out = Vec::new();
    t.tick(&mut rng, |_| false, &mut out);
    assert!(out.is_empty(), "a solid world spawns nothing");
    // The tracker cleared itself; a fresh explosion into air spawns again.
    t.track(DVec3::new(10.0, 65.0, -5.0), 4.0, 64, vec![entry(3)]);
    t.tick(&mut rng, |_| true, &mut out);
    assert!(!out.is_empty());
}

/// The per-tick cap: 512 even for a huge block count (vanilla
/// `MAX_PARTICLES_PER_TICK`).
#[test]
fn tick_caps_at_512() {
    let mut t = tracker_with(100_000);
    let mut rng = LegacyRandom::new(0x5eed);
    let mut out = Vec::new();
    t.tick(&mut rng, |_| true, &mut out);
    assert_eq!(out.len(), 512);
}

/// Two explosions share the tick's particles by block count; an explosion without block
/// particles is never tracked.
#[test]
fn explosions_share_by_block_count() {
    let mut t = ExplosionTracker::default();
    // An empty weighted list: not tracked at all.
    t.track(DVec3::ZERO, 1.0, 100, Vec::new());
    t.track(DVec3::new(0.0, 0.0, 0.0), 1.0, 3, vec![entry(1)]);
    t.track(DVec3::new(100.0, 0.0, 0.0), 1.0, 1, vec![entry(1)]);
    let mut rng = LegacyRandom::new(0x5eed);
    let mut out = Vec::new();
    t.tick(&mut rng, |_| true, &mut out);
    assert_eq!(out.len(), 4, "3 + 1 block counts");
    for spawn in &out {
        let near_origin = spawn.pos.distance(DVec3::new(0.0, 0.0, 0.0)) <= 1.0;
        let near_far = spawn.pos.distance(DVec3::new(100.0, 0.0, 0.0)) <= 1.0;
        assert!(
            near_origin || near_far,
            "one of the two centres: {}",
            spawn.pos
        );
    }
}

/// The weighted list picks by weight: a zero-weight alternative is never drawn, an
/// unknown type's weight still counts (its spawn is skipped, like vanilla's engine
/// finding no provider).
#[test]
fn alternatives_pick_by_weight() {
    let mut t = ExplosionTracker::default();
    let mut unknown = entry(0);
    unknown.kind = None;
    t.track(
        DVec3::new(0.0, 0.0, 0.0),
        1.0,
        40,
        vec![
            unknown,
            TrackedParticle {
                kind: Some(Identifier::minecraft("explosion")),
                scaling: 0.5,
                speed: 2.0,
                weight: 5,
                dust: None,
                block_state: None,
            },
        ],
    );
    let mut rng = LegacyRandom::new(0x5eed);
    let mut out = Vec::new();
    t.tick(&mut rng, |_| true, &mut out);
    assert!(!out.is_empty());
    for spawn in &out {
        assert_eq!(spawn.kind, Identifier::minecraft("explosion"));
        // scaling 0.5: the position is at most half the drawn radius from the centre.
        let offset = spawn.pos - DVec3::new(0.0, 0.0, 0.0);
        assert!(offset.length() <= 0.5 + 1e-6, "{}", offset.length());
    }
    // A block-state option rides along into the spawn.
    let mut t = ExplosionTracker::default();
    let mut blocky = entry(1);
    blocky.block_state = Some(BlockStateId(1));
    t.track(DVec3::new(0.0, 0.0, 0.0), 1.0, 4, vec![blocky]);
    let mut rng = LegacyRandom::new(0x5eed);
    let mut out = Vec::new();
    t.tick(&mut rng, |_| true, &mut out);
    assert_eq!(out.len(), 4);
    for spawn in &out {
        assert_eq!(spawn.block_state, Some(BlockStateId(1)));
    }
}
