//! Sound instances the caller keeps controlling after the play: vanilla's ticking sound
//! instances (`TickableSoundInstance`), whose volume `SoundEngine.tickInGameSound` pushes
//! to the channel every tick and which `SoundEngine.stop(SoundInstance)` stops one by one.
//!
//! A play through [`SoundSystem::play_instance`] hands back a [`SoundHandle`] (vanilla's
//! `instanceToChannel` entry); the handle sets the instance's volume, stops it, and tells
//! whether its channel still plays (`SoundEngine.isActive`). A looping instance plays on
//! from the start at its end until it is stopped. Every call is a short scan over the
//! playing sounds and allocates nothing.

use glam::DVec3;

use super::{SoundId, SoundSystem};
use crate::engine::SoundSource;

/// One playing sound instance, as [`SoundSystem::play_instance`] started it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SoundHandle(SoundId);

impl SoundHandle {
    /// A handle from its raw id, for stand-in outputs (a caller's tests) that hand out
    /// their own handles; a real system only answers to the handles it gave out.
    pub fn from_raw(id: u64) -> SoundHandle {
        SoundHandle(id)
    }

    /// The raw id (logs, stand-in outputs).
    pub fn raw(self) -> u64 {
        self.0
    }
}

/// What to play as an instance: the event and its category, where (`None` for a sound
/// relative to the listener, as vanilla's `relative` instances at 0,0,0 play), the
/// instance volume and pitch, the seed its variant is picked with, and whether it loops.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InstancePlay<'a> {
    pub event: &'a str,
    pub source: SoundSource,
    pub pos: Option<DVec3>,
    pub volume: f32,
    pub pitch: f32,
    pub seed: i64,
    pub looping: bool,
}

impl SoundSystem {
    /// Play `play` as an instance the caller keeps controlling (vanilla's
    /// `SoundEngine.play` for a ticking instance). `None` when it was not started: an
    /// unknown event, the volume-0 rule, or no free channel — as vanilla, such an
    /// instance is never ticked.
    pub fn play_instance(&mut self, play: &InstancePlay<'_>) -> Option<SoundHandle> {
        let variant = self.events().pick(play.event, play.seed)?;
        self.start(
            &variant,
            play.source,
            play.pos,
            play.volume,
            play.pitch,
            play.event,
            play.looping,
        )
        .map(SoundHandle)
    }

    /// The handle's place in the play order, while its sound is tracked.
    fn index_of(&self, handle: SoundHandle) -> Option<usize> {
        self.sounded.iter().position(|s| s.id == handle.0)
    }

    /// Set a playing instance's volume (vanilla's `tickInGameSound` pushing
    /// `calculateVolume(instance)`); the gains follow on the next `tick`. A handle whose
    /// sound is gone changes nothing.
    pub fn set_instance_volume(&mut self, handle: SoundHandle, volume: f32) {
        if let Some(index) = self.index_of(handle) {
            self.engine_mut().set_sound_volume(index, volume);
        }
    }

    /// Stop one instance (vanilla's `SoundEngine.stop(SoundInstance)`): its player stops,
    /// its channel goes back to the pool and it is no longer tracked. A handle whose sound
    /// is gone stops nothing.
    pub fn stop_instance(&mut self, handle: SoundHandle) {
        let Some(index) = self.index_of(handle) else {
            return;
        };
        let sounded = self.sounded.remove(index);
        self.output.send_stop(sounded.id);
        self.channels.release(sounded.pool);
        self.engine_mut().remove(index);
    }

    /// Whether the instance's sound still plays (vanilla's `SoundEngine.isActive`): false
    /// once it played through, was stopped, or was never tracked.
    pub fn is_instance_active(&self, handle: SoundHandle) -> bool {
        self.index_of(handle).is_some()
    }
}
