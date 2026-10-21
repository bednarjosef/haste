//! The sound event listeners (vanilla `SoundEngine.listeners`, `notifyListeners`): the
//! HUD's `SubtitleOverlay` is real vanilla Java in the JVM, so the sounds the sound system
//! played with a subtitle are handed to it once per tick, as one batch, together with the
//! listener transform it reads for its arrows (`SoundManager.getListenerTransform`).
//!
//! The overlay registers itself with the `SoundManager` proxy while "Show Subtitles" is on
//! (`haste_jvm::sound_manager::sound_listening`); only then does the sound system record
//! anything.

use haste_audio::engine::SoundSource;
use haste_audio::sound_event_listener::PlayedSound;
use haste_audio::spatial::ListenerTransform;
use haste_jvm::sound_manager as java;

use super::{Sounds, SOURCES};

impl Sounds {
    /// The end of the sound tick: the sounds played since the last call and the listener
    /// (set by this tick's `SoundEngine.updateSource`) go to the Java listeners, then
    /// whether they still listen decides if the next tick's plays are recorded.
    pub fn publish_played_sounds(&mut self) {
        let listener = java_listener(&self.system.listener());
        java::publish_sounds(listener, self.system.drain_played().map(java_sound));
        self.system.set_listening(java::sound_listening());
    }
}

/// The listener transform as the Java `ListenerTransform` record holds it.
pub(super) fn java_listener(l: &ListenerTransform) -> java::ListenerTransform {
    java::ListenerTransform {
        position: l.position.to_array(),
        forward: l.forward.to_array(),
        up: l.up.to_array(),
    }
}

/// One played sound as the Java side builds its `SoundInstance` from it: the source as
/// vanilla's `SoundSource` ordinal.
pub(super) fn java_sound(p: PlayedSound) -> java::PlayedSound {
    java::PlayedSound {
        source: source_ordinal(p.source),
        pos: p.pos.to_array(),
        range: p.range,
        volume: p.volume,
        pitch: p.pitch,
        looping: p.looping,
        relative: p.relative,
        event: p.event,
        subtitle: p.subtitle,
    }
}

/// Vanilla's `SoundSource.ordinal()` (the order of [`SOURCES`]).
fn source_ordinal(source: SoundSource) -> u8 {
    SOURCES.iter().position(|s| *s == source).unwrap_or(0) as u8
}

#[cfg(test)]
mod tests;
