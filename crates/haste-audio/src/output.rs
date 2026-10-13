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
//! entries, music, are decoded on the fly and never cached).
//!
//! `AudioOutput::open()` never fails: with no output device (headless CI, tests) it logs
//! once and stays silent, and every call is accepted and does nothing.
use std::collections::VecDeque;
use std::io::Cursor;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use rodio::source::ChannelVolume;
use rodio::{Decoder, Player, Sample, Source};
use tracing::{debug, warn};

use crate::engine::{SilentEngine, SoundSource};
use crate::sounds::{SoundEvents, SoundVariant};
use glam::DVec3;

/// How many decoded non-streamed sounds the worker keeps.
const CACHE_CAPACITY: usize = 64;

/// How often the audio thread picks up new per-channel gains for a playing sound.
const GAIN_UPDATE_PERIOD: Duration = Duration::from_millis(50);

/// Identifies one playing sound across threads; assigned by `SoundSystem`.
type SoundId = u64;

/// A sound to decode and start playing, sent to the worker thread.
struct PlayRequest {
    id: SoundId,
    /// The engine's sound identity, e.g. `sounds/dig/stone1.ogg`.
    path: String,
    /// The worker owns the assets after the first request: `Assets` reads need `&mut` and
    /// a thread cannot borrow another thread's, so the first request moves a reopened
    /// copy over and the worker reads every sound itself.
    assets: Option<haste_assets::Assets>,
    /// `stream: true` (music): decoded on the fly, never cached.
    stream: bool,
    /// Initial (left, right) gains and pitch; kept in sync on `tick`.
    gains: (f32, f32),
    pitch: f32,
}

/// Main-thread handle to a playing sound: the player (volume, speed, end) and the shared
/// per-channel gains the audio thread applies.
#[derive(Clone)]
struct SoundControls {
    player: Weak<Player>,
    gains: Arc<Mutex<(f32, f32)>>,
}

/// A fully decoded sound: interleaved samples with channel count and sample rate.
#[derive(Clone)]
struct DecodedSound {
    samples: Arc<Vec<Sample>>,
    channels: u16,
    sample_rate: u32,
}

/// Bounded LRU cache of decoded sounds, by path. The front is the least recently used.
struct LruCache {
    entries: VecDeque<(String, DecodedSound)>,
    capacity: usize,
}

impl LruCache {
    fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            capacity,
        }
    }

    /// The entry for `path`, moved to the back (most recently used).
    fn get(&mut self, path: &str) -> Option<DecodedSound> {
        let index = self.entries.iter().position(|(p, _)| p == path)?;
        let (_, sound) = self.entries.remove(index).unwrap();
        self.entries.push_back((path.to_string(), sound.clone()));
        Some(sound)
    }

    /// Insert an entry; when full, the least recently used entry is evicted. Re-inserting
    /// an existing path refreshes it instead of duplicating it.
    fn insert(&mut self, path: String, sound: DecodedSound) {
        if let Some(index) = self.entries.iter().position(|(p, _)| *p == path) {
            self.entries.remove(index);
        } else if self.entries.len() >= self.capacity {
            if let Some((evicted, _)) = self.entries.pop_front() {
                debug!("evicted {evicted} from the decoded-sound cache");
            }
        }
        self.entries.push_back((path, sound));
    }

    #[cfg(test)]
    fn paths(&self) -> Vec<&str> {
        self.entries.iter().map(|(p, _)| p.as_str()).collect()
    }
}

/// A mono `Source` over an `Arc`ed, already-decoded sample buffer.
///
/// rodio 0.23 has no owned-buffer source that is cheap to reuse per play
/// (`StaticSamplesBuffer` needs a `&'static [Sample]`, `SamplesBuffer` moves a `Vec`), so
/// the cache stores the raw interleaved samples and this feeds them as mono (channels
/// averaged per frame), which is also what `ChannelVolume` wants as its input.
#[derive(Clone)]
struct CachedSamples {
    samples: Arc<Vec<Sample>>,
    frame_len: usize,
    sample_rate: u32,
    /// Interleaved-sample cursor.
    cursor: usize,
}

impl CachedSamples {
    fn new(sound: &DecodedSound) -> Self {
        Self {
            samples: Arc::clone(&sound.samples),
            frame_len: sound.channels as usize,
            sample_rate: sound.sample_rate,
            cursor: 0,
        }
    }
}

impl Iterator for CachedSamples {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        let frame = self
            .samples
            .get(self.cursor..self.cursor + self.frame_len)?;
        self.cursor += self.frame_len;
        Some(frame.iter().sum::<Sample>() / self.frame_len as Sample)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let frames = (self.samples.len() - self.cursor) / self.frame_len;
        (frames, Some(frames))
    }
}

impl Source for CachedSamples {
    fn current_span_len(&self) -> Option<usize> {
        self.size_hint().1
    }

    fn channels(&self) -> rodio::ChannelCount {
        rodio::ChannelCount::new(1).expect("constant 1 is non-zero")
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        rodio::SampleRate::new(self.sample_rate).expect("validated non-zero at decode")
    }

    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f64(
            self.size_hint().0 as f64 / self.sample_rate as f64,
        ))
    }
}

/// What the worker plays: a streamed decode (music) or a cached, already-decoded sound.
enum DecodedSource {
    Streamed(Decoder<Cursor<Vec<u8>>>),
    Cached(CachedSamples),
}

impl Iterator for DecodedSource {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        match self {
            DecodedSource::Streamed(decoder) => decoder.next(),
            DecodedSource::Cached(cached) => cached.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            DecodedSource::Streamed(decoder) => decoder.size_hint(),
            DecodedSource::Cached(cached) => cached.size_hint(),
        }
    }
}

impl Source for DecodedSource {
    fn current_span_len(&self) -> Option<usize> {
        match self {
            DecodedSource::Streamed(decoder) => decoder.current_span_len(),
            DecodedSource::Cached(cached) => cached.current_span_len(),
        }
    }

    fn channels(&self) -> rodio::ChannelCount {
        match self {
            DecodedSource::Streamed(decoder) => decoder.channels(),
            DecodedSource::Cached(cached) => cached.channels(),
        }
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        match self {
            DecodedSource::Streamed(decoder) => decoder.sample_rate(),
            DecodedSource::Cached(cached) => cached.sample_rate(),
        }
    }

    fn total_duration(&self) -> Option<Duration> {
        match self {
            DecodedSource::Streamed(decoder) => decoder.total_duration(),
            DecodedSource::Cached(cached) => cached.total_duration(),
        }
    }
}

/// One sound the worker keeps playing: the player (kept alive here) and its gain cell.
struct Playing {
    id: SoundId,
    player: Arc<Player>,
    gains: Arc<Mutex<(f32, f32)>>,
}

/// The decode worker: owns the assets, the LRU cache and the live players.
struct Worker {
    mixer: rodio::mixer::Mixer,
    assets: Option<haste_assets::Assets>,
    cache: LruCache,
    playing: Vec<Playing>,
    /// Live controls, shared read-only with the main thread.
    shared: Arc<Mutex<Vec<(SoundId, SoundControls)>>>,
}

impl Worker {
    /// Decode the request's sound and start playing it, then publish the live controls.
    fn play(&mut self, mut request: PlayRequest) {
        // Drop finished players first: the main thread has seen them as empty already.
        self.playing.retain(|p| !p.player.empty());
        if let Some(assets) = request.assets.take() {
            self.assets = Some(assets);
        }
        let source = match self.decode(&request) {
            Some(source) => source,
            None => {
                // Nothing to play. Publish an already-finished player under this id so the
                // engine drops the sound on its next tick instead of tracking it forever.
                self.playing.push(Playing {
                    id: request.id,
                    player: Arc::new(Player::connect_new(&self.mixer)),
                    gains: Arc::new(Mutex::new((0.0, 0.0))),
                });
                self.publish();
                return;
            }
        };
        // Per-channel (left, right) gains, updated by the main thread and applied to the
        // source on the audio thread every GAIN_UPDATE_PERIOD.
        let gains = Arc::new(Mutex::new(request.gains));
        let updater = Arc::clone(&gains);
        let balanced = ChannelVolume::new(source, vec![request.gains.0, request.gains.1])
            .periodic_access(GAIN_UPDATE_PERIOD, move |channel_volume| {
                let (left, right) = *updater.lock().unwrap();
                channel_volume.set_volume(0, left);
                channel_volume.set_volume(1, right);
            });
        let player = Player::connect_new(&self.mixer);
        player.set_speed(request.pitch);
        player.append(balanced);
        self.playing.push(Playing {
            id: request.id,
            player: Arc::new(player),
            gains,
        });
        self.publish();
    }

    /// The decoded source for a request: streamed live, from the cache, or freshly
    /// decoded (and cached). `None` when the sound is missing or undecodable.
    fn decode(&mut self, request: &PlayRequest) -> Option<DecodedSource> {
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
        if let Some(sound) = self.cache.get(&request.path) {
            return Some(DecodedSource::Cached(CachedSamples::new(&sound)));
        }
        let sound = self
            .read(request)
            .and_then(|bytes| decode(bytes, &request.path))?;
        self.cache.insert(request.path.clone(), sound.clone());
        Some(DecodedSource::Cached(CachedSamples::new(&sound)))
    }

    /// The Ogg bytes of the request's sound, read from the assets.
    fn read(&mut self, request: &PlayRequest) -> Option<Vec<u8>> {
        let assets = self.assets.as_mut()?;
        let key = format!("assets/minecraft/{}", request.path);
        match assets.read_asset(&key) {
            Ok(bytes) => Some(bytes),
            Err(e) => {
                warn!("cannot read sound {}: {e:#}", request.path);
                None
            }
        }
    }

    /// Publish the live controls to the main thread.
    fn publish(&self) {
        *self.shared.lock().unwrap() = self
            .playing
            .iter()
            .map(|p| {
                (
                    p.id,
                    SoundControls {
                        player: Arc::downgrade(&p.player),
                        gains: Arc::clone(&p.gains),
                    },
                )
            })
            .collect();
    }
}

/// Real audio output: the channel to the decode worker, or nothing when there is no
/// device.
pub struct AudioOutput {
    /// `None` = silent fallback (no device or no worker; logged once in `open`).
    tx: Option<Sender<PlayRequest>>,
    /// Live controls, mirrored by the worker after each request it handles.
    shared: Arc<Mutex<Vec<(SoundId, SoundControls)>>>,
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
        let shared = Arc::new(Mutex::new(Vec::new()));
        let worker_shared = Arc::clone(&shared);
        let spawn = std::thread::Builder::new()
            .name("audio-decode".to_string())
            .spawn(move || run_worker(rx, sink.mixer().clone(), worker_shared));
        match spawn {
            // Detached: the worker loop ends when the last `AudioOutput` drops the sender,
            // which also drops the sink and stops the device stream.
            Ok(thread) => {
                std::mem::forget(thread);
                AudioOutput {
                    tx: Some(tx),
                    shared,
                }
            }
            Err(e) => {
                warn!("cannot start the audio decode worker ({e}); audio stays silent");
                AudioOutput::silent()
            }
        }
    }

    fn silent() -> AudioOutput {
        AudioOutput {
            tx: None,
            shared: Arc::new(Mutex::new(Vec::new())),
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

    /// The live controls, in play order. Empty on the silent path.
    fn controls(&self) -> Vec<(SoundId, SoundControls)> {
        self.shared.lock().unwrap().clone()
    }
}

/// The worker loop: play each request as it arrives, publishing the live controls.
fn run_worker(
    rx: Receiver<PlayRequest>,
    mixer: rodio::mixer::Mixer,
    shared: Arc<Mutex<Vec<(SoundId, SoundControls)>>>,
) {
    let mut worker = Worker {
        mixer,
        assets: None,
        cache: LruCache::new(CACHE_CAPACITY),
        playing: Vec::new(),
        shared,
    };
    for request in rx {
        worker.play(request);
    }
}

/// Decode Ogg Vorbis bytes fully into a `DecodedSound`; `None` (with a log) when they are
/// not decodable.
fn decode(bytes: Vec<u8>, path: &str) -> Option<DecodedSound> {
    let decoder = match Decoder::new_vorbis(Cursor::new(bytes)) {
        Ok(decoder) => decoder,
        Err(e) => {
            warn!("cannot decode {path}: {e}");
            return None;
        }
    };
    let channels = decoder.channels().get();
    let sample_rate = decoder.sample_rate().get();
    let samples: Vec<Sample> = decoder.collect();
    if channels == 0 || sample_rate == 0 || samples.is_empty() {
        warn!("decoded {path} is empty");
        return None;
    }
    Some(DecodedSound {
        samples: Arc::new(samples),
        channels,
        sample_rate,
    })
}

/// One sound the `SoundSystem` sent to the output, parallel to the engine's sound list:
/// the same order, the same length, kept in lockstep by `play`, `stop` and `tick`.
struct Sounded {
    id: SoundId,
    /// The engine's sound identity, used for `stop`'s path filter.
    path: String,
    source: SoundSource,
    /// The output's controls once the worker created the player; `None` until then.
    controls: Option<SoundControls>,
}

/// The facade over `SoundEvents`, the silent engine and the audio output.
///
/// `play` picks the event's variant, registers it with the engine and sends it to the
/// output worker; `tick` recomputes gains and keeps each playing sound's stereo gains and
/// speed in sync with the engine, and drops finished sounds from the engine.
pub struct SoundSystem {
    events: SoundEvents,
    engine: SilentEngine,
    output: AudioOutput,
    assets: haste_assets::Assets,
    /// The engine's sounds that were sent to the output, in play order.
    sounded: Vec<Sounded>,
    next_id: SoundId,
    /// Whether the worker already owns a copy of the assets (set on the first send).
    assets_sent: bool,
}

impl SoundSystem {
    /// Build the system from parsed `sounds.json` and the open assets, opening the
    /// default output device (or the silent fallback when there is none).
    pub fn new(events: SoundEvents, assets: haste_assets::Assets) -> Self {
        Self::with_output(events, assets, AudioOutput::open())
    }

    /// Build the system over a specific output (tests; callers managing the device).
    pub fn with_output(
        events: SoundEvents,
        assets: haste_assets::Assets,
        output: AudioOutput,
    ) -> Self {
        Self {
            events,
            engine: SilentEngine::new(),
            output,
            assets,
            sounded: Vec::new(),
            next_id: 0,
            assets_sent: false,
        }
    }

    /// Play a sound event at `pos` (or globally when `None`): pick the variant by `seed`
    /// (the same seed always picks the same variant), register it with the engine and
    /// hand it to the output worker. Unknown events do nothing.
    pub fn play(
        &mut self,
        event: &str,
        source: SoundSource,
        pos: Option<DVec3>,
        volume: f32,
        pitch: f32,
        seed: i64,
    ) {
        let Some(variant) = self.events.pick(event, seed) else {
            debug!("no variant for sound event {event}");
            return;
        };
        self.play_variant(&variant, source, pos, volume, pitch);
    }

    /// Play a concrete variant (the one `sounds.rs` picked, or a music module's choice).
    pub fn play_variant(
        &mut self,
        variant: &SoundVariant,
        source: SoundSource,
        pos: Option<DVec3>,
        volume: f32,
        pitch: f32,
    ) {
        self.engine.play(variant, source, pos, volume, pitch);
        // The engine's gains for this sound right now, so the player starts at the right
        // volume instead of full until the first tick.
        self.engine.tick();
        let gains = self.engine.gains().last().unwrap_or((0.0, 0.0));
        // The worker owns the assets after the first send and reads the Ogg itself.
        let assets = if self.assets_sent {
            None
        } else {
            match self.assets.reopen() {
                Ok(assets) => {
                    self.assets_sent = true;
                    Some(assets)
                }
                Err(e) => {
                    warn!("cannot reopen the assets for the audio worker: {e:#}");
                    None
                }
            }
        };
        let id = self.next_id;
        self.next_id += 1;
        self.output.send(PlayRequest {
            id,
            path: variant.path.clone(),
            assets,
            stream: variant.stream,
            gains,
            pitch,
        });
        self.sounded.push(Sounded {
            id,
            path: variant.path.clone(),
            source,
            controls: None,
        });
    }

    /// Stop playing sounds. `None` on an axis matches everything; a sound is stopped when
    /// both the source filter and the path filter match it.
    pub fn stop(&mut self, source: Option<SoundSource>, path: Option<&str>) {
        let matches = |s: &Sounded| {
            source.is_none_or(|src| s.source == src) && path.is_none_or(|p| s.path == p)
        };
        // Stop the players of the sounds being removed before dropping them from the
        // engine; sounds still decoding simply never start.
        let controls = self.output.controls();
        let stopped: Vec<SoundControls> = self
            .sounded
            .iter()
            .filter(|s| matches(s))
            .filter_map(|s| controls.iter().find(|(id, _)| *id == s.id).map(|(_, c)| c))
            .cloned()
            .collect();
        for control in stopped {
            if let Some(player) = control.player.upgrade() {
                player.stop();
            }
        }
        self.engine.stop(source, path);
        self.sounded.retain(|s| !matches(s));
    }

    /// Set the listener (camera) position and yaw for attenuation and panning.
    pub fn set_listener(&mut self, pos: DVec3, yaw_deg: f32) {
        self.engine.set_listener(pos, yaw_deg);
    }

    /// Set the volume of one sound source (or the master volume).
    pub fn set_volume(&mut self, source: SoundSource, volume: f32) {
        self.engine.set_volume(source, volume);
    }

    /// 20 Hz: recompute gains, push them and the pitches to the playing sounds, and drop
    /// finished sounds from the engine.
    pub fn tick(&mut self) {
        self.engine.tick();
        if !self.output.is_live() {
            return;
        }
        // Adopt the controls of players the worker created since the last tick.
        let controls = self.output.controls();
        for sounded in &mut self.sounded {
            if sounded.controls.is_none() {
                if let Some((_, control)) = controls.iter().find(|(id, _)| *id == sounded.id) {
                    sounded.controls = Some(control.clone());
                }
            }
        }
        let gains: Vec<(f32, f32)> = self.engine.gains().collect();
        let pitches: Vec<f32> = self.engine.pitches().collect();
        let mut finished = Vec::new();
        for (i, sounded) in self.sounded.iter().enumerate() {
            let Some(control) = &sounded.controls else {
                continue; // still decoding
            };
            // A gone player was pruned by the worker: it played through.
            let Some(player) = control.player.upgrade() else {
                finished.push(i);
                continue;
            };
            if player.empty() {
                finished.push(i);
                continue;
            }
            if let Some(&(left, right)) = gains.get(i) {
                *control.gains.lock().unwrap() = (left, right);
            }
            if let Some(&pitch) = pitches.get(i) {
                player.set_speed(pitch);
            }
        }
        // Highest index first so the lower indices stay valid.
        for i in finished.into_iter().rev() {
            self.engine.remove(i);
            self.sounded.remove(i);
        }
    }

    /// Number of sounds currently tracked as playing.
    pub fn playing(&self) -> usize {
        self.engine.playing()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sound(samples: Vec<Sample>, channels: u16) -> DecodedSound {
        DecodedSound {
            samples: Arc::new(samples),
            channels,
            sample_rate: 44_100,
        }
    }

    #[test]
    fn lru_cache_evicts_the_least_recently_used() {
        let mut cache = LruCache::new(2);
        cache.insert("a".into(), sound(vec![0.1], 1));
        cache.insert("b".into(), sound(vec![0.2], 1));
        assert_eq!(cache.paths(), vec!["a", "b"]);
        // Touch "a": "b" is now the least recently used and gets evicted by "c".
        assert!(cache.get("a").is_some());
        cache.insert("c".into(), sound(vec![0.3], 1));
        assert_eq!(cache.paths(), vec!["a", "c"]);
        assert!(cache.get("b").is_none());
        // Re-inserting an existing path refreshes it without duplicating.
        cache.insert("a".into(), sound(vec![0.4], 1));
        assert_eq!(cache.paths(), vec!["c", "a"]);
    }

    #[test]
    fn cached_samples_average_channels_to_mono() {
        let decoded = sound(vec![0.2, 0.6, 0.4, 0.8], 2);
        let mut cached = CachedSamples::new(&decoded);
        assert_eq!(cached.channels().get(), 1);
        assert_eq!(cached.sample_rate().get(), 44_100);
        assert_eq!(cached.next(), Some(0.4));
        assert_eq!(cached.next(), Some(0.6));
        assert_eq!(cached.next(), None);
    }

    #[test]
    fn decode_rejects_non_vorbis_bytes() {
        // A tiny valid Ogg Vorbis file cannot be produced without an encoder dependency
        // (rodio only decodes), so the decode-worker test covers the rejection path; the
        // `real_device_plays_harp` test below decodes a real Ogg from the assets.
        assert!(decode(b"not an ogg".to_vec(), "sounds/x.ogg").is_none());
    }

    #[test]
    fn silent_output_accepts_every_call() {
        let output = AudioOutput::silent();
        assert!(!output.is_live());
        output.send(PlayRequest {
            id: 0,
            path: "sounds/dig/stone1.ogg".into(),
            assets: None,
            stream: false,
            gains: (1.0, 1.0),
            pitch: 1.0,
        });
        assert!(output.controls().is_empty());
        output.send(PlayRequest {
            id: 1,
            path: "sounds/x.ogg".into(),
            assets: None,
            stream: true,
            gains: (0.0, 0.0),
            pitch: 0.5,
        });
        assert!(output.controls().is_empty());
    }

    /// The real assets when the cache is present, else `None` (the caller skips).
    fn cached_assets() -> Option<haste_assets::Assets> {
        let jar = haste_core::paths::client_jar();
        if !jar.exists() {
            return None;
        }
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").ok()
    }

    #[test]
    fn facade_plays_and_stops_through_the_silent_path() {
        let Some(assets) = cached_assets() else {
            // No downloaded assets in this environment; the silent path needs none of
            // them, but `SoundSystem` owns an `Assets` by value, so skip when absent.
            return;
        };
        let events = SoundEvents::parse(br#"{"e": {"sounds": ["dig/stone1"]}}"#).unwrap();
        let mut system = SoundSystem::with_output(events, assets, AudioOutput::silent());
        assert!(!system.output.is_live());
        system.set_listener(DVec3::ZERO, 0.0);
        system.set_volume(SoundSource::Master, 0.5);
        system.play(
            "e",
            SoundSource::Block,
            Some(DVec3::new(2.0, 0.0, 0.0)),
            1.0,
            1.0,
            7,
        );
        assert_eq!(system.playing(), 1);
        system.tick();
        system.stop(Some(SoundSource::Block), None);
        assert_eq!(system.playing(), 0);
        system.tick();
        assert_eq!(system.playing(), 0);
        // An unknown event does nothing.
        system.play("nope", SoundSource::Block, None, 1.0, 1.0, 1);
        assert_eq!(system.playing(), 0);
    }

    /// Opens the real device and plays `minecraft:block.note_block.harp` from the real
    /// assets for 1 s. Run explicitly: `cargo test -p haste-audio -- --ignored`.
    #[test]
    #[ignore = "opens the real audio device and needs the downloaded assets in the cache"]
    fn real_device_plays_harp() {
        let assets = cached_assets().expect("the downloaded assets in the cache");
        let mut assets_owner = assets;
        let bytes = assets_owner
            .read_asset("assets/minecraft/sounds.json")
            .unwrap();
        let events = SoundEvents::parse(&bytes).unwrap();
        let mut system = SoundSystem::new(events, assets_owner);
        assert!(system.output.is_live(), "no audio output device");
        system.play(
            "block.note_block.harp",
            SoundSource::Record,
            None,
            1.0,
            1.0,
            42,
        );
        assert_eq!(system.playing(), 1);
        std::thread::sleep(Duration::from_secs(1));
        system.tick();
        system.stop(None, None);
        assert_eq!(system.playing(), 0);
    }
}
