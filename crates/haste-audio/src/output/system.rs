//! `SoundSystem`: the facade over `SoundEvents`, the silent engine and the audio output.

use std::collections::HashSet;
use std::sync::Arc;

use glam::DVec3;
use tracing::{debug, warn};

use super::{AudioOutput, PlayRequest, SoundControls, SoundId};
use crate::channels::{ChannelPools, Pool};
use crate::engine::{event_eq, PlayResult, SilentEngine, SoundSource};
use crate::sound_event_listener::{PlayedSound, SoundEventListeners};
use crate::sounds::{SoundEvents, SoundVariant};

/// One sound the `SoundSystem` sent to the output, parallel to the engine's sound list:
/// the same order, the same length, kept in lockstep by `play`, `stop` and `tick`.
pub(super) struct Sounded {
    pub(super) id: SoundId,
    /// The sound event this was started with, shared with the engine and interned (so
    /// tracking it costs one reference count): what `stop`'s event filter and
    /// [`SoundSystem::is_event_playing`] match on, as vanilla's
    /// `SoundManager.stop(Identifier, SoundSource)` matches the event, not a file.
    event: Arc<str>,
    source: SoundSource,
    /// The channel pool this sound's channel came from (streamed sounds stream), released
    /// when it finishes or is stopped.
    pub(super) pool: Pool,
    /// The output's controls once the worker created the player; `None` until then.
    controls: Option<SoundControls>,
}

/// The facade over `SoundEvents`, the silent engine and the audio output.
///
/// `play` picks the event's variant, registers it with the engine and sends it to the
/// output worker; `tick` recomputes gains and keeps each playing sound's stereo gains and
/// speed in sync with the engine, and drops finished sounds from the engine. A `tick`
/// with nothing new started allocates nothing: the controls arriving from the worker are
/// drained into a reused buffer and merged in id order.
pub struct SoundSystem {
    events: SoundEvents,
    engine: SilentEngine,
    /// The channel pools (vanilla's `Library` split, 247 static + 8 streaming): a sound
    /// that cannot get a channel is not started.
    pub(super) channels: ChannelPools,
    pub(super) output: AudioOutput,
    assets: haste_assets::Assets,
    /// The engine's sounds that were sent to the output, in play order.
    pub(super) sounded: Vec<Sounded>,
    /// The interned event names, so playing an event again clones a shared `Arc<str>`
    /// instead of building a new string per sound (the first play of a name allocates
    /// once; every later play of it only bumps the reference count).
    pub(super) interned: HashSet<Arc<str>>,
    next_id: SoundId,
    /// Whether the worker already owns a copy of the assets (set on the first send).
    assets_sent: bool,
    /// Controls drained from the worker since the last tick, reused across ticks.
    pub(super) incoming: Vec<SoundControls>,
    /// Indices of sounds whose player finished this tick, reused across ticks.
    pub(super) finished: Vec<usize>,
    /// The sound event listeners' record (vanilla `SoundEngine.listeners`, the subtitles).
    listeners: SoundEventListeners,
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
            channels: ChannelPools::new(),
            output,
            assets,
            sounded: Vec::new(),
            interned: HashSet::new(),
            next_id: 0,
            assets_sent: false,
            incoming: Vec::new(),
            finished: Vec::new(),
            listeners: SoundEventListeners::default(),
        }
    }

    /// The engine's sound list, for the per-instance controls (`instances.rs`).
    pub(super) fn engine_mut(&mut self) -> &mut SilentEngine {
        &mut self.engine
    }

    /// The parsed `sounds.json`, for the per-instance plays (`instances.rs`).
    pub(super) fn events(&self) -> &SoundEvents {
        &self.events
    }

    /// The interned name of an event, shared with every sound that plays it.
    fn intern(&mut self, event: &str) -> Arc<str> {
        if let Some(event) = self.interned.get(event) {
            return Arc::clone(event);
        }
        let event: Arc<str> = Arc::from(event);
        self.interned.insert(Arc::clone(&event));
        event
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
        self.play_variant(&variant, source, pos, volume, pitch, event);
    }

    /// Play a concrete variant (the one `sounds.rs` picked, or a music module's choice)
    /// under the sound event name `event`, which is remembered for `stop`'s event filter
    /// and [`Self::is_event_playing`].
    pub fn play_variant(
        &mut self,
        variant: &SoundVariant,
        source: SoundSource,
        pos: Option<DVec3>,
        volume: f32,
        pitch: f32,
        event: &str,
    ) {
        self.start(variant, source, pos, volume, pitch, event, false);
    }

    /// Start one variant (vanilla's `SoundEngine.play`); the new sound's id, or `None` when
    /// it was not started (the volume-0 rule, or no free channel). `looping` plays it on
    /// from the start at its end until it is stopped (a looping sound instance).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start(
        &mut self,
        variant: &SoundVariant,
        source: SoundSource,
        pos: Option<DVec3>,
        volume: f32,
        pitch: f32,
        event: &str,
        looping: bool,
    ) -> Option<SoundId> {
        let event = self.intern(event);
        // Vanilla's `SoundEngine.play` tells its listeners (the subtitles) before anything
        // can keep the sound from starting.
        self.listeners.notify(
            &self.events,
            &event,
            variant,
            source,
            pos,
            volume,
            pitch,
            looping,
        );
        // Vanilla's `SoundEngine.play` order: the volume-0 rule first (a sound at zero
        // volume is not started; music starts silently), then the channel acquisition
        // (a full pool drops the sound).
        match self
            .engine
            .play(variant, source, pos, volume, pitch, Arc::clone(&event))
        {
            PlayResult::NotStarted => return None,
            PlayResult::Started | PlayResult::StartedSilently => {}
        }
        let pool = if variant.stream {
            Pool::Streaming
        } else {
            Pool::Static
        };
        if !self.channels.acquire(pool) {
            self.engine.remove_last();
            return None;
        }
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
            looping,
            gains,
            pitch,
        });
        self.sounded.push(Sounded {
            id,
            event,
            source,
            pool,
            controls: None,
        });
        Some(id)
    }

    /// Stop playing sounds. `None` on an axis matches everything; a sound is stopped when
    /// both the source filter and the event filter match it, as vanilla's
    /// `SoundManager.stop(Identifier, SoundSource)` does: both `None` stops everything,
    /// one `None` stops that whole axis. An event that is not playing stops nothing, so
    /// an unknown event name never stops more than it should.
    pub fn stop(&mut self, source: Option<SoundSource>, event: Option<&str>) {
        let matches = |s: &Sounded| {
            source.is_none_or(|src| s.source == src) && event.is_none_or(|e| event_eq(&s.event, e))
        };
        // Tell the worker to stop the players; a sound still in the decode queue is
        // stopped by its own play handling, because the stop order queues behind its
        // play request in the shared channel. Each stopped sound gives its channel back
        // (`ChannelHandle.release`).
        for sounded in &self.sounded {
            if matches(sounded) {
                self.output.send_stop(sounded.id);
                self.channels.release(sounded.pool);
            }
        }
        // Stopped sounds are no longer tracked; controls the worker still queues for them
        // are skipped on the next tick's merge (their ids are not tracked any more), so the
        // controls of sounds that keep playing are never lost.
        self.sounded.retain(|s| !matches(s));
        self.engine.stop(source, event);
    }

    /// Set the listener (the camera) position and look (yaw/pitch) for attenuation and
    /// panning, vanilla's `SoundEngine.updateSource(Camera)`.
    pub fn set_listener(&mut self, pos: DVec3, yaw_deg: f32, pitch_deg: f32) {
        self.engine.set_listener(pos, yaw_deg, pitch_deg);
    }

    /// Whether a sound event listener is registered (vanilla `SoundManager.addListener` /
    /// `removeListener`: the subtitle overlay while "Show Subtitles" is on). While none
    /// is, plays record nothing.
    pub fn set_listening(&mut self, listening: bool) {
        self.listeners.set_listening(listening);
    }

    /// The sounds with a subtitle played since the last call, oldest first, for the
    /// listeners (vanilla `SoundEngine.notifyListeners`, batched per tick).
    pub fn drain_played(&mut self) -> std::vec::Drain<'_, PlayedSound> {
        self.listeners.drain()
    }

    /// The listener's position and orientation (vanilla `SoundEngine.getListenerTransform`),
    /// as the last [`Self::set_listener`] set it.
    pub fn listener(&self) -> crate::spatial::ListenerTransform {
        self.engine.listener()
    }

    /// Set the volume of one sound source (or the master volume).
    pub fn set_volume(&mut self, source: SoundSource, volume: f32) {
        self.engine.set_volume(source, volume);
    }

    /// Set one source's extra category gain (the music fade's lever), vanilla's
    /// `SoundEngine.updateCategoryVolume`: it multiplies with the option volumes.
    pub fn update_category_volume(&mut self, source: SoundSource, gain: f32) {
        self.engine.update_category_volume(source, gain);
    }

    /// 20 Hz: recompute gains, push them and the pitches to the playing sounds, and drop
    /// finished sounds from the engine. Allocates nothing when no sound started since the
    /// last tick (all scratch buffers are fields, cleared and refilled in place).
    pub fn tick(&mut self) {
        self.engine.tick();
        if !self.output.is_live() {
            return;
        }
        // Adopt the controls of players the worker created since the last tick. Both this
        // queue and `sounded` are ordered by id (ids are handed out in play order and the
        // worker queues controls in the order plays arrive), so one merge pass matches
        // them with no per-sound scan.
        self.incoming.clear();
        self.output.drain_controls(&mut self.incoming);
        let mut next_incoming = 0;
        for (i, sounded) in self.sounded.iter_mut().enumerate() {
            if sounded.controls.is_some() {
                continue;
            }
            // Controls of sounds stopped before adoption (`stop`, `stop_instance`) are
            // skipped: ids only grow, so anything below this sound's id is no longer
            // tracked.
            while self
                .incoming
                .get(next_incoming)
                .is_some_and(|c| c.id < sounded.id)
            {
                next_incoming += 1;
            }
            if self
                .incoming
                .get(next_incoming)
                .is_some_and(|c| c.id == sounded.id)
            {
                let controls = self.incoming[next_incoming].clone();
                next_incoming += 1;
                if controls.stereo {
                    // The worker decoded a multi-channel file: OpenAL plays such a
                    // buffer unpositioned, so the sound stops attenuating and panning.
                    self.engine.set_stereo(i);
                }
                sounded.controls = Some(controls);
            }
        }
        // Anything still queued belongs to a sound this side no longer tracks (stopped
        // before adoption); its player was stopped by the worker.
        self.finished.clear();
        {
            let (mut gains, mut pitches) = (self.engine.gains(), self.engine.pitches());
            for (i, sounded) in self.sounded.iter().enumerate() {
                // The iterators yield one item per sound, in play order; every branch
                // consumes exactly one pair so the next sound reads its own values.
                let Some(control) = &sounded.controls else {
                    let _ = (gains.next(), pitches.next()); // still decoding
                    continue;
                };
                // A gone player was pruned by the worker: it played through.
                let player = match control.player.upgrade() {
                    Some(player) => player,
                    None => {
                        self.finished.push(i);
                        let _ = (gains.next(), pitches.next());
                        continue;
                    }
                };
                if player.empty() {
                    self.finished.push(i);
                    let _ = (gains.next(), pitches.next());
                    continue;
                }
                if let Some((left, right)) = gains.next() {
                    if control.stereo {
                        // Unpositioned multi-channel playback: the engine's two gains are
                        // equal; the player's volume carries them both.
                        player.set_volume(left);
                    } else {
                        *control.gains.lock().unwrap() = (left, right);
                    }
                }
                if let Some(pitch) = pitches.next() {
                    player.set_speed(pitch);
                }
            }
        }
        // Highest index first so the lower indices stay valid. The engine iterators are
        // dropped by now, so `remove` can borrow the engine again. A sound that played
        // through gives its channel back.
        for i in self.finished.iter().rev() {
            self.channels.release(self.sounded[*i].pool);
            self.engine.remove(*i);
            self.sounded.remove(*i);
        }
    }

    /// Number of sounds currently tracked as playing.
    pub fn playing(&self) -> usize {
        self.engine.playing()
    }

    /// Number of sounds currently playing in one source, so a caller can tell its own
    /// sounds apart from the rest (block and mob sounds share the `playing` count).
    pub fn playing_in(&self, source: SoundSource) -> usize {
        self.engine.playing_in(source)
    }

    /// Whether a sound with this event name is playing (namespace-defaulted match), as
    /// vanilla's `SoundManager.isActive(SoundInstance)` answers for one sound rather
    /// than for a whole source.
    pub fn is_event_playing(&self, event: &str) -> bool {
        self.engine.is_event_playing(event)
    }

    /// The current (left, right) gains of all playing sounds, in play order (tests,
    /// diagnostics; the output stage pushes these to the players on `tick`).
    pub fn engine_gains(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        self.engine.gains()
    }
}
