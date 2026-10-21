//! Decoded sounds for the output worker: the Ogg decode, the LRU cache of decoded
//! non-streamed sounds and the rodio `Source`s the worker plays them through.

use std::collections::VecDeque;
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use rodio::decoder::LoopedDecoder;
use rodio::{Decoder, Sample, Source};
use tracing::{debug, warn};

/// A fully decoded sound: interleaved samples with channel count and sample rate.
#[derive(Clone)]
pub(super) struct DecodedSound {
    pub(super) samples: Arc<Vec<Sample>>,
    pub(super) channels: u16,
    pub(super) sample_rate: u32,
}

/// Bounded LRU cache of decoded sounds, by path. The front is the least recently used.
pub(super) struct LruCache {
    entries: VecDeque<(String, DecodedSound)>,
    capacity: usize,
}

impl LruCache {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            capacity,
        }
    }

    /// The entry for `path`, moved to the back (most recently used).
    pub(super) fn get(&mut self, path: &str) -> Option<DecodedSound> {
        let index = self.entries.iter().position(|(p, _)| p == path)?;
        let (_, sound) = self.entries.remove(index).unwrap();
        self.entries.push_back((path.to_string(), sound.clone()));
        Some(sound)
    }

    /// Insert an entry; when full, the least recently used entry is evicted. Re-inserting
    /// an existing path refreshes it instead of duplicating it.
    pub(super) fn insert(&mut self, path: String, sound: DecodedSound) {
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
    pub(super) fn paths(&self) -> Vec<&str> {
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
pub(super) struct CachedSamples {
    samples: Arc<Vec<Sample>>,
    frame_len: usize,
    sample_rate: u32,
    /// Interleaved-sample cursor.
    cursor: usize,
    /// Start over at the end instead of ending (a looping sound instance, vanilla's
    /// `Channel.setLooping` on a static buffer).
    looping: bool,
}

impl CachedSamples {
    pub(super) fn new(sound: &DecodedSound) -> Self {
        Self {
            samples: Arc::clone(&sound.samples),
            frame_len: sound.channels as usize,
            sample_rate: sound.sample_rate,
            cursor: 0,
            looping: false,
        }
    }

    /// The same source, playing on from the start every time it reaches the end.
    pub(super) fn looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }
}

impl Iterator for CachedSamples {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if self.looping && self.cursor + self.frame_len > self.samples.len() {
            self.cursor = 0;
        }
        let frame = self
            .samples
            .get(self.cursor..self.cursor + self.frame_len)?;
        self.cursor += self.frame_len;
        Some(frame.iter().sum::<Sample>() / self.frame_len as Sample)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.looping {
            return (usize::MAX, None);
        }
        let frames = (self.samples.len() - self.cursor) / self.frame_len;
        (frames, Some(frames))
    }
}

impl Source for CachedSamples {
    fn current_span_len(&self) -> Option<usize> {
        // A loop never changes format, so it is one endless span.
        self.size_hint().1
    }

    fn channels(&self) -> rodio::ChannelCount {
        rodio::ChannelCount::new(1).expect("constant 1 is non-zero")
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        rodio::SampleRate::new(self.sample_rate).expect("validated non-zero at decode")
    }

    fn total_duration(&self) -> Option<Duration> {
        if self.looping {
            return None;
        }
        Some(Duration::from_secs_f64(
            self.size_hint().0 as f64 / self.sample_rate as f64,
        ))
    }
}

/// What the worker plays: a streamed decode (music), a looping streamed decode (the
/// ambient loops, vanilla's `LoopingAudioStream`), a cached mono sound (channels averaged
/// per frame), or a cached stereo file passed through interleaved and untouched.
pub(super) enum DecodedSource {
    Streamed(Decoder<Cursor<Vec<u8>>>),
    StreamedLooping(LoopedDecoder<Cursor<Vec<u8>>>),
    Cached(CachedSamples),
    Stereo(StereoSamples),
}

impl DecodedSource {
    /// The file's channel count. OpenAL plays a multi-channel buffer as-is,
    /// unpositioned, so everything above 1 bypasses the per-channel panning of
    /// `ChannelVolume` (which would average the channels down to mono).
    pub(super) fn channels(&self) -> u16 {
        match self {
            // The decoder reports the file's own channels.
            DecodedSource::Streamed(decoder) => decoder.channels().get(),
            DecodedSource::StreamedLooping(decoder) => decoder.channels().get(),
            DecodedSource::Cached(_) => 1,
            DecodedSource::Stereo(stereo) => stereo.channels,
        }
    }
}

impl Iterator for DecodedSource {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        match self {
            DecodedSource::Streamed(decoder) => decoder.next(),
            DecodedSource::StreamedLooping(decoder) => decoder.next(),
            DecodedSource::Cached(cached) => cached.next(),
            DecodedSource::Stereo(stereo) => stereo.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            DecodedSource::Streamed(decoder) => decoder.size_hint(),
            DecodedSource::StreamedLooping(decoder) => decoder.size_hint(),
            DecodedSource::Cached(cached) => cached.size_hint(),
            DecodedSource::Stereo(stereo) => stereo.size_hint(),
        }
    }
}

impl Source for DecodedSource {
    fn current_span_len(&self) -> Option<usize> {
        match self {
            DecodedSource::Streamed(decoder) => decoder.current_span_len(),
            DecodedSource::StreamedLooping(decoder) => decoder.current_span_len(),
            DecodedSource::Cached(cached) => cached.current_span_len(),
            DecodedSource::Stereo(stereo) => stereo.current_span_len(),
        }
    }

    fn channels(&self) -> rodio::ChannelCount {
        match self {
            DecodedSource::Streamed(decoder) => decoder.channels(),
            DecodedSource::StreamedLooping(decoder) => decoder.channels(),
            DecodedSource::Cached(cached) => cached.channels(),
            DecodedSource::Stereo(stereo) => stereo.channels(),
        }
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        match self {
            DecodedSource::Streamed(decoder) => decoder.sample_rate(),
            DecodedSource::StreamedLooping(decoder) => decoder.sample_rate(),
            DecodedSource::Cached(cached) => cached.sample_rate(),
            DecodedSource::Stereo(stereo) => stereo.sample_rate(),
        }
    }

    fn total_duration(&self) -> Option<Duration> {
        match self {
            DecodedSource::Streamed(decoder) => decoder.total_duration(),
            DecodedSource::StreamedLooping(decoder) => decoder.total_duration(),
            DecodedSource::Cached(cached) => cached.total_duration(),
            DecodedSource::Stereo(stereo) => stereo.total_duration(),
        }
    }
}

/// An interleaved multi-channel `Source` over an `Arc`ed, already-decoded sample buffer:
/// the samples pass through untouched (no downmix), with the file's channel count.
#[derive(Clone)]
pub(super) struct StereoSamples {
    samples: Arc<Vec<Sample>>,
    channels: u16,
    sample_rate: u32,
    /// Sample cursor.
    cursor: usize,
    /// Start over at the end instead of ending (see [`CachedSamples::looping`]).
    looping: bool,
}

impl StereoSamples {
    pub(super) fn new(sound: &DecodedSound) -> Self {
        Self {
            samples: Arc::clone(&sound.samples),
            channels: sound.channels,
            sample_rate: sound.sample_rate,
            cursor: 0,
            looping: false,
        }
    }

    /// The same source, playing on from the start every time it reaches the end.
    pub(super) fn looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }
}

impl Iterator for StereoSamples {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if self.looping && self.cursor >= self.samples.len() {
            self.cursor = 0;
        }
        self.samples.get(self.cursor).map(|s| {
            self.cursor += 1;
            *s
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.looping {
            return (usize::MAX, None);
        }
        let left = self.samples.len() - self.cursor;
        (left, Some(left))
    }
}

impl Source for StereoSamples {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> rodio::ChannelCount {
        rodio::ChannelCount::new(self.channels).expect("validated non-zero at decode")
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        rodio::SampleRate::new(self.sample_rate).expect("validated non-zero at decode")
    }

    fn total_duration(&self) -> Option<Duration> {
        if self.looping {
            return None;
        }
        Some(Duration::from_secs_f64(
            self.size_hint().0 as f64 / self.channels as f64 / self.sample_rate as f64,
        ))
    }
}

/// Decode Ogg Vorbis bytes fully into a `DecodedSound`; `None` (with a log) when they are
/// not decodable.
pub(super) fn decode(bytes: Vec<u8>, path: &str) -> Option<DecodedSound> {
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
