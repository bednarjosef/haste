//! The sound side of the Java `SoundManager` proxy (`haste.gui.SoundImpl`): the sound event
//! listeners vanilla code registers (the HUD's `SubtitleOverlay` while "Show Subtitles" is
//! on), the listener transform they read (`SoundManager.getListenerTransform`) and the
//! sounds played for them (`SoundEventListener.onPlaySound`).
//!
//! Sound is Rust's (haste-audio): the client publishes the listener and the subtitled
//! sounds its sound system played here once per tick ([`publish_sounds`]), and the Java
//! side drains them once per frame with one native (`takePlayedSounds`), before the HUD
//! draws, and calls the real listeners with them. Java tells Rust whether any listener is
//! registered (`soundListening`), so with subtitles off nothing is queued at all
//! ([`sound_listening`]).

use std::collections::VecDeque;
use std::sync::Arc;

use parking_lot::Mutex;

/// One played sound for the listeners (vanilla `SoundEngine.notifyListeners`): what the
/// Java side builds the `SoundInstance` and the `WeighedSoundEvents` it passes from.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayedSound {
    /// The sound event (`minecraft:entity.chicken.ambient`): the instance's identifier.
    pub event: Arc<str>,
    /// The event's subtitle translation key (`WeighedSoundEvents.getSubtitle`).
    pub subtitle: Arc<str>,
    /// Vanilla's `SoundSource` ordinal (`master` 0 .. `ui` 10).
    pub source: u8,
    /// The instance's `getX/getY/getZ` (the origin for a relative sound).
    pub pos: [f64; 3],
    /// The range `notifyListeners` passes (`SoundEngine.getRange`; infinite when relative).
    pub range: f32,
    pub volume: f32,
    pub pitch: f32,
    pub looping: bool,
    pub relative: bool,
}

/// The listener's position and orientation (vanilla `ListenerTransform`): where the sound
/// engine hears from, set from the camera each tick (`SoundEngine.updateSource`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListenerTransform {
    pub position: [f64; 3],
    pub forward: [f64; 3],
    pub up: [f64; 3],
}

impl ListenerTransform {
    /// Vanilla's `ListenerTransform.INITIAL`: at the origin, looking north, up is up.
    pub const INITIAL: ListenerTransform = ListenerTransform {
        position: [0.0; 3],
        forward: [0.0, 0.0, -1.0],
        up: [0.0, 1.0, 0.0],
    };

    /// The layout `takePlayedSounds` fills its `double[LISTENER_LEN]` with: position,
    /// forward, up.
    pub fn to_array(&self) -> [f64; LISTENER_LEN] {
        let [px, py, pz] = self.position;
        let [fx, fy, fz] = self.forward;
        let [ux, uy, uz] = self.up;
        [px, py, pz, fx, fy, fz, ux, uy, uz]
    }
}

/// Length of the listener array (`haste.gui.SoundImpl.LISTENER`).
pub const LISTENER_LEN: usize = 9;
/// Doubles per played sound in `takePlayedSounds`' value array (`haste.gui.SoundImpl`):
/// x, y, z, range, volume, pitch, source ordinal, looping (0/1), relative (0/1).
pub const VALUES_PER_SOUND: usize = 9;
/// How many played sounds wait for the Java side at most. It drains every frame while it
/// listens; this only bounds a queue whose frames stopped (a minimised window), dropping
/// the oldest sounds, which the overlay would have faded out by the next frame anyway.
pub const MAX_QUEUED: usize = 256;

/// The queue between the client's sound system and the Java listeners.
#[derive(Debug)]
pub(crate) struct SoundEventQueue {
    listening: bool,
    listener: ListenerTransform,
    played: VecDeque<PlayedSound>,
}

impl Default for SoundEventQueue {
    fn default() -> Self {
        Self {
            listening: false,
            listener: ListenerTransform::INITIAL,
            played: VecDeque::new(),
        }
    }
}

impl SoundEventQueue {
    /// Java registered its first listener or removed its last one. Off forgets the queue.
    pub(crate) fn set_listening(&mut self, listening: bool) {
        self.listening = listening;
        if !listening {
            self.played.clear();
        }
    }

    /// One tick's listener and played sounds from the client (ignored while nobody
    /// listens, beyond the listener itself).
    pub(crate) fn publish(
        &mut self,
        listener: ListenerTransform,
        played: impl IntoIterator<Item = PlayedSound>,
    ) {
        self.listener = listener;
        if !self.listening {
            return;
        }
        for sound in played {
            if self.played.len() == MAX_QUEUED {
                self.played.pop_front();
            }
            self.played.push_back(sound);
        }
    }

    /// The frame's drain: the listener as an array, and the queued sounds encoded as the
    /// Java side reads them (two names, event and subtitle, and [`VALUES_PER_SOUND`]
    /// values per sound, oldest first). The names are pushed into `names` and the values
    /// into `values`, both cleared first.
    pub(crate) fn take(
        &mut self,
        names: &mut Vec<Arc<str>>,
        values: &mut Vec<f64>,
    ) -> [f64; LISTENER_LEN] {
        names.clear();
        values.clear();
        for sound in self.played.drain(..) {
            let [x, y, z] = sound.pos;
            values.extend_from_slice(&[
                x,
                y,
                z,
                f64::from(sound.range),
                f64::from(sound.volume),
                f64::from(sound.pitch),
                f64::from(sound.source),
                f64::from(u8::from(sound.looping)),
                f64::from(u8::from(sound.relative)),
            ]);
            names.push(sound.event);
            names.push(sound.subtitle);
        }
        self.listener.to_array()
    }
}

static QUEUE: std::sync::LazyLock<Mutex<SoundEventQueue>> =
    std::sync::LazyLock::new(|| Mutex::new(SoundEventQueue::default()));

/// Whether a Java sound event listener is registered (the subtitle overlay while "Show
/// Subtitles" is on): the client's sound system records played sounds only then.
pub fn sound_listening() -> bool {
    QUEUE.lock().listening
}

/// Publishes one tick's listener transform and the subtitled sounds played since the last
/// call (the client, once per tick after its sound system's tick).
pub fn publish_sounds(listener: ListenerTransform, played: impl IntoIterator<Item = PlayedSound>) {
    QUEUE.lock().publish(listener, played);
}

pub(crate) fn with_queue<T>(f: impl FnOnce(&mut SoundEventQueue) -> T) -> T {
    f(&mut QUEUE.lock())
}

#[cfg(test)]
mod tests;
