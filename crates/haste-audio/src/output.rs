//! Real audio output on top of rodio 0.23: the default output device, an off-thread
//! decoder with a bounded LRU cache of decoded sounds, and the `SoundSystem` facade over
//! `SoundEvents`, the engine and the output.
//!
//! rodio 0.23 plays through [`Player`]s connected to a device sink's
//! [`Mixer`](rodio::mixer::Mixer)
//! (there is no `Sink` type any more). Volume and speed live on the `Player`; the stereo
//! balance is a [`ChannelVolume`](rodio::source::ChannelVolume) around the source whose
//! per-channel gains the main thread updates through a shared cell that the audio thread
//! applies every 50 ms.
//!
//! The caller's thread never decodes: `play` only sends a request over a channel; the
//! worker thread reads the Ogg from the assets, decodes it and starts the player.
//! Non-streamed sounds keep their decoded samples in an LRU cache in the worker (streamed
//! entries, music, are decoded on the fly and never cached). The worker hands each new
//! player's controls to the main thread through a queue the main thread drains on
//! `tick`, so a quiet tick allocates nothing on the game thread.
//!
//! `AudioOutput::open()` never fails: with no output device (headless CI, tests) it logs
//! once and stays silent, and every call is accepted and does nothing.
use std::io::Cursor;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use rodio::source::ChannelVolume;
use rodio::{Decoder, Player, Source};
use tracing::warn;

mod decoded;
mod instances;
mod system;

use decoded::{decode, CachedSamples, DecodedSource, LruCache, StereoSamples};
pub use instances::{InstancePlay, SoundHandle};
pub use system::SoundSystem;

/// How many decoded non-streamed sounds the worker keeps.
const CACHE_CAPACITY: usize = 64;

/// How often the audio thread picks up new per-channel gains for a playing sound.
const GAIN_UPDATE_PERIOD: Duration = Duration::from_millis(50);

/// How often the worker wakes on an idle channel to drop finished players.
const WORKER_PRUNE_INTERVAL: Duration = Duration::from_millis(100);

/// Identifies one playing sound across threads; assigned by `SoundSystem`.
type SoundId = u64;

/// A sound to decode and start playing, sent to the worker thread.
struct PlayRequest {
    id: SoundId,
    /// The engine's sound identity, e.g. `minecraft/sounds/dig/stone1.ogg`.
    path: String,
    /// The worker owns the assets after the first request: `Assets` reads need `&mut` and
    /// a thread cannot borrow another thread's, so the first request moves a reopened
    /// copy over and the worker reads every sound itself.
    assets: Option<haste_assets::Assets>,
    /// `stream: true` (music): decoded on the fly, never cached.
    stream: bool,
    /// Play on from the start at the end, for as long as the sound is not stopped (a
    /// looping sound instance, vanilla's `Channel.setLooping` or the looping stream).
    looping: bool,
    /// Initial (left, right) gains and pitch; kept in sync on `tick`.
    gains: (f32, f32),
    pitch: f32,
}

/// Main-thread handle to a playing sound: the player (volume, speed, end), the shared
/// per-channel gains the audio thread applies, and the sound's id.
#[derive(Clone)]
struct SoundControls {
    id: SoundId,
    player: Weak<Player>,
    gains: Arc<Mutex<(f32, f32)>>,
    /// A multi-channel file playing unpositioned: the tick sets the player's volume from
    /// the gain instead of pushing per-channel gains.
    stereo: bool,
}

impl SoundControls {
    fn new(playing: &Playing) -> Self {
        Self {
            id: playing.id,
            player: Arc::downgrade(&playing.player),
            gains: Arc::clone(&playing.gains),
            stereo: playing.stereo,
        }
    }
}

/// One sound the worker keeps playing: the player (kept alive here) and its gain cell.
struct Playing {
    id: SoundId,
    player: Arc<Player>,
    gains: Arc<Mutex<(f32, f32)>>,
    /// A multi-channel file playing unpositioned; its volume is the player's own instead
    /// of the per-channel gains.
    stereo: bool,
}

/// The decode worker: owns the assets, the LRU cache and the live players.
struct Worker {
    mixer: rodio::mixer::Mixer,
    assets: Option<haste_assets::Assets>,
    cache: LruCache,
    playing: Vec<Playing>,
    /// Live controls, queued one message per new player for the main thread.
    shared: Sender<SoundControls>,
}

impl Worker {
    /// Drop players that played through.
    fn prune_finished(&mut self) {
        self.playing.retain(|p| !p.player.empty());
    }

    /// Stop the player of a sound the main thread stopped. A sound still in the decode
    /// queue is stopped by its own `Play` handling, because the stop order queues behind
    /// its play request in the shared channel.
    fn stop(&mut self, id: SoundId) {
        if let Some(playing) = self.playing.iter().find(|p| p.id == id) {
            playing.player.stop();
        }
    }

    /// Queue one player's controls for the main thread.
    fn publish(&self, index: usize) {
        let _ = self.shared.send(SoundControls::new(&self.playing[index]));
    }

    /// Decode the request's sound and start playing it, then queue its controls.
    fn play(&mut self, mut request: PlayRequest) {
        self.prune_finished();
        if let Some(assets) = request.assets.take() {
            self.assets = Some(assets);
        }
        let source = match self.decode(&request) {
            Some(source) => source,
            None => {
                // Nothing to play. Queue an already-finished player under this id so the
                // engine drops the sound on its next tick instead of tracking it forever.
                self.playing.push(Playing {
                    id: request.id,
                    player: Arc::new(Player::connect_new(&self.mixer)),
                    gains: Arc::new(Mutex::new((0.0, 0.0))),
                    stereo: false,
                });
                self.publish(self.playing.len() - 1);
                return;
            }
        };
        let channels = source.channels();
        let player = Player::connect_new(&self.mixer);
        player.set_speed(request.pitch);
        let gains = Arc::new(Mutex::new(request.gains));
        if channels > 1 {
            // A multi-channel buffer plays as-is, unpositioned (OpenAL ignores the
            // source position for it): no per-channel panning, which would average the
            // file down to mono — the whole gain (equal on both channels) goes to the
            // player's own volume instead.
            player.set_volume(request.gains.0);
            player.append(source);
        } else {
            // Per-channel (left, right) gains, updated by the main thread and applied to
            // the source on the audio thread every GAIN_UPDATE_PERIOD.
            let updater = Arc::clone(&gains);
            let balanced = ChannelVolume::new(source, vec![request.gains.0, request.gains.1])
                .periodic_access(GAIN_UPDATE_PERIOD, move |channel_volume| {
                    let (left, right) = *updater.lock().unwrap();
                    channel_volume.set_volume(0, left);
                    channel_volume.set_volume(1, right);
                });
            player.append(balanced);
        }
        self.playing.push(Playing {
            id: request.id,
            player: Arc::new(player),
            gains,
            stereo: channels > 1,
        });
        self.publish(self.playing.len() - 1);
    }

    /// The decoded source for a request: streamed live, from the cache, or freshly
    /// decoded (and cached). `None` when the sound is missing or undecodable. A cached
    /// file with more than one channel passes through interleaved (stereo), not averaged
    /// down to mono.
    fn decode(&mut self, request: &PlayRequest) -> Option<DecodedSource> {
        if request.stream && request.looping {
            // Vanilla's `SoundBufferLibrary.getStream(path, true)`: a stream that starts
            // over at its end.
            let bytes = self.read(request)?;
            let looped = rodio::decoder::DecoderBuilder::new()
                .with_data(Cursor::new(bytes))
                .with_hint("ogg")
                .build_looped();
            return match looped {
                Ok(decoder) => Some(DecodedSource::StreamedLooping(decoder)),
                Err(e) => {
                    warn!("cannot decode {}: {e}", request.path);
                    None
                }
            };
        }
        if request.stream {
            let bytes = self.read(request)?;
            match Decoder::new_vorbis(Cursor::new(bytes)) {
                Ok(decoder) => return Some(DecodedSource::Streamed(decoder)),
                Err(e) => {
                    warn!("cannot decode {}: {e}", request.path);
                    return None;
                }
            }
        }
        let cached = if let Some(sound) = self.cache.get(&request.path) {
            sound
        } else {
            let sound = self
                .read(request)
                .and_then(|bytes| decode(bytes, &request.path))?;
            self.cache.insert(request.path.clone(), sound.clone());
            sound
        };
        Some(if cached.channels > 1 {
            DecodedSource::Stereo(StereoSamples::new(&cached).looping(request.looping))
        } else {
            DecodedSource::Cached(CachedSamples::new(&cached).looping(request.looping))
        })
    }

    /// The Ogg bytes of the request's sound, read from the assets.
    fn read(&mut self, request: &PlayRequest) -> Option<Vec<u8>> {
        let assets = self.assets.as_mut()?;
        let key = format!("assets/{}", request.path);
        match assets.read_asset(&key) {
            Ok(bytes) => Some(bytes),
            Err(e) => {
                warn!("cannot read sound {}: {e:#}", request.path);
                None
            }
        }
    }
}

/// Real audio output: the channel to the decode worker, or nothing when there is no
/// device.
pub struct AudioOutput {
    /// `None` = silent fallback (no device or no worker; logged once in `open`).
    tx: Option<Sender<PlayRequest>>,
    /// Controls of new players, one message each, drained into `SoundSystem::incoming`.
    controls_rx: Receiver<SoundControls>,
    /// Stop orders for the worker, one message per stopped sound.
    stop_tx: Option<Sender<SoundId>>,
}

impl AudioOutput {
    /// Open the default output device and start the decode worker. Never fails: without a
    /// device (or when the worker thread cannot start) the output stays silent, logged
    /// once, and every method is a no-op.
    pub fn open() -> AudioOutput {
        let sink = match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(mut sink) => {
                sink.log_on_drop(false);
                sink
            }
            Err(e) => {
                warn!("no audio output device ({e}); audio stays silent");
                return AudioOutput::silent();
            }
        };
        let (tx, rx) = channel::<PlayRequest>();
        let (controls_tx, controls_rx) = channel::<SoundControls>();
        let (stop_tx, stop_rx) = channel::<SoundId>();
        let spawn = std::thread::Builder::new()
            .name("audio-decode".to_string())
            .spawn(move || run_worker(rx, stop_rx, sink.mixer().clone(), controls_tx));
        match spawn {
            // Detached: the worker loop ends when the last `AudioOutput` drops the sender,
            // which also drops the sink and stops the device stream.
            Ok(thread) => {
                std::mem::forget(thread);
                AudioOutput {
                    tx: Some(tx),
                    controls_rx,
                    stop_tx: Some(stop_tx),
                }
            }
            Err(e) => {
                warn!("cannot start the audio decode worker ({e}); audio stays silent");
                AudioOutput::silent()
            }
        }
    }

    /// The silent fallback: no device, no worker, every call a no-op. Tests and callers
    /// managing the output themselves build their system over it, so no test ever opens
    /// the audio device.
    pub fn silent() -> AudioOutput {
        let (_controls_tx, controls_rx) = channel::<SoundControls>();
        AudioOutput {
            tx: None,
            controls_rx,
            stop_tx: None,
        }
    }

    /// Whether a real device is attached (diagnostics, tests).
    pub fn is_live(&self) -> bool {
        self.tx.is_some()
    }

    /// Hand a sound to the worker to decode and play. A no-op on the silent path; never
    /// blocks (the request is only moved, all reading and decoding happens on the worker).
    fn send(&self, request: PlayRequest) {
        let Some(tx) = &self.tx else {
            return;
        };
        if tx.send(request).is_err() {
            warn!("audio decode worker is gone; sound not played");
        }
    }

    /// Tell the worker to stop a sound (already dispatched to play). A no-op on the silent
    /// path. The message queues behind any earlier `Play` for the same sound, so the
    /// worker stops the player once it exists.
    fn send_stop(&self, id: SoundId) {
        let Some(tx) = &self.stop_tx else {
            return;
        };
        if tx.send(id).is_err() {
            warn!("audio decode worker is gone; sound not stopped");
        }
    }

    /// Move the controls queued since the last call into `into`, reusing its allocation.
    /// Does nothing on the silent path (nothing is ever queued there).
    fn drain_controls(&self, into: &mut Vec<SoundControls>) {
        while let Ok(controls) = self.controls_rx.try_recv() {
            into.push(controls);
        }
    }
}

/// The worker loop: handle each request or stop order as it arrives; on an idle channel
/// wake periodically to drop finished players.
fn run_worker(
    rx: Receiver<PlayRequest>,
    stop_rx: Receiver<SoundId>,
    mixer: rodio::mixer::Mixer,
    controls_tx: Sender<SoundControls>,
) {
    let mut worker = Worker {
        mixer,
        assets: None,
        cache: LruCache::new(CACHE_CAPACITY),
        playing: Vec::new(),
        shared: controls_tx,
    };
    loop {
        // Process everything already queued before waiting, so orders keep their order.
        while let Ok(request) = rx.try_recv() {
            worker.play(request);
        }
        while let Ok(id) = stop_rx.try_recv() {
            worker.stop(id);
        }
        worker.prune_finished();
        match rx.recv_timeout(WORKER_PRUNE_INTERVAL) {
            Ok(request) => worker.play(request),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
use decoded::DecodedSound;
#[cfg(test)]
use rodio::Sample;
