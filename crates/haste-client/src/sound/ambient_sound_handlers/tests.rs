//! Tests for the local player's ambient handler list: `updateIsUnderwater`'s enter/exit
//! sounds and loop, the spectator rule, and what a respawn keeps and drops.

use super::*;
use crate::sound::test_output::RecordingOutput;
use crate::sound::under_liquid_ambient_sound_instance::AMBIENT_UNDERWATER_LOOP;

struct Lit;

impl LevelBrightness for Lit {
    fn brightness(&self, _layer: LightLayer, _pos: BlockPos) -> u8 {
        15
    }
}

fn player(under_water: bool, spectator: bool) -> AmbientPlayer {
    AmbientPlayer {
        pos: DVec3::new(3.5, 40.0, -2.5),
        eye_y: 41.62,
        under_water,
        spectator,
    }
}

struct Rig {
    handlers: AmbientSoundHandlers,
    ticking: TickingSounds,
    out: RecordingOutput,
}

impl Rig {
    fn new() -> Self {
        Self {
            handlers: AmbientSoundHandlers::new(11),
            ticking: TickingSounds::new(12),
            out: RecordingOutput::default(),
        }
    }

    /// The player's tick, then the sound tick.
    fn tick(&mut self, p: &AmbientPlayer, sounds: &AmbientSounds) {
        let level = AmbientLevel {
            sounds,
            light: &Lit,
        };
        self.handlers
            .tick(p, &level, &mut self.ticking, &mut self.out);
        let ticking_player = self.handlers.ticking_player(p.under_water);
        self.ticking.tick(&ticking_player, &mut self.out);
    }
}

#[test]
fn going_under_plays_the_enter_sound_at_the_feet_and_starts_the_loop() {
    let mut rig = Rig::new();
    let mut level_random = LegacyRandom::new(11);
    rig.tick(&player(false, false), &AmbientSounds::EMPTY);
    assert!(rig.out.played.is_empty());
    rig.tick(&player(true, false), &AmbientSounds::EMPTY);
    let enter = &rig.out.played[0];
    assert_eq!(enter.event, AMBIENT_UNDERWATER_ENTER);
    assert_eq!(enter.source, SoundSource::Ambient);
    assert_eq!(enter.pos, Some(player(true, false).pos));
    assert_eq!(enter.volume, 1.0);
    // `playLocalSound` seeds from the level's random, before the handlers roll it.
    assert_eq!(enter.seed, level_random.next_long());
    let looped = &rig.out.played[1];
    assert_eq!(looped.event, AMBIENT_UNDERWATER_LOOP);
    assert!(looped.looping);
    // Coming up: the exit sound; the loop fades out on its own.
    let under_ticks = rig.out.played.len();
    rig.tick(&player(false, false), &AmbientSounds::EMPTY);
    let exit = &rig.out.played[under_ticks];
    assert_eq!(exit.event, AMBIENT_UNDERWATER_EXIT);
    assert_eq!(exit.pos, Some(player(false, false).pos));
}

#[test]
fn a_spectator_changes_state_silently() {
    let mut rig = Rig::new();
    rig.tick(&player(true, true), &AmbientSounds::EMPTY);
    rig.tick(&player(false, true), &AmbientSounds::EMPTY);
    assert!(!rig
        .out
        .events()
        .iter()
        .any(|e| *e == AMBIENT_UNDERWATER_ENTER || *e == AMBIENT_UNDERWATER_EXIT));
    // Under water as a spectator, then back to survival still under water: no change,
    // no enter sound.
    rig.tick(&player(true, true), &AmbientSounds::EMPTY);
    rig.tick(&player(true, false), &AmbientSounds::EMPTY);
    assert!(!rig.out.events().contains(&AMBIENT_UNDERWATER_ENTER));
}

#[test]
fn a_respawn_stops_the_old_underwater_sounds_and_starts_over() {
    let mut rig = Rig::new();
    let nether = AmbientSounds {
        loop_sound: Some("minecraft:ambient.nether_wastes.loop".into()),
        ..AmbientSounds::EMPTY
    };
    rig.tick(&player(true, false), &nether);
    let water_loop = rig
        .out
        .played
        .iter()
        .position(|p| p.event == AMBIENT_UNDERWATER_LOOP)
        .unwrap();
    let biome_loop = rig
        .out
        .played
        .iter()
        .position(|p| p.event == "minecraft:ambient.nether_wastes.loop")
        .unwrap();
    // Same dimension: the old player's water loop stops, its biome loop plays on, and
    // the new player, still under water, enters the water again with a new loop.
    rig.handlers.respawn(&mut rig.ticking, false);
    rig.tick(&player(true, false), &nether);
    assert!(rig.out.stopped.contains(&rig.out.handle(water_loop)));
    assert!(!rig.out.stopped.contains(&rig.out.handle(biome_loop)));
    let enters = rig
        .out
        .events()
        .iter()
        .filter(|e| **e == AMBIENT_UNDERWATER_ENTER)
        .count();
    assert_eq!(enters, 2);
    assert_eq!(rig.handlers.moodiness(), 0.0);
    // A dimension change stops every sound before the new player's first sounds.
    let before = rig.out.played.len();
    rig.handlers.respawn(&mut rig.ticking, true);
    rig.tick(&player(false, false), &AmbientSounds::EMPTY);
    assert!(rig.out.stopped.contains(&rig.out.handle(biome_loop)));
    // Every loop is silenced (the one-shot sounds are the output's own business).
    for p in rig.out.played.iter().filter(|p| p.looping) {
        assert!(
            !rig.out.is_active(p.handle.unwrap()),
            "{} plays on",
            p.event
        );
    }
    assert!(rig.ticking.is_empty());
    assert_eq!(rig.out.played.len(), before);
}

#[test]
fn world_brightness_reads_no_sky_light_without_skylight() {
    struct Bright;
    impl BlockView for Bright {
        fn block_state(&self, _: BlockPos) -> haste_core::BlockStateId {
            haste_core::BlockStateId::AIR
        }
        fn is_chunk_loaded(&self, _: haste_core::ChunkPos) -> bool {
            true
        }
        fn sky_light(&self, _: BlockPos) -> u8 {
            15
        }
        fn block_light(&self, _: BlockPos) -> u8 {
            7
        }
        fn min_y(&self) -> i32 {
            0
        }
        fn height(&self) -> i32 {
            256
        }
    }
    let pos = BlockPos::new(0, 64, 0);
    let nether = WorldBrightness {
        world: &Bright,
        has_skylight: false,
    };
    assert_eq!(nether.brightness(LightLayer::Sky, pos), 0);
    assert_eq!(nether.brightness(LightLayer::Block, pos), 7);
    let overworld = WorldBrightness {
        world: &Bright,
        has_skylight: true,
    };
    assert_eq!(overworld.brightness(LightLayer::Sky, pos), 15);
}
