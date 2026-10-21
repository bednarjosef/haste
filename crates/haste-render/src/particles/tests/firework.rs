use haste_core::Identifier;

use super::*;
use crate::particles::firework::{self, Firework, Kind, Starter};
use crate::particles::{FireworkExplosion, FireworkShape};

// --- Vanilla-number tests: the firework starter, sparks, trails and twinkle as vanilla has them. ---

fn firework_kind() -> Kind {
    Kind {
        index: 0,
        behaviour: crate::particles::providers::ParticleBehaviour::of(&Identifier::minecraft(
            "firework",
        )),
    }
}

fn explosion(shape: FireworkShape, twinkle: bool, trail: bool) -> FireworkExplosion {
    FireworkExplosion {
        shape,
        colors: vec![0xFF_00_00],
        fade_colors: vec![0x00_00_FF],
        trail,
        twinkle,
    }
}

/// Ticks a starter until it is done, returning how many ticks it ran and every particle it
/// emitted.
fn run(starter: &mut Starter, rng: &mut Rng) -> (i32, Vec<ParticleState>) {
    let mut out = Vec::new();
    let mut ticks = 0;
    while starter.tick(&mut out, rng) {
        ticks += 1;
    }
    (ticks + 1, out)
}

#[test]
fn starter_lasts_two_ticks_per_explosion_less_one() {
    let mut rng = Rng(7);
    let two = [
        explosion(FireworkShape::SmallBall, false, false),
        explosion(FireworkShape::Star, false, false),
    ];
    let mut starter = Starter::new(DVec3::ZERO, DVec3::ZERO, &two, firework_kind(), None);
    // `explosions.size() * 2 - 1` = 3 life ticks; the tick that finds life past it is the last.
    assert_eq!(run(&mut starter, &mut rng).0, 4);
}

#[test]
fn twinkling_starter_lasts_fifteen_ticks_longer() {
    let mut rng = Rng(7);
    let one = [explosion(FireworkShape::Burst, true, false)];
    let mut starter = Starter::new(DVec3::ZERO, DVec3::ZERO, &one, firework_kind(), None);
    // 1 * 2 - 1 + 15 = 16 life ticks, so 17 ticks run in all.
    assert_eq!(run(&mut starter, &mut rng).0, 17);
}

#[test]
fn burst_emits_seventy_sparks_on_the_first_tick_only() {
    let mut rng = Rng(11);
    let one = [explosion(FireworkShape::Burst, false, false)];
    let mut starter = Starter::new(DVec3::ZERO, DVec3::ZERO, &one, firework_kind(), None);
    let mut out = Vec::new();
    starter.tick(&mut out, &mut rng);
    assert_eq!(out.len(), 70);
    // Life 1 is odd: no explosion, so nothing more is emitted.
    starter.tick(&mut out, &mut rng);
    assert_eq!(out.len(), 70);
}

#[test]
fn flash_is_emitted_with_the_first_colour_on_each_explosion() {
    let mut rng = Rng(5);
    let two = [
        explosion(FireworkShape::Burst, false, false),
        explosion(FireworkShape::Burst, false, false),
    ];
    let flash = Kind {
        index: 1,
        behaviour: crate::particles::providers::ParticleBehaviour::of(&Identifier::minecraft(
            "flash",
        )),
    };
    let mut starter = Starter::new(DVec3::ZERO, DVec3::ZERO, &two, firework_kind(), Some(flash));
    let (_, out) = run(&mut starter, &mut rng);
    let flashes = out
        .iter()
        .filter(|p| matches!(p.firework, Some(Firework::Flash)))
        .count();
    assert_eq!(flashes, 2);
}

#[test]
fn spark_lifetime_is_forty_eight_to_fifty_nine() {
    let mut rng = Rng(3);
    let one = [explosion(FireworkShape::SmallBall, false, false)];
    let mut starter = Starter::new(DVec3::ZERO, DVec3::ZERO, &one, firework_kind(), None);
    let mut out = Vec::new();
    starter.tick(&mut out, &mut rng);
    assert!(!out.is_empty());
    for p in &out {
        // SparkParticle: 48 + nextInt(12).
        assert!((48.0..=59.0).contains(&p.lifetime), "life {}", p.lifetime);
    }
}

#[test]
fn twinkle_hides_the_odd_thirds_after_the_first_third() {
    let mut rng = Rng(9);
    let one = [explosion(FireworkShape::SmallBall, true, false)];
    let mut starter = Starter::new(DVec3::ZERO, DVec3::ZERO, &one, firework_kind(), None);
    let mut out = Vec::new();
    starter.tick(&mut out, &mut rng);
    let mut spark = out.swap_remove(0);
    spark.lifetime = 60.0;
    // Before a third of the life: shown.
    for age in 0..20 {
        spark.age = age as f32;
        assert!(!firework::hidden(&spark), "age {age}");
    }
    // From a third on, hidden where `(age + 60) / 3` is odd: 21 (27), 24 is shown (28),
    // 27 hidden (29), 30 shown (30).
    for (age, hidden) in [(20, false), (21, true), (24, false), (27, true), (30, false)] {
        spark.age = age as f32;
        assert_eq!(firework::hidden(&spark), hidden, "age {age}");
    }
}

#[test]
fn non_twinkling_sparks_are_never_hidden() {
    let mut rng = Rng(13);
    let one = [explosion(FireworkShape::SmallBall, false, false)];
    let mut starter = Starter::new(DVec3::ZERO, DVec3::ZERO, &one, firework_kind(), None);
    let mut out = Vec::new();
    starter.tick(&mut out, &mut rng);
    for mut spark in out {
        for age in 0..60 {
            spark.age = age as f32;
            assert!(!firework::hidden(&spark));
        }
    }
}

#[test]
fn trail_children_appear_on_even_ticks_before_half_life() {
    let mut rng = Rng(21);
    let one = [explosion(FireworkShape::SmallBall, false, true)];
    let mut starter = Starter::new(DVec3::ZERO, DVec3::ZERO, &one, firework_kind(), None);
    let mut out = Vec::new();
    starter.tick(&mut out, &mut rng);
    let spark = out.swap_remove(0);
    let life = spark.lifetime as i32;
    let mut parent = spark;
    parent.age = 0.0;
    let mut pending = Vec::new();
    for _ in 1..=60 {
        parent.age += 1.0;
        firework::tick(&mut parent, &mut pending, &mut rng);
    }
    // SparkParticle.tick: a child every tick with age < life / 2 and (age + life) even.
    let expected = (1..=60)
        .filter(|&a| a < life / 2 && (a + life) % 2 == 0)
        .count();
    assert_eq!(pending.len(), expected);
    for child in &pending {
        // The child starts half done, still, at the parent's spot, with no trail of its own.
        assert_eq!(child.vel, glam::Vec3::ZERO);
        assert_eq!(child.age, (child.lifetime as i32 / 2) as f32);
        assert!(matches!(
            child.firework,
            Some(Firework::Spark { trail: false, .. })
        ));
    }
}
