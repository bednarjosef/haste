//! The sound engine core: which sounds are playing and how loud they are, with no audio
//! output at all.
//!
//! This is the fully testable tracking half of the engine; the rodio output stage is built
//! on top of it later and only consumes `gains()`.
use std::sync::Arc;

use glam::DVec3;

use crate::sounds::SoundVariant;
use crate::spatial::{attenuation, clamp_pitch, pan, stereo_gains, ListenerTransform};

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

    /// The source's index in the per-source arrays (`Master` has none: it scales the
    /// others and carries no gain of its own).
    pub(crate) fn index_for(source: SoundSource) -> Option<usize> {
        Self::index(source)
    }

    /// Effective gain multiplier of a source, as vanilla's `SoundEngine.calculateVolume`
    /// takes `Options.getFinalSoundSourceVolume` clamped to 0..=1: `master * source`, and
    /// the master volume alone for the master source (not master squared).
    pub fn effective(&self, source: SoundSource) -> f32 {
        let final_volume = match source {
            SoundSource::Master => self.master,
            _ => self.master * self.get(source),
        };
        final_volume.clamp(0.0, 1.0)
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

/// What playing a sound did, vanilla's `SoundEngine.PlayResult`: not started at all, or
/// started (possibly silently — a volume-0 music track that keeps its channel so a later
/// volume change can fade it in).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayResult {
    Started,
    StartedSilently,
    NotStarted,
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
    /// The variant's own volume from `sounds.json`, so a ticking instance's new volume
    /// ([`SilentEngine::set_sound_volume`]) multiplies with it as the play volume did.
    variant_volume: f32,
    /// Final pitch multiplier: the play pitch times the variant's pitch, clamped to
    /// 0.5..=2.0.
    pitch: f32,
    attenuation_distance: u32,
    /// Whether the sound's file has more than one channel, as the decoder found. OpenAL
    /// plays a multi-channel buffer as-is, unpositioned (no distance, no panning), so a
    /// stereo file (music, records) is global even when the server sent a position; the
    /// output stage reports the channel count once the worker has decoded the file.
    stereo: bool,
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
    /// Extra per-source gains on top of the option volumes, vanilla's `gainBySource` map
    /// (default 1.0 for a missing source): what `SoundEngine.updateCategoryVolume` sets
    /// and what the music fade drives; `stopAll` clears it.
    category_gains: [f32; 10],
    listener: ListenerTransform,
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
            category_gains: [1.0; 10],
            listener: ListenerTransform::from_look(DVec3::ZERO, 0.0, 0.0),
        }
    }

    /// The whole gain multiplier of a sound's volume in `source` (the instance volume
    /// times this is the channel volume): the option volumes times the category gain.
    fn channel_gain(&self, source: SoundSource) -> f32 {
        let gain = match SourceVolumes::index_for(source) {
            Some(i) => self.category_gains[i],
            None => return 1.0,
        };
        self.volumes.effective(source) * gain
    }

    /// Set one source's extra category gain, vanilla's
    /// `SoundEngine.updateCategoryVolume(source, gain)`: the gain is clamped to 0..=1
    /// and multiplies with the option volumes (the music fade drives this; at 1.0, the
    /// default, it changes nothing).
    pub fn update_category_volume(&mut self, source: SoundSource, gain: f32) {
        if let Some(i) = SourceVolumes::index_for(source) {
            self.category_gains[i] = gain.clamp(0.0, 1.0);
        }
    }

    /// Reset every category gain to its default of 1.0, vanilla's `SoundEngine.stopAll`
    /// (which clears `gainBySource`), so a later fade starts from the full gain.
    pub fn reset_category_gains(&mut self) {
        self.category_gains = [1.0; 10];
    }

    /// Start tracking a sound. The variant's `volume`/`pitch` from sounds.json multiply
    /// into the play `volume`/`pitch`; the pitch is clamped here so every consumer sees
    /// valid input. Sounds with a position are attenuated and panned on `tick()`; sounds
    /// without one play at full gain, centred. `event` is the sound event the sound was
    /// started with, kept for `stop`'s event filter and [`Self::is_event_playing`]; it is
    /// shared, so tracking it costs no allocation.
    ///
    /// Vanilla's volume-0 rule (`SoundEngine.play`): a sound whose whole volume works out
    /// to 0 is not started — except music, which starts silently and keeps playing, so a
    /// later volume change (or the music fade) can raise it without restarting the track.
    pub fn play(
        &mut self,
        variant: &SoundVariant,
        source: SoundSource,
        pos: Option<DVec3>,
        volume: f32,
        pitch: f32,
        event: Arc<str>,
    ) -> PlayResult {
        let instance_volume = (volume * variant.volume).clamp(0.0, 1.0);
        let channel_volume = instance_volume * self.channel_gain(source);
        if channel_volume == 0.0 && source != SoundSource::Music {
            return PlayResult::NotStarted;
        }
        let started_silently = channel_volume == 0.0;
        self.sounds.push(PlayingSound {
            event,
            source,
            pos,
            volume: volume * variant.volume,
            variant_volume: variant.volume,
            pitch: clamp_pitch(pitch * variant.pitch),
            attenuation_distance: variant.attenuation_distance,
            stereo: false,
            gains: (0.0, 0.0),
        });
        if started_silently {
            PlayResult::StartedSilently
        } else {
            PlayResult::Started
        }
    }

    /// Drop the most recently started sound, after a channel could not be acquired for it
    /// (vanilla's `SoundEngine.play` gives up on the sound when `createHandle` fails, so
    /// it is never tracked anywhere).
    pub fn remove_last(&mut self) {
        self.sounds.pop();
    }

    /// Set the listener (camera) position and yaw for attenuation and panning.
    /// Minecraft yaw: 0 = facing +Z (south), increasing clockwise seen from above.
    /// Set the listener (the camera) for attenuation and panning: position and look
    /// (yaw/pitch), as vanilla's `SoundEngine.updateSource(Camera)` feeds the camera's
    /// position, forward and up into `Listener.setTransform`.
    pub fn set_listener(&mut self, pos: DVec3, yaw_deg: f32, pitch_deg: f32) {
        self.listener = ListenerTransform::from_look(pos, yaw_deg, pitch_deg);
    }

    /// Set the volume of one sound source (or the master volume).
    pub fn set_volume(&mut self, source: SoundSource, volume: f32) {
        self.volumes.set(source, volume);
    }

    /// Stop playing sounds. `None` on an axis matches everything; a sound is stopped
    /// when both the source filter and the event filter match it, as vanilla's
    /// `SoundManager.stop(Identifier, SoundSource)` does: both `None` stops everything,
    /// one `None` stops that whole axis. Stopping everything also resets the category
    /// gains (vanilla's `SoundEngine.stopAll` clears `gainBySource`, so a faded music
    /// source comes back at full gain for the next track).
    pub fn stop(&mut self, source: Option<SoundSource>, event: Option<&str>) {
        if source.is_none() && event.is_none() {
            self.reset_category_gains();
        }
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

    /// Set the instance volume of the sound at `index` (play order): a ticking sound
    /// instance's new volume, which vanilla's `SoundEngine.tickInGameSound` pushes to the
    /// channel every tick (`calculateVolume(instance)`). It multiplies with the variant's
    /// volume like the play volume; the gains follow on the next `tick()`.
    pub fn set_sound_volume(&mut self, index: usize, volume: f32) {
        if let Some(sound) = self.sounds.get_mut(index) {
            sound.volume = volume * sound.variant_volume;
        }
    }

    /// Mark the sound at `index` (play order) as stereo (its file has more than one
    /// channel). The output stage calls this when the worker has decoded the file and
    /// found several channels; from then on the sound ignores its position, as OpenAL
    /// plays multi-channel buffers unpositioned.
    pub fn set_stereo(&mut self, index: usize) {
        if let Some(sound) = self.sounds.get_mut(index) {
            sound.stereo = true;
        }
    }

    /// Recompute the (left, right) gains of every playing sound from the current
    /// listener and source volumes.
    pub fn tick(&mut self) {
        // Stack copies of the read-only state, so the loop can hold `&mut` sounds.
        let volumes = self.volumes.clone();
        let category_gains = self.category_gains;
        let listener = self.listener;
        for sound in &mut self.sounds {
            let effective = match SourceVolumes::index_for(sound.source) {
                Some(i) => volumes.effective(sound.source) * category_gains[i],
                None => 1.0,
            };
            // A stereo file plays unpositioned, as OpenAL plays multi-channel buffers:
            // full gain, centred, wherever it was asked to play.
            let (gain, pan) = if sound.stereo {
                (sound.volume.min(1.0) * effective, 0.0)
            } else {
                match sound.pos {
                    Some(pos) => (
                        attenuation(
                            (pos - listener.position).length(),
                            sound.volume,
                            sound.attenuation_distance,
                        ) * effective,
                        pan(&listener, pos),
                    ),
                    None => (sound.volume.min(1.0) * effective, 0.0),
                }
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
