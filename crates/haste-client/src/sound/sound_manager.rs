//! What the ambient sound handlers and the ticking sound instances play through: the
//! slice of vanilla's `SoundManager` they use (`play`, `stop(SoundInstance)`,
//! `isActive`, and the per-tick volume `SoundEngine.tickInGameSound` pushes), as a trait
//! so the scheduler runs against the real output, a recording fake in the tests, or
//! nothing at all (headless, or no audio set up).

use haste_audio::output::{InstancePlay, SoundHandle};

use super::Sounds;

/// The sound output a sound instance is played, updated and stopped through.
pub trait SoundManager {
    /// Play `play` (vanilla `SoundManager.play`); the handle when it started, `None` when
    /// it did not (unknown event, the volume-0 rule, no free channel, no output).
    fn play(&mut self, play: &InstancePlay<'_>) -> Option<SoundHandle>;
    /// A ticking instance's volume for this tick.
    fn set_volume(&mut self, handle: SoundHandle, volume: f32);
    /// Stop one instance (vanilla `SoundManager.stop(SoundInstance)`).
    fn stop(&mut self, handle: SoundHandle);
    /// Whether the instance's channel still plays (vanilla `SoundManager.isActive`).
    fn is_active(&self, handle: SoundHandle) -> bool;
}

/// No output: nothing starts, so no instance ever ticks (headless, no audio device set
/// up). Vanilla never ticks an instance whose play did not start either.
pub struct NoSoundManager;

impl SoundManager for NoSoundManager {
    fn play(&mut self, _play: &InstancePlay<'_>) -> Option<SoundHandle> {
        None
    }
    fn set_volume(&mut self, _handle: SoundHandle, _volume: f32) {}
    fn stop(&mut self, _handle: SoundHandle) {}
    fn is_active(&self, _handle: SoundHandle) -> bool {
        false
    }
}

impl SoundManager for Sounds {
    /// An event `sounds.json` does not know (a mod's) plays nothing and logs once.
    fn play(&mut self, play: &InstancePlay<'_>) -> Option<SoundHandle> {
        if self.events.get(play.event).is_none() {
            self.log_unknown(play.event);
            return None;
        }
        self.system.play_instance(play)
    }

    fn set_volume(&mut self, handle: SoundHandle, volume: f32) {
        self.system.set_instance_volume(handle, volume);
    }

    fn stop(&mut self, handle: SoundHandle) {
        self.system.stop_instance(handle);
    }

    fn is_active(&self, handle: SoundHandle) -> bool {
        self.system.is_instance_active(handle)
    }
}
