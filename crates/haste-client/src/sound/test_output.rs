//! A recording stand-in for the sound output, for the ambient scheduler's tests: every
//! play is recorded, started sounds get handles and remember their volume, and a test can
//! refuse plays (the volume-0 rule, no channel) or let a sound play through.

use glam::DVec3;
use haste_audio::engine::SoundSource;
use haste_audio::output::{InstancePlay, SoundHandle};

use super::sound_manager::SoundManager;

/// One recorded play.
#[derive(Clone, Debug, PartialEq)]
pub struct Played {
    pub event: String,
    pub source: SoundSource,
    pub pos: Option<DVec3>,
    pub volume: f32,
    pub seed: i64,
    pub looping: bool,
    pub handle: Option<SoundHandle>,
}

/// The recording output.
#[derive(Debug, Default)]
pub struct RecordingOutput {
    pub played: Vec<Played>,
    /// Sounds playing, with their current instance volume.
    pub active: Vec<(SoundHandle, f32)>,
    /// Sounds stopped through [`SoundManager::stop`], in order.
    pub stopped: Vec<SoundHandle>,
    /// Refuse every play (nothing starts).
    pub refuse: bool,
    pub next: u64,
}

impl RecordingOutput {
    /// The events played, in order.
    pub fn events(&self) -> Vec<&str> {
        self.played.iter().map(|p| p.event.as_str()).collect()
    }

    /// The current volume of a playing sound.
    pub fn volume(&self, handle: SoundHandle) -> Option<f32> {
        self.active
            .iter()
            .find(|(h, _)| *h == handle)
            .map(|(_, v)| *v)
    }

    /// The handle of the `index`th play.
    pub fn handle(&self, index: usize) -> SoundHandle {
        self.played[index].handle.expect("the play started")
    }

    /// Let a sound play through (its channel ends without a stop).
    pub fn finish(&mut self, handle: SoundHandle) {
        self.active.retain(|(h, _)| *h != handle);
    }
}

impl SoundManager for RecordingOutput {
    fn play(&mut self, play: &InstancePlay<'_>) -> Option<SoundHandle> {
        let handle = (!self.refuse).then(|| {
            self.next += 1;
            SoundHandle::from_raw(self.next)
        });
        if let Some(h) = handle {
            self.active.push((h, play.volume));
        }
        self.played.push(Played {
            event: play.event.to_owned(),
            source: play.source,
            pos: play.pos,
            volume: play.volume,
            seed: play.seed,
            looping: play.looping,
            handle,
        });
        handle
    }

    fn set_volume(&mut self, handle: SoundHandle, volume: f32) {
        if let Some(entry) = self.active.iter_mut().find(|(h, _)| *h == handle) {
            entry.1 = volume;
        }
    }

    fn stop(&mut self, handle: SoundHandle) {
        self.active.retain(|(h, _)| *h != handle);
        self.stopped.push(handle);
    }

    fn is_active(&self, handle: SoundHandle) -> bool {
        self.active.iter().any(|(h, _)| *h == handle)
    }
}
