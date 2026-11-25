//! Vanilla `BiomeAmbientSoundsHandler` (and its `LoopSoundInstance`): the
//! `audio/ambient_sounds` attribute at the player's position, played every tick.
//!
//! - **Loop**: when the attribute's loop changes, every loop this handler keeps fades out
//!   and the new one (already kept, or started now) fades in, cross-fading over 40 ticks;
//!   a loop that has faded out stops and is dropped.
//! - **Additions**: each entry plays with its `tick_chance` per tick, relative to the
//!   listener (`SimpleSoundInstance.forAmbientAddition`).
//! - **Mood**: a block within `block_search_extent` of the eye is sampled; sky light there
//!   lowers the moodiness a little, darkness raises it by `1 / tick_delay` per tick (block
//!   light lowers it), and at 1 the mood sound plays `offset` blocks beyond the sampled
//!   block, seen from the eye (`SimpleSoundInstance.forAmbientMood`).

use glam::DVec3;
use haste_audio::engine::SoundSource;
use haste_audio::output::InstancePlay;
use haste_audio::random::LegacyRandom;
use haste_core::BlockPos;

use super::ambient_sound_handlers::{AmbientLevel, AmbientPlayer, LightLayer};
use super::sound_manager::SoundManager;
use super::ticking_sounds::{InstanceId, TickableInstance, TickingSounds};
use crate::app::biomes::ambient_sounds::AmbientMoodSettings;

/// Vanilla `BiomeAmbientSoundsHandler.LOOP_SOUND_CROSS_FADE_TIME`, in ticks.
pub const LOOP_SOUND_CROSS_FADE_TIME: i32 = 40;

/// Vanilla `BiomeAmbientSoundsHandler.SKY_MOOD_RECOVERY_RATE`: how much full sky light
/// lowers the moodiness per tick.
pub const SKY_MOOD_RECOVERY_RATE: f32 = 0.001;

/// Vanilla `BiomeAmbientSoundsHandler.LoopSoundInstance`: one biome loop, looping, volume
/// from its fade, relative to the listener.
#[derive(Clone, Debug, PartialEq)]
pub struct LoopSoundInstance {
    event: String,
    fade_direction: i32,
    fade: i32,
    volume: f32,
    stopped: bool,
}

impl LoopSoundInstance {
    /// A new loop of `event`: no fade yet, volume 1 until its first tick.
    pub fn new(event: &str) -> Self {
        Self {
            event: event.to_owned(),
            fade_direction: 0,
            fade: 0,
            volume: 1.0,
            stopped: false,
        }
    }

    /// The event it plays.
    pub fn event(&self) -> &str {
        &self.event
    }

    /// The instance volume.
    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// `AbstractTickableSoundInstance.isStopped`.
    pub fn is_stopped(&self) -> bool {
        self.stopped
    }

    /// `LoopSoundInstance.tick`: a fade below 0 stops it; the fade moves one step in its
    /// direction and the volume follows it over the cross-fade time.
    pub fn tick(&mut self) {
        if self.fade < 0 {
            self.stopped = true;
        }
        self.fade += self.fade_direction;
        self.volume = (self.fade as f32 / LOOP_SOUND_CROSS_FADE_TIME as f32).clamp(0.0, 1.0);
    }

    /// `LoopSoundInstance.fadeOut`: from at most full, downwards.
    pub fn fade_out(&mut self) {
        self.fade = self.fade.min(LOOP_SOUND_CROSS_FADE_TIME);
        self.fade_direction = -1;
    }

    /// `LoopSoundInstance.fadeIn`: from at least silent, upwards.
    pub fn fade_in(&mut self) {
        self.fade = self.fade.max(0);
        self.fade_direction = 1;
    }
}

/// The loop instance behind a kept id, when it is one.
fn loop_instance(ticking: &mut TickingSounds, id: InstanceId) -> Option<&mut LoopSoundInstance> {
    match ticking.get_mut(id) {
        Some(TickableInstance::BiomeLoop(instance)) => Some(instance),
        _ => None,
    }
}

/// Vanilla `BiomeAmbientSoundsHandler`.
#[derive(Debug, Default)]
pub struct BiomeAmbientSoundsHandler {
    /// `loopSounds`: the loops this handler keeps, by event.
    loop_sounds: Vec<(String, InstanceId)>,
    moodiness: f32,
    /// `previousLoopSound`: the loop the attribute named last tick.
    previous_loop_sound: Option<String>,
}

impl BiomeAmbientSoundsHandler {
    /// `getMoodiness` (the debug overlay's mood percentage, `LocalPlayer.getCurrentMood`).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn moodiness(&self) -> f32 {
        self.moodiness
    }

    /// Let go of every loop this handler keeps (the handler is being replaced): a loop
    /// still playing plays on as it was, ticked by the sound tick alone.
    pub fn release_loops(&mut self, ticking: &mut TickingSounds) {
        for (_, id) in self.loop_sounds.drain(..) {
            ticking.release(id);
        }
    }

    /// `BiomeAmbientSoundsHandler.tick`: drop the loops that stopped, follow a change of
    /// the attribute's loop, roll the additions, then the mood step. `random` is the
    /// level's random (`player.level().getRandom()`).
    pub fn tick(
        &mut self,
        player: &AmbientPlayer,
        level: &AmbientLevel<'_>,
        random: &mut LegacyRandom,
        ticking: &mut TickingSounds,
        out: &mut dyn SoundManager,
    ) {
        self.loop_sounds.retain(|(_, id)| {
            let stopped = ticking.get(*id).is_none_or(TickableInstance::is_stopped);
            if stopped {
                ticking.release(*id);
            }
            !stopped
        });
        let sounds = level.sounds;
        let current = sounds.loop_sound.as_deref();
        if current != self.previous_loop_sound.as_deref() {
            self.previous_loop_sound = current.map(str::to_owned);
            for (_, id) in &self.loop_sounds {
                if let Some(instance) = loop_instance(ticking, *id) {
                    instance.fade_out();
                }
            }
            if let Some(event) = current {
                let id = match self.loop_sounds.iter().find(|(e, _)| e == event) {
                    Some((_, id)) => *id,
                    None => {
                        let instance = TickableInstance::BiomeLoop(LoopSoundInstance::new(event));
                        let id = ticking.play(instance, true, out);
                        self.loop_sounds.push((event.to_owned(), id));
                        id
                    }
                };
                if let Some(instance) = loop_instance(ticking, id) {
                    instance.fade_in();
                }
            }
        }
        for additions in &sounds.additions {
            if random.next_double() < additions.tick_chance {
                // `SimpleSoundInstance.forAmbientAddition`: volume and pitch 1, relative,
                // no attenuation, its own unseeded random.
                let seed = ticking.unseeded_seed();
                out.play(&InstancePlay {
                    event: &additions.sound,
                    source: SoundSource::Ambient,
                    pos: None,
                    volume: 1.0,
                    pitch: 1.0,
                    seed,
                    looping: false,
                });
            }
        }
        if let Some(mood) = &sounds.mood {
            self.tick_mood(mood, player, level, random, out);
        }
    }

    /// The mood step of `tick`: sample a block around the eye, move the moodiness by its
    /// light, and at 1 play the mood sound past that block and start over.
    fn tick_mood(
        &mut self,
        mood: &AmbientMoodSettings,
        player: &AmbientPlayer,
        level: &AmbientLevel<'_>,
        random: &mut LegacyRandom,
        out: &mut dyn SoundManager,
    ) {
        let extent = mood.block_search_extent;
        let span = extent * 2 + 1;
        // `BlockPos.containing`: each coordinate floored, the draws in x, y, z order.
        let x = player.pos.x + f64::from(random.next_int(span)) - f64::from(extent);
        let y = player.eye_y + f64::from(random.next_int(span)) - f64::from(extent);
        let z = player.pos.z + f64::from(random.next_int(span)) - f64::from(extent);
        let sample = BlockPos::new(x.floor() as i32, y.floor() as i32, z.floor() as i32);
        self.moodiness = mood_step(
            self.moodiness,
            level.light.brightness(LightLayer::Sky, sample),
            || level.light.brightness(LightLayer::Block, sample),
            mood.tick_delay,
        );
        if self.moodiness >= 1.0 {
            let pos = mood_sound_position(player, sample, mood.offset);
            out.play(&InstancePlay {
                event: &mood.sound,
                source: SoundSource::Ambient,
                pos: Some(pos),
                volume: 1.0,
                pitch: 1.0,
                seed: random.next_long(),
                looping: false,
            });
            self.moodiness = 0.0;
        } else {
            self.moodiness = self.moodiness.max(0.0);
        }
    }
}

/// One tick's moodiness change: sky light above 0 lowers it by `sky / 15 *
/// SKY_MOOD_RECOVERY_RATE`; under no sky, the block light minus one over the tick delay
/// is taken off (darkness, block light 0, adds `1 / tick_delay`). The block light is only
/// read when the sky light is 0, as in vanilla.
pub fn mood_step(moodiness: f32, sky: u8, block: impl FnOnce() -> u8, tick_delay: i32) -> f32 {
    if sky > 0 {
        moodiness - f32::from(sky) / 15.0 * SKY_MOOD_RECOVERY_RATE
    } else {
        moodiness - (i32::from(block()) - 1) as f32 / tick_delay as f32
    }
}

/// Where the mood sound plays: from the eye through the sampled block's centre, `offset`
/// blocks farther than the block. A sample centred exactly on the eye (no direction)
/// plays at the block's centre instead of at an undefined point.
pub fn mood_sound_position(player: &AmbientPlayer, sample: BlockPos, offset: f64) -> DVec3 {
    let eye = DVec3::new(player.pos.x, player.eye_y, player.pos.z);
    let centre = DVec3::new(
        f64::from(sample.x) + 0.5,
        f64::from(sample.y) + 0.5,
        f64::from(sample.z) + 0.5,
    );
    let direction = centre - eye;
    let distance = direction.length();
    if distance == 0.0 {
        return centre;
    }
    eye + direction / distance * (distance + offset)
}

#[cfg(test)]
mod tests;
