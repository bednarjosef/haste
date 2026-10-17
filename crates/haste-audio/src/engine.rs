//! The sound engine core: which sounds are playing and how loud they are, with no audio
//! output at all.
//!
//! This is the fully testable tracking half of the engine; the rodio output stage is built
//! on top of it later and only consumes `gains()`.
use std::sync::Arc;

use glam::DVec3;

use crate::sounds::SoundVariant;
use crate::spatial::{attenuation, clamp_pitch, pan, stereo_gains};

/// The vanilla sound categories a sound can belong to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SoundSource {
    Master,
    Music,
    Record,
    Weather,
    Block,
    Hostile,
    Neutral,
    Player,
    Ambient,
    Voice,
    Ui,
}

/// All sound-source volumes; the effective volume of a source is `master * source`.
#[derive(Clone, Debug)]
pub struct SourceVolumes {
    master: f32,
    sources: [f32; 10],
}

impl Default for SourceVolumes {
    fn default() -> Self {
        Self {
            master: 1.0,
            sources: [1.0; 10],
        }
    }
}

impl SourceVolumes {
    fn index(source: SoundSource) -> Option<usize> {
        match source {
            SoundSource::Master => None,
            SoundSource::Music => Some(0),
            SoundSource::Record => Some(1),
            SoundSource::Weather => Some(2),
            SoundSource::Block => Some(3),
            SoundSource::Hostile => Some(4),
            SoundSource::Neutral => Some(5),
            SoundSource::Player => Some(6),
            SoundSource::Ambient => Some(7),
            SoundSource::Voice => Some(8),
            SoundSource::Ui => Some(9),
        }
    }

    /// The volume of one source (`Master` gives the master volume as-is).
    pub fn get(&self, source: SoundSource) -> f32 {
        match Self::index(source) {
            Some(i) => self.sources[i],
            None => self.master,
        }
    }

    /// Set the volume of one source; `Master` scales every other source with it.
    pub fn set(&mut self, source: SoundSource, volume: f32) {
        match Self::index(source) {
            Some(i) => self.sources[i] = volume,
            None => self.master = volume,
        }
    }

    /// Effective gain multiplier of a source: `master * source`.
    pub fn effective(&self, source: SoundSource) -> f32 {
        self.master * self.get(source)
    }
}

/// The (namespace, path) of an event name, with the `minecraft` namespace defaulted the
/// way `sounds.json` keys and packet identifiers are namespaced.
fn namespace_and_path(event: &str) -> (&str, &str) {
    match event.split_once(':') {
        Some((namespace, path)) => (namespace, path),
        None => ("minecraft", event),
    }
}

/// Whether two event names name the same event: exact match, or the same namespace and
/// path once the `minecraft` namespace is defaulted (`music.game` and
/// `minecraft:music.game` are one event, as they are for vanilla's `Identifier`).
pub(crate) fn event_eq(stored: &str, queried: &str) -> bool {
    let (stored_ns, stored_path) = namespace_and_path(stored);
    let (queried_ns, queried_path) = namespace_and_path(queried);
    stored_ns == queried_ns && stored_path == queried_path
}

/// One sound currently tracked by the engine.
#[derive(Clone, Debug)]
struct PlayingSound {
    /// The sound event this was started with (namespaced, e.g.
    /// `minecraft:block.stone.break`): what `stop`'s event filter and
    /// [`SilentEngine::is_event_playing`] match on. Shared and interned, so tracking it
    /// costs one reference count per played sound.
    event: Arc<str>,
    source: SoundSource,
    pos: Option<DVec3>,
    /// Overall volume of the sound: the play volume times the variant's volume.
    volume: f32,
    /// Final pitch multiplier: the play pitch times the variant's pitch, clamped to
    /// 0.5..=2.0.
    pitch: f32,
    attenuation_distance: u32,
    /// (left, right) gains as of the last `tick()`.
    gains: (f32, f32),
}

/// A sound engine that tracks playing sounds and their stereo gains but produces no
/// audio output whatsoever.
///
/// Gains are recomputed on `tick()` from the current listener state and source volumes,
/// so moving the listener or changing a volume takes effect on the next tick.
#[derive(Debug)]
pub struct SilentEngine {
    sounds: Vec<PlayingSound>,
    volumes: SourceVolumes,
    listener_pos: DVec3,
    listener_yaw: f32,
}

impl Default for SilentEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SilentEngine {
    pub fn new() -> Self {
        Self {
            sounds: Vec::new(),
            volumes: SourceVolumes::default(),
            listener_pos: DVec3::ZERO,
            listener_yaw: 0.0,
        }
    }

    /// Start tracking a sound. The variant's `volume`/`pitch` from sounds.json multiply
    /// into the play `volume`/`pitch`; the pitch is clamped here so every consumer sees
    /// valid input. Sounds with a position are attenuated and panned on `tick()`; sounds
    /// without one play at full gain, centred. `event` is the sound event the sound was
    /// started with, kept for `stop`'s event filter and [`Self::is_event_playing`]; it is
    /// shared, so tracking it costs no allocation.
    pub fn play(
        &mut self,
        variant: &SoundVariant,
        source: SoundSource,
        pos: Option<DVec3>,
        volume: f32,
        pitch: f32,
        event: Arc<str>,
    ) {
        self.sounds.push(PlayingSound {
            event,
            source,
            pos,
            volume: volume * variant.volume,
            pitch: clamp_pitch(pitch * variant.pitch),
            attenuation_distance: variant.attenuation_distance,
            gains: (0.0, 0.0),
        });
    }

    /// Set the listener (camera) position and yaw for attenuation and panning.
    /// Minecraft yaw: 0 = facing +Z (south), increasing clockwise seen from above.
    pub fn set_listener(&mut self, pos: DVec3, yaw_deg: f32) {
        self.listener_pos = pos;
        self.listener_yaw = yaw_deg;
    }

    /// Set the volume of one sound source (or the master volume).
    pub fn set_volume(&mut self, source: SoundSource, volume: f32) {
        self.volumes.set(source, volume);
    }

    /// Stop playing sounds. `None` on an axis matches everything; a sound is stopped
    /// when both the source filter and the event filter match it, as vanilla's
    /// `SoundManager.stop(Identifier, SoundSource)` does: both `None` stops everything,
    /// one `None` stops that whole axis.
    pub fn stop(&mut self, source: Option<SoundSource>, event: Option<&str>) {
        self.sounds.retain(|s| {
            let source_matches = source.is_none_or(|src| s.source == src);
            let event_matches = event.is_none_or(|e| event_eq(&s.event, e));
            !(source_matches && event_matches)
        });
    }

    /// Remove the sound at `index` (play order), like `Vec::remove`: indices after it
    /// shift down. The output stage uses this for sounds whose player finished.
    pub fn remove(&mut self, index: usize) {
        self.sounds.remove(index);
    }

    /// Recompute the (left, right) gains of every playing sound from the current
    /// listener and source volumes.
    pub fn tick(&mut self) {
        for sound in &mut self.sounds {
            let effective = self.volumes.effective(sound.source);
            let (gain, pan) = match sound.pos {
                Some(pos) => (
                    attenuation(
                        (pos - self.listener_pos).length(),
                        sound.volume,
                        sound.attenuation_distance,
                    ) * effective,
                    pan(self.listener_pos, self.listener_yaw, pos),
                ),
                None => (sound.volume.min(1.0) * effective, 0.0),
            };
            sound.gains = stereo_gains(gain, pan);
        }
    }

    /// Number of sounds currently tracked as playing.
    pub fn playing(&self) -> usize {
        self.sounds.len()
    }

    /// Number of sounds currently playing in one source: how the music logic tells the
    /// track apart from block and mob sounds, which all share the `playing` count.
    pub fn playing_in(&self, source: SoundSource) -> usize {
        self.sounds.iter().filter(|s| s.source == source).count()
    }

    /// Whether a sound with this event name is playing (namespace-defaulted match), so
    /// the caller can follow one specific sound rather than a whole source.
    pub fn is_event_playing(&self, event: &str) -> bool {
        self.sounds.iter().any(|s| event_eq(&s.event, event))
    }

    /// The current (left, right) gains of all playing sounds, in play order.
    pub fn gains(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        self.sounds.iter().map(|s| s.gains)
    }

    /// The final (variant-adjusted, clamped) pitch of all playing sounds, in play order.
    pub fn pitches(&self) -> impl Iterator<Item = f32> + '_ {
        self.sounds.iter().map(|s| s.pitch)
    }
}

#[cfg(test)]
mod tests;
