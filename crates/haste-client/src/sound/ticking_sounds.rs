//! The ticking sound instances (vanilla `SoundEngine.tickingSounds` and the instance half
//! of `SoundEngine.tickInGameSound`): instances that keep changing after their play —
//! the underwater loop and additions, the biome loops. Each sound tick every playing
//! instance ticks itself; one that stopped itself is stopped on the output, the others
//! push their new volume. An instance whose sound has played through, or was stopped
//! from outside (`stop_sound`, a level change), leaves the list without being marked
//! stopped, as in vanilla.
//!
//! The instances live here, addressed by an [`InstanceId`]; a handler that keeps an
//! instance (vanilla `BiomeAmbientSoundsHandler.loopSounds`) *holds* it, so its state
//! stays reachable after its sound is gone, and *releases* it when it drops it. Runs on
//! the game thread at 20 Hz; a tick that starts nothing allocates nothing.

use haste_audio::engine::SoundSource;
use haste_audio::output::{InstancePlay, SoundHandle};
use haste_audio::random::LegacyRandom;

use super::biome_ambient_sounds_handler::LoopSoundInstance;
use super::sound_manager::SoundManager;
use super::under_liquid_ambient_sound_instance::UnderLiquidAmbientSoundInstance;
use super::under_liquid_sub_sound::UnderLiquidSubSound;

/// One instance's address in [`TickingSounds`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InstanceId(u64);

/// What a ticking instance reads off the local player on the sound tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TickingPlayer {
    /// The current local player (the generation [`super::AmbientSoundHandlers`] hands
    /// out; a respawn builds a new one and the old one counts as removed).
    pub player: u32,
    /// `LocalPlayer.isUnderWater`.
    pub under_water: bool,
}

impl TickingPlayer {
    /// `Entity.isRemoved` for the player an instance was made for.
    pub fn is_removed(&self, player: u32) -> bool {
        player != self.player
    }
}

/// The ticking instance types the ambient scheduler plays.
#[derive(Clone, Debug, PartialEq)]
pub enum TickableInstance {
    UnderLiquidAmbient(UnderLiquidAmbientSoundInstance),
    UnderLiquidSub(UnderLiquidSubSound),
    BiomeLoop(LoopSoundInstance),
}

impl TickableInstance {
    /// `TickableSoundInstance.tick`.
    fn tick(&mut self, player: &TickingPlayer) {
        match self {
            TickableInstance::UnderLiquidAmbient(i) => i.tick(player),
            TickableInstance::UnderLiquidSub(i) => i.tick(player),
            TickableInstance::BiomeLoop(i) => i.tick(),
        }
    }

    /// `TickableSoundInstance.isStopped`.
    pub fn is_stopped(&self) -> bool {
        match self {
            TickableInstance::UnderLiquidAmbient(i) => i.is_stopped(),
            TickableInstance::UnderLiquidSub(i) => i.is_stopped(),
            TickableInstance::BiomeLoop(i) => i.is_stopped(),
        }
    }

    /// The instance volume (`SoundInstance.getVolume` before the variant's factor).
    pub fn volume(&self) -> f32 {
        match self {
            TickableInstance::UnderLiquidAmbient(i) => i.volume(),
            TickableInstance::UnderLiquidSub(i) => i.volume(),
            TickableInstance::BiomeLoop(i) => i.volume(),
        }
    }

    /// The event it plays.
    pub fn event(&self) -> &str {
        match self {
            TickableInstance::UnderLiquidAmbient(i) => i.event(),
            TickableInstance::UnderLiquidSub(i) => i.event(),
            TickableInstance::BiomeLoop(i) => i.event(),
        }
    }

    /// `SoundInstance.isLooping` at the play: the two loops loop, the additions do not.
    fn looping(&self) -> bool {
        !matches!(self, TickableInstance::UnderLiquidSub(_))
    }
}

/// One instance and its sound.
#[derive(Debug)]
struct Entry {
    id: InstanceId,
    instance: TickableInstance,
    /// The playing sound; `None` once it is gone (or never started): not ticked any more.
    handle: Option<SoundHandle>,
    /// Kept by a handler, so it stays after its sound is gone until released.
    held: bool,
}

/// The ticking instances (vanilla `SoundEngine.tickingSounds`).
#[derive(Debug)]
pub struct TickingSounds {
    entries: Vec<Entry>,
    next_id: u64,
    /// Seeds of vanilla's `SoundInstance.createUnseededRandom()`, one per new instance.
    unseeded: LegacyRandom,
    /// A level change asked for every sound to stop (vanilla `SoundManager.stop()` in
    /// `Minecraft.updateLevelInEngines`), done on the next tick that has the output.
    stop_all_pending: bool,
}

impl Default for TickingSounds {
    fn default() -> Self {
        Self::new(super::own_block::unseeded())
    }
}

impl TickingSounds {
    /// An empty list whose unseeded instance randoms start from `seed`.
    pub fn new(seed: i64) -> Self {
        Self {
            entries: Vec::new(),
            next_id: 0,
            unseeded: LegacyRandom::new(seed),
            stop_all_pending: false,
        }
    }

    /// A seed for a new instance's own random (vanilla `createUnseededRandom`).
    pub fn unseeded_seed(&mut self) -> i64 {
        self.unseeded.next_long()
    }

    /// Play `instance` (vanilla `SoundManager.play` of a ticking instance): ambient,
    /// relative to the listener, at its volume, pitch 1. Started, it ticks from the next
    /// sound tick on; not started, it never ticks. `held` keeps it reachable for the
    /// handler that plays it (released with [`Self::release`]).
    pub fn play(
        &mut self,
        instance: TickableInstance,
        held: bool,
        out: &mut dyn SoundManager,
    ) -> InstanceId {
        let id = InstanceId(self.next_id);
        self.next_id += 1;
        let seed = self.unseeded_seed();
        let handle = out.play(&InstancePlay {
            event: instance.event(),
            source: SoundSource::Ambient,
            pos: None,
            volume: instance.volume(),
            pitch: 1.0,
            seed,
            looping: instance.looping(),
        });
        if handle.is_some() || held {
            self.entries.push(Entry {
                id,
                instance,
                handle,
                held,
            });
        }
        id
    }

    /// The instance at `id`, while it is playing or held.
    pub fn get(&self, id: InstanceId) -> Option<&TickableInstance> {
        self.entries
            .iter()
            .find(|e| e.id == id)
            .map(|e| &e.instance)
    }

    /// The instance at `id`, mutably (a handler fading its loop).
    pub fn get_mut(&mut self, id: InstanceId) -> Option<&mut TickableInstance> {
        self.entries
            .iter_mut()
            .find(|e| e.id == id)
            .map(|e| &mut e.instance)
    }

    /// The handler no longer keeps `id`: it goes once its sound is gone (at once if it
    /// is). A released instance that still plays keeps ticking, as an instance a dropped
    /// handler started does in vanilla.
    pub fn release(&mut self, id: InstanceId) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.held = false;
        }
        self.entries.retain(|e| e.handle.is_some() || e.held);
    }

    /// Whether `id` has a playing sound (tests, diagnostics).
    #[cfg(test)]
    pub fn is_playing(&self, id: InstanceId) -> bool {
        self.entries
            .iter()
            .any(|e| e.id == id && e.handle.is_some())
    }

    /// How many instances are playing or held.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is playing or held.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Ask for every sound here to stop on the next tick that has the output (a level
    /// change; vanilla `Minecraft.updateLevelInEngines` stops all sounds).
    pub fn request_stop_all(&mut self) {
        self.stop_all_pending = true;
    }

    /// Carry out a pending [`Self::request_stop_all`]: every playing sound stops and
    /// leaves the list; the instances are not marked stopped (vanilla stops channels, not
    /// instances), so a held one stays with its handler, silent.
    pub fn flush_stop_all(&mut self, out: &mut dyn SoundManager) {
        if !std::mem::take(&mut self.stop_all_pending) {
            return;
        }
        for entry in &mut self.entries {
            if let Some(handle) = entry.handle.take() {
                out.stop(handle);
            }
        }
        self.entries.retain(|e| e.held);
    }

    /// The sound tick (vanilla `SoundEngine.tickInGameSound` for ticking instances):
    /// every playing instance ticks; one that stopped itself is stopped on the output,
    /// the others push their volume. Then instances whose sound is gone leave the list
    /// (unless held).
    pub fn tick(&mut self, player: &TickingPlayer, out: &mut dyn SoundManager) {
        self.flush_stop_all(out);
        for entry in &mut self.entries {
            let Some(handle) = entry.handle else {
                continue;
            };
            entry.instance.tick(player);
            if entry.instance.is_stopped() {
                out.stop(handle);
                entry.handle = None;
            } else if out.is_active(handle) {
                out.set_volume(handle, entry.instance.volume());
            } else {
                // Played through or stopped from outside: off the ticking list.
                entry.handle = None;
            }
        }
        self.entries.retain(|e| e.handle.is_some() || e.held);
    }
}

#[cfg(test)]
mod tests;
