//! The sound side of vanilla's `SoundEventListener`: what `SoundEngine.play` tells its
//! listeners about every sound it plays (`SoundEngine.notifyListeners`), recorded for
//! the one listener vanilla has, the HUD's `SubtitleOverlay` (the "Show Subtitles"
//! option).
//!
//! The overlay is real vanilla Java in the embedded JVM, so it cannot be called from
//! here per sound: the [`SoundSystem`](crate::output::SoundSystem) records each played
//! sound that has a subtitle into [`SoundEventListeners`] while someone listens, and the
//! client drains the record once per tick and hands it to the JVM bridge in one batch.
//! Nothing is recorded while nobody listens (vanilla skips `notifyListeners` when its
//! listener list is empty), so with subtitles off a play costs one flag check.

use std::sync::Arc;

use glam::DVec3;

use crate::engine::SoundSource;
use crate::sounds::{SoundEvents, SoundVariant};

/// One played sound as a `SoundEventListener.onPlaySound` call sees it: the instance's
/// event, source, position, volume, pitch and loop flag, the event's subtitle, and the
/// range `SoundEngine.notifyListeners` passes.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayedSound {
    /// The sound event the instance plays (`minecraft:entity.chicken.ambient`), vanilla's
    /// `SoundInstance.getIdentifier`.
    pub event: Arc<str>,
    /// The event's subtitle translation key (`WeighedSoundEvents.getSubtitle`).
    pub subtitle: Arc<str>,
    pub source: SoundSource,
    /// Where the instance plays (`getX/getY/getZ`): the origin for a sound relative to
    /// the listener, as vanilla's relative instances sit at 0,0,0.
    pub pos: DVec3,
    /// How far away it can be heard: `SoundEngine.getRange`, infinite for a relative sound.
    pub range: f32,
    /// The instance's own volume and pitch (`AbstractSoundInstance.volume`/`pitch`, the
    /// play's; `getVolume`/`getPitch` multiply the picked sound's into them).
    pub volume: f32,
    pub pitch: f32,
    pub looping: bool,
    /// Whether the sound plays relative to the listener (`SoundInstance.isRelative`).
    pub relative: bool,
}

/// Vanilla's `Sound.getAttenuationDistance(volume)`: a sound louder than 1 carries
/// further, by its volume times its `attenuation_distance`.
pub fn attenuation_distance(volume: f32, attenuation_distance: u32) -> f32 {
    volume.max(1.0) * attenuation_distance as f32
}

/// Vanilla's `SoundEngine.getRange`: a relative (or unattenuated) sound is heard
/// everywhere, any other within its attenuation distance.
pub fn range(relative: bool, attenuation_distance: f32) -> f32 {
    if relative {
        f32::INFINITY
    } else {
        attenuation_distance
    }
}

/// The listeners' side of the engine (vanilla `SoundEngine.listeners`): whether any
/// listener is registered, and the sounds played for them since the last drain.
#[derive(Debug, Default)]
pub struct SoundEventListeners {
    listening: bool,
    played: Vec<PlayedSound>,
}

impl SoundEventListeners {
    /// Whether a listener is registered (vanilla `SoundManager.addListener` /
    /// `removeListener`; the overlay registers itself while subtitles are on). Turning
    /// it off forgets what was not drained yet.
    pub fn set_listening(&mut self, listening: bool) {
        self.listening = listening;
        if !listening {
            self.played.clear();
        }
    }

    pub fn is_listening(&self) -> bool {
        self.listening
    }

    /// `SoundEngine.notifyListeners` for one play: called by `SoundEngine.play` once the
    /// event resolved to a sound, before the volume-0 rule and the channel (so a sound
    /// that is then not started is still captioned, as in vanilla). A sound whose event
    /// has no subtitle is recorded too in vanilla but ignored by the overlay; it is not
    /// recorded here. `pos` is `None` for a sound relative to the listener; `volume` is
    /// the play volume (the variant's own volume multiplies into it).
    #[allow(clippy::too_many_arguments)]
    pub fn notify(
        &mut self,
        events: &SoundEvents,
        event: &Arc<str>,
        variant: &SoundVariant,
        source: SoundSource,
        pos: Option<DVec3>,
        volume: f32,
        pitch: f32,
        looping: bool,
    ) {
        if !self.listening {
            return;
        }
        let Some(subtitle) = events.get(event).and_then(|e| e.subtitle.clone()) else {
            return;
        };
        // `SoundInstance.getVolume`: the play volume times the picked sound's own.
        let instance_volume = volume * variant.volume;
        let relative = pos.is_none();
        self.played.push(PlayedSound {
            event: Arc::clone(event),
            subtitle,
            source,
            pos: pos.unwrap_or(DVec3::ZERO),
            range: range(
                relative,
                attenuation_distance(instance_volume, variant.attenuation_distance),
            ),
            volume,
            pitch,
            looping,
            relative,
        });
    }

    /// The sounds played since the last drain, oldest first.
    pub fn drain(&mut self) -> std::vec::Drain<'_, PlayedSound> {
        self.played.drain(..)
    }
}

#[cfg(test)]
mod tests;
