//! Tests for `BiomeAmbientSoundsHandler`: the loop's cross-fade, the additions' chance,
//! the mood's light test, delay and offset, against vanilla's numbers.

use std::cell::RefCell;

use super::*;
use crate::app::biomes::ambient_sounds::{AmbientAdditionsSettings, AmbientSounds};
use crate::sound::ambient_sound_handlers::LevelBrightness;
use crate::sound::test_output::RecordingOutput;
use crate::sound::ticking_sounds::TickingPlayer;

/// Fixed light everywhere, recording where it was asked.
struct Light {
    sky: u8,
    block: u8,
    asked: RefCell<Vec<(LightLayer, BlockPos)>>,
}

impl Light {
    fn new(sky: u8, block: u8) -> Self {
        Self {
            sky,
            block,
            asked: RefCell::new(Vec::new()),
        }
    }
}

impl LevelBrightness for Light {
    fn brightness(&self, layer: LightLayer, pos: BlockPos) -> u8 {
        self.asked.borrow_mut().push((layer, pos));
        match layer {
            LightLayer::Sky => self.sky,
            LightLayer::Block => self.block,
        }
    }
}

fn player() -> AmbientPlayer {
    AmbientPlayer {
        pos: DVec3::new(0.5, 64.0, 0.5),
        eye_y: 65.62,
        under_water: false,
        spectator: false,
    }
}

fn looping(event: &str) -> AmbientSounds {
    AmbientSounds {
        loop_sound: Some(event.into()),
        ..AmbientSounds::EMPTY
    }
}

/// One handler tick then one sound tick, as the game tick runs them.
struct Rig {
    handler: BiomeAmbientSoundsHandler,
    random: LegacyRandom,
    ticking: TickingSounds,
    out: RecordingOutput,
}

impl Rig {
    fn new() -> Self {
        Self {
            handler: BiomeAmbientSoundsHandler::default(),
            random: LegacyRandom::new(3),
            ticking: TickingSounds::new(4),
            out: RecordingOutput::default(),
        }
    }

    fn tick(&mut self, sounds: &AmbientSounds, light: &Light) {
        let level = AmbientLevel { sounds, light };
        self.handler.tick(
            &player(),
            &level,
            &mut self.random,
            &mut self.ticking,
            &mut self.out,
        );
        let p = TickingPlayer {
            player: 0,
            under_water: false,
        };
        self.ticking.tick(&p, &mut self.out);
    }
}

#[test]
fn loop_instance_fades_over_the_cross_fade_time() {
    let mut l = LoopSoundInstance::new("minecraft:ambient.crimson_forest.loop");
    assert_eq!(l.volume(), 1.0);
    l.fade_in();
    for t in 1..=40 {
        l.tick();
        assert_eq!(l.volume(), t as f32 / 40.0);
    }
    // Faded in, the fade keeps counting up; fading out starts from the full volume.
    for _ in 0..30 {
        l.tick();
    }
    l.fade_out();
    for t in 1..=40 {
        l.tick();
        assert_eq!(l.volume(), (40 - t) as f32 / 40.0);
    }
    // At 0 it goes once more below 0, and the tick after that stops it.
    l.tick();
    assert!(!l.is_stopped());
    l.tick();
    assert!(l.is_stopped());
    assert_eq!(LOOP_SOUND_CROSS_FADE_TIME, 40);
}

#[test]
fn a_new_loop_cross_fades_with_the_old_one() {
    let mut rig = Rig::new();
    let light = Light::new(15, 0);
    let crimson = looping("minecraft:ambient.crimson_forest.loop");
    let warped = looping("minecraft:ambient.warped_forest.loop");
    rig.tick(&crimson, &light);
    assert_eq!(rig.out.events(), ["minecraft:ambient.crimson_forest.loop"]);
    assert!(rig.out.played[0].looping && rig.out.played[0].pos.is_none());
    let old = rig.out.handle(0);
    assert_eq!(rig.out.volume(old), Some(1.0 / 40.0));
    for _ in 0..59 {
        rig.tick(&crimson, &light);
    }
    assert_eq!(rig.out.volume(old), Some(1.0));
    // Into the warped forest: the old loop fades out from full, the new one fades in.
    rig.tick(&warped, &light);
    let new = rig.out.handle(1);
    assert_eq!(rig.out.volume(old), Some(39.0 / 40.0));
    assert_eq!(rig.out.volume(new), Some(1.0 / 40.0));
    for _ in 0..39 {
        rig.tick(&warped, &light);
    }
    assert_eq!(rig.out.volume(old), Some(0.0));
    assert_eq!(rig.out.volume(new), Some(1.0));
    rig.tick(&warped, &light);
    assert!(rig.out.stopped.is_empty());
    rig.tick(&warped, &light);
    assert_eq!(rig.out.stopped, vec![old]);
    // The stopped loop is dropped on the next handler tick; only the new one is kept.
    rig.tick(&warped, &light);
    assert_eq!(rig.ticking.len(), 1);
    assert_eq!(rig.out.played.len(), 2);
}

#[test]
fn coming_back_in_time_fades_the_same_loop_back_in() {
    let mut rig = Rig::new();
    let light = Light::new(15, 0);
    let crimson = looping("minecraft:ambient.crimson_forest.loop");
    for _ in 0..40 {
        rig.tick(&crimson, &light);
    }
    for _ in 0..10 {
        rig.tick(&AmbientSounds::EMPTY, &light);
    }
    let h = rig.out.handle(0);
    assert_eq!(rig.out.volume(h), Some(30.0 / 40.0));
    rig.tick(&crimson, &light);
    assert_eq!(rig.out.volume(h), Some(31.0 / 40.0));
    // No second play: the kept instance came back.
    assert_eq!(rig.out.played.len(), 1);
}

#[test]
fn additions_play_with_their_chance_per_tick() {
    let mut rig = Rig::new();
    let light = Light::new(15, 0);
    let sounds = AmbientSounds {
        additions: vec![
            AmbientAdditionsSettings {
                sound: "minecraft:ambient.basalt_deltas.additions".into(),
                tick_chance: 0.0111,
            },
            AmbientAdditionsSettings {
                sound: "minecraft:ambient.nether_wastes.additions".into(),
                tick_chance: 0.5,
            },
        ],
        ..AmbientSounds::EMPTY
    };
    let mut mirror = rig.random.clone();
    let mut expected = Vec::new();
    for _ in 0..2_000 {
        rig.tick(&sounds, &light);
        // One `nextDouble` per entry per tick, in the list's order.
        for a in &sounds.additions {
            if mirror.next_double() < a.tick_chance {
                expected.push(a.sound.as_str());
            }
        }
    }
    assert_eq!(rig.out.events(), expected);
    // Relative to the listener, volume 1, not looping (`forAmbientAddition`).
    assert!(rig
        .out
        .played
        .iter()
        .all(|p| p.pos.is_none() && p.volume == 1.0 && !p.looping));
}

#[test]
fn mood_step_follows_the_sky_and_block_light() {
    // Full sky light: down by 1/15 * 15 * 0.001 per tick.
    assert_eq!(mood_step(0.5, 15, || unreachable!(), 6000), 0.5 - 0.001);
    assert_eq!(
        mood_step(0.5, 3, || unreachable!(), 6000),
        0.5 - 3.0 / 15.0 * 0.001
    );
    // No sky: darkness adds 1/tick_delay, block light 1 changes nothing, brighter lowers.
    assert_eq!(mood_step(0.0, 0, || 0, 6000), 1.0 / 6000.0);
    assert_eq!(mood_step(0.25, 0, || 1, 6000), 0.25);
    assert_eq!(mood_step(0.5, 0, || 15, 6000), 0.5 - 14.0 / 6000.0);
}

fn cave(tick_delay: i32) -> AmbientSounds {
    AmbientSounds {
        mood: Some(AmbientMoodSettings {
            sound: "minecraft:ambient.cave".into(),
            tick_delay,
            block_search_extent: 8,
            offset: 2.0,
        }),
        ..AmbientSounds::EMPTY
    }
}

#[test]
fn mood_plays_after_tick_delay_ticks_of_darkness() {
    let mut rig = Rig::new();
    let dark = Light::new(0, 0);
    let sounds = cave(4);
    for t in 1..=3 {
        rig.tick(&sounds, &dark);
        assert_eq!(rig.handler.moodiness(), t as f32 / 4.0);
    }
    assert!(rig.out.played.is_empty());
    rig.tick(&sounds, &dark);
    assert_eq!(rig.out.events(), ["minecraft:ambient.cave"]);
    assert_eq!(rig.handler.moodiness(), 0.0);
    // Under the sky the moodiness never goes below 0.
    let lit = Light::new(15, 0);
    rig.tick(&sounds, &lit);
    assert_eq!(rig.handler.moodiness(), 0.0);
}

#[test]
fn mood_samples_around_the_eye_and_plays_offset_beyond_the_block() {
    let mut rig = Rig::new();
    let dark = Light::new(0, 0);
    let sounds = cave(1);
    let mut mirror = rig.random.clone();
    rig.tick(&sounds, &dark);
    // The sampled block: x, y, z drawn in that order from the eye's column, each
    // `nextInt(17) - 8`; the sky light asked first, then the block light.
    let p = player();
    let x = (p.pos.x + f64::from(mirror.next_int(17)) - 8.0).floor() as i32;
    let y = (p.eye_y + f64::from(mirror.next_int(17)) - 8.0).floor() as i32;
    let z = (p.pos.z + f64::from(mirror.next_int(17)) - 8.0).floor() as i32;
    let sample = BlockPos::new(x, y, z);
    assert_eq!(
        *dark.asked.borrow(),
        vec![(LightLayer::Sky, sample), (LightLayer::Block, sample)]
    );
    // Played 2 blocks past the sampled block's centre, on the line from the eye, seeded
    // from the level's random, positional and not looping.
    let played = &rig.out.played[0];
    assert_eq!(played.seed, mirror.next_long());
    assert!(!played.looping);
    let pos = played.pos.unwrap();
    let eye = DVec3::new(p.pos.x, p.eye_y, p.pos.z);
    let centre = DVec3::new(x as f64 + 0.5, y as f64 + 0.5, z as f64 + 0.5);
    assert!(((pos - eye).length() - ((centre - eye).length() + 2.0)).abs() < 1e-9);
    assert!(((pos - eye).normalize() - (centre - eye).normalize()).length() < 1e-9);
}
