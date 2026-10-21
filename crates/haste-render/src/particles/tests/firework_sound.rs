use super::*;
use crate::particles::firework::{Kind, Starter};
use crate::particles::{FireworkExplosion, FireworkShape, FireworkSound, FireworkSoundKind};

// --- Vanilla-number tests: the sounds `FireworkParticles.Starter.tick` plays for a burst. ---

fn firework_kind() -> Kind {
    Kind {
        index: 0,
        behaviour: crate::particles::providers::ParticleBehaviour::of(&Identifier::minecraft(
            "firework",
        )),
    }
}

fn explosion(shape: FireworkShape, twinkle: bool) -> FireworkExplosion {
    FireworkExplosion {
        shape,
        colors: vec![0xFF_00_00],
        fade_colors: vec![0x00_00_FF],
        trail: false,
        twinkle,
    }
}

/// Ticks a starter until it is done with the camera at `camera`, returning every sound it
/// queued, in order.
fn sounds_for(explosions: &[FireworkExplosion], camera: DVec3) -> Vec<FireworkSound> {
    let mut rng = Rng(17);
    let mut starter = Starter::new(DVec3::ZERO, DVec3::ZERO, explosions, firework_kind(), None);
    let mut out = Vec::new();
    let mut sounds = Vec::new();
    while starter.tick(&mut out, &mut rng, camera, &mut sounds) {}
    sounds
}

/// A camera right on the burst, so every sound is near.
const NEAR: DVec3 = DVec3::ZERO;

#[test]
fn one_small_explosion_plays_the_blast() {
    let sounds = sounds_for(&[explosion(FireworkShape::SmallBall, false)], NEAR);
    assert_eq!(sounds.len(), 1);
    assert_eq!(sounds[0].kind, FireworkSoundKind::Blast);
}

#[test]
fn two_explosions_still_play_the_blast() {
    let two = [
        explosion(FireworkShape::Star, false),
        explosion(FireworkShape::Creeper, false),
    ];
    let sounds = sounds_for(&two, NEAR);
    assert_eq!(sounds.len(), 1);
    assert_eq!(sounds[0].kind, FireworkSoundKind::Blast);
}

#[test]
fn three_explosions_play_the_large_blast() {
    let three = [
        explosion(FireworkShape::Star, false),
        explosion(FireworkShape::Star, false),
        explosion(FireworkShape::Star, false),
    ];
    let sounds = sounds_for(&three, NEAR);
    assert_eq!(sounds[0].kind, FireworkSoundKind::LargeBlast);
}

#[test]
fn a_large_ball_plays_the_large_blast_even_alone() {
    let sounds = sounds_for(&[explosion(FireworkShape::LargeBall, false)], NEAR);
    assert_eq!(sounds[0].kind, FireworkSoundKind::LargeBlast);
}

#[test]
fn the_blast_is_queued_on_the_first_tick_only() {
    let mut rng = Rng(3);
    let two = [
        explosion(FireworkShape::SmallBall, false),
        explosion(FireworkShape::SmallBall, false),
    ];
    let mut starter = Starter::new(DVec3::ZERO, DVec3::ZERO, &two, firework_kind(), None);
    let mut out = Vec::new();
    let mut sounds = Vec::new();
    starter.tick(&mut out, &mut rng, NEAR, &mut sounds);
    assert_eq!(sounds.len(), 1);
    // The second explosion's tick, and the rest of the run, add no blast.
    for _ in 0..4 {
        starter.tick(&mut out, &mut rng, NEAR, &mut sounds);
    }
    assert_eq!(sounds.len(), 1);
}

#[test]
fn no_twinkle_sound_without_a_twinkling_explosion() {
    let sounds = sounds_for(&[explosion(FireworkShape::Burst, false)], NEAR);
    assert!(sounds.iter().all(|s| s.kind != FireworkSoundKind::Twinkle));
}

#[test]
fn a_twinkling_burst_queues_the_twinkle_after_its_blast() {
    let sounds = sounds_for(&[explosion(FireworkShape::Burst, true)], NEAR);
    let kinds: Vec<_> = sounds.iter().map(|s| s.kind).collect();
    assert_eq!(
        kinds,
        [FireworkSoundKind::Blast, FireworkSoundKind::Twinkle]
    );
}

#[test]
fn the_twinkle_is_queued_only_once_the_starter_ends() {
    let mut rng = Rng(5);
    let one = [explosion(FireworkShape::Burst, true)];
    let mut starter = Starter::new(DVec3::ZERO, DVec3::ZERO, &one, firework_kind(), None);
    let mut out = Vec::new();
    let mut sounds = Vec::new();
    // 1 * 2 - 1 + 15 = 16 life ticks: no twinkle while the burst is still going.
    for _ in 0..16 {
        assert!(starter.tick(&mut out, &mut rng, NEAR, &mut sounds));
        assert!(sounds.iter().all(|s| s.kind != FireworkSoundKind::Twinkle));
    }
    // The tick that finds life past the lifetime queues it, then the starter ends.
    assert!(!starter.tick(&mut out, &mut rng, NEAR, &mut sounds));
    assert_eq!(
        sounds.last().map(|s| s.kind),
        Some(FireworkSoundKind::Twinkle)
    );
}

#[test]
fn sounds_are_far_at_sixteen_blocks_and_near_just_under() {
    let at_sixteen = sounds_for(
        &[explosion(FireworkShape::SmallBall, false)],
        DVec3::new(16.0, 0.0, 0.0),
    );
    assert!(at_sixteen[0].far);
    // 15.9 blocks: 252.81 squared, under 256.
    let just_under = sounds_for(
        &[explosion(FireworkShape::SmallBall, false)],
        DVec3::new(15.9, 0.0, 0.0),
    );
    assert!(!just_under[0].far);
}

#[test]
fn far_sounds_use_the_far_event_ids() {
    let far = sounds_for(
        &[explosion(FireworkShape::Burst, true)],
        DVec3::new(0.0, 0.0, 20.0),
    );
    assert_eq!(far[0].event(), "minecraft:entity.firework_rocket.blast_far");
    assert_eq!(
        far[1].event(),
        "minecraft:entity.firework_rocket.twinkle_far"
    );
    let near = sounds_for(&[explosion(FireworkShape::Burst, true)], NEAR);
    assert_eq!(near[0].event(), "minecraft:entity.firework_rocket.blast");
    assert_eq!(near[1].event(), "minecraft:entity.firework_rocket.twinkle");
}

#[test]
fn large_blast_event_ids() {
    let three = [
        explosion(FireworkShape::Star, false),
        explosion(FireworkShape::Star, false),
        explosion(FireworkShape::Star, false),
    ];
    let near = sounds_for(&three, NEAR);
    assert_eq!(
        near[0].event(),
        "minecraft:entity.firework_rocket.large_blast"
    );
    let far = sounds_for(&three, DVec3::new(0.0, 0.0, 16.0));
    assert_eq!(
        far[0].event(),
        "minecraft:entity.firework_rocket.large_blast_far"
    );
}

#[test]
fn blast_pitch_is_within_point_nine_five_to_one_point_zero_five() {
    for seed in 1..200 {
        let mut rng = Rng(seed);
        let one = [explosion(FireworkShape::SmallBall, false)];
        let mut starter = Starter::new(DVec3::ZERO, DVec3::ZERO, &one, firework_kind(), None);
        let mut out = Vec::new();
        let mut sounds = Vec::new();
        starter.tick(&mut out, &mut rng, NEAR, &mut sounds);
        let pitch = sounds[0].pitch;
        // Vanilla: 0.95 + nextFloat() * 0.1, so [0.95, 1.05).
        assert!((0.95..1.05).contains(&pitch), "blast pitch {pitch}");
    }
}

#[test]
fn twinkle_pitch_is_within_point_nine_to_one_point_zero_five() {
    for seed in 1..200 {
        let sounds = {
            let mut rng = Rng(seed);
            let one = [explosion(FireworkShape::Burst, true)];
            let mut starter = Starter::new(DVec3::ZERO, DVec3::ZERO, &one, firework_kind(), None);
            let mut out = Vec::new();
            let mut sounds = Vec::new();
            while starter.tick(&mut out, &mut rng, NEAR, &mut sounds) {}
            sounds
        };
        let twinkle = sounds
            .iter()
            .find(|s| s.kind == FireworkSoundKind::Twinkle)
            .expect("twinkle sound");
        // Vanilla: 0.9 + nextFloat() * 0.15, so [0.9, 1.05).
        assert!(
            (0.9..1.05).contains(&twinkle.pitch),
            "twinkle pitch {}",
            twinkle.pitch
        );
    }
}

#[test]
fn sounds_play_at_the_burst_position() {
    let mut rng = Rng(2);
    let one = [explosion(FireworkShape::SmallBall, false)];
    let at = DVec3::new(4.0, 70.0, -9.0);
    let mut starter = Starter::new(at, DVec3::ZERO, &one, firework_kind(), None);
    let mut out = Vec::new();
    let mut sounds = Vec::new();
    starter.tick(&mut out, &mut rng, NEAR, &mut sounds);
    assert_eq!(sounds[0].pos, at);
}
