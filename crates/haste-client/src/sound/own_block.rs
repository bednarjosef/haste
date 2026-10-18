//! Block sounds: the local player's own (mining, breaking, placing) and the ones
//! the server's `level_event` asks for (a break, a mining tick). Footsteps live in
//! `steps.rs`, fall sounds in `falls.rs`.

use glam::DVec3;
use haste_audio::random::LegacyRandom;
use haste_core::BlockPos;

use super::{PlayRequest, SoundPos, SoundRef, SoundRequest};

/// The local player's own block sounds (vanilla plays these client-side: the
/// server excludes the acting player from its broadcasts, so a client that
/// waits for packets stays silent while mining, placing).
///
/// The rules are vanilla's, read from the 26.3 client:
/// - mining hit: `MultiPlayerGameMode.startDestroyBlock` /
///   `continueDestroyBlock` call `addBreakingBlockEffects(playSound=true)`
///   on the *start* tick only (the server broadcasts the 2020 follow-ups
///   itself, every 4th tick: `ServerPlayerGameMode`). Hit volume is the
///   sound type's `(v + 1) / 8`, pitch `p * 0.5`;
/// - break: the server's `level_event` 2001 broadcast excludes nobody, so the
///   client gets it as a packet — but it does not decode that packet yet, so
///   we play it ourselves at the moment we predict the block gone: volume
///   `(v + 1) / 2`, pitch `p * 0.8` (`LevelEventHandler` case 2001);
/// - place: `BlockItem.place` plays on both sides; the client plays it when
///   it sends `use_item_on` with a block item (the server's copy never
///   reaches us — same `playSound(player, ...)` exclusion). Volume
///   `(v + 1) / 2`, pitch `p * 0.8`.
///
/// Blocks whose sound type the dump does not know (mods) play nothing — no
/// guess, no panic. Every request goes through the normal `SoundRequest`
/// path, so headless counts them and the game thread plays them through
/// `Sounds::apply` with the category volumes applied.
pub struct OwnBlockSounds {
    /// `ClientLevel.random`: `playLocalSound` seeds each sound instance with its
    /// `nextLong()`, so footsteps and hits pick a different file each time instead of
    /// always the same one.
    random: LegacyRandom,
}

/// `SoundSource.BLOCKS` for the block-driven ones (mining hit, break, place).
const CATEGORY_BLOCKS: i32 = 4;

impl OwnBlockSounds {
    pub fn new() -> Self {
        Self {
            random: LegacyRandom::new(unseeded()),
        }
    }

    /// A block break we predicted (the moment the block goes for us).
    pub fn block_broken(
        &mut self,
        pos: DVec3,
        t: &haste_data::BlockSoundType,
        requests: &mut Vec<SoundRequest>,
    ) {
        break_request(pos, t, self.random.next_long(), requests);
    }

    /// The first hit of mining a block (start tick only).
    pub fn mining_hit(
        &mut self,
        pos: DVec3,
        t: &haste_data::BlockSoundType,
        requests: &mut Vec<SoundRequest>,
    ) {
        hit_request(pos, t, self.random.next_long(), requests);
    }

    /// A block placement we initiated (`use_item_on` with a block item).
    pub fn block_placed(
        &mut self,
        pos: DVec3,
        t: &haste_data::BlockSoundType,
        requests: &mut Vec<SoundRequest>,
    ) {
        // BlockItem.place: place at (v+1)/2, pitch*0.8, BLOCKS source.
        play_event(
            self.random.next_long(),
            &t.place_sound,
            CATEGORY_BLOCKS,
            pos,
            (t.volume + 1.0) / 2.0,
            t.pitch * 0.8,
            requests,
        );
    }
}

impl Default for OwnBlockSounds {
    fn default() -> Self {
        Self::new()
    }
}

/// The sound a server `level_event` asks for (`LevelEventHandler` cases 2001 and
/// 2020; `Level.playLocalSound(BlockPos, ...)` plays at the block centre): a
/// break (2001) the block's break sound, a mining tick (2020) its hit sound.
/// Every other event plays nothing, as does vanilla's `intentionally_empty`.
///
/// The server excludes the breaker from its 2001 broadcast
/// (`Block.spawnDestroyByEntityParticles` with the player as the source), so
/// playing this on the wire does not double the local player's own break sound.
pub fn level_event_sound(
    event: i32,
    pos: BlockPos,
    t: &haste_data::BlockSoundType,
    requests: &mut Vec<SoundRequest>,
) {
    let pos = pos.as_dvec3() + DVec3::new(0.5, 0.5, 0.5);
    let seed = unseeded();
    match event {
        2001 => break_request(pos, t, seed, requests),
        2020 => hit_request(pos, t, seed, requests),
        _ => {}
    }
}

/// The mining hit request (`ClientLevel.playBreakingSound`): hit sound, volume
/// `(v + 1) / 8`, pitch `p * 0.5`, BLOCKS source.
fn hit_request(
    pos: DVec3,
    t: &haste_data::BlockSoundType,
    seed: i64,
    requests: &mut Vec<SoundRequest>,
) {
    play_event(
        seed,
        &t.hit_sound,
        CATEGORY_BLOCKS,
        pos,
        (t.volume + 1.0) / 8.0,
        t.pitch * 0.5,
        requests,
    );
}

/// The break request (`LevelEventHandler` case 2001): break sound, volume
/// `(v + 1) / 2`, pitch `p * 0.8`, BLOCKS source.
fn break_request(
    pos: DVec3,
    t: &haste_data::BlockSoundType,
    seed: i64,
    requests: &mut Vec<SoundRequest>,
) {
    play_event(
        seed,
        &t.break_sound,
        CATEGORY_BLOCKS,
        pos,
        (t.volume + 1.0) / 2.0,
        t.pitch * 0.8,
        requests,
    );
}

/// One own-sound request: an inline event name the normal `Sounds::apply`
/// path resolves (known to `sounds.json` or dropped as unknown, logged once).
/// The position is the player (vanilla plays its own sounds at itself); the
/// break/place/hit ones are at the block, passed by the caller.
fn play_event(
    seed: i64,
    event: &str,
    category: i32,
    pos: DVec3,
    volume: f32,
    pitch: f32,
    requests: &mut Vec<SoundRequest>,
) {
    if event == "minecraft:intentionally_empty" {
        return; // vanilla's "no sound" event
    }
    requests.push(SoundRequest::Play(PlayRequest {
        sound: SoundRef::Inline(event.to_owned()),
        category,
        pos: SoundPos::Fixed(pos),
        volume,
        pitch,
        seed,
    }));
}

/// A seed for an unseeded generator (vanilla's `RandomSource.create()` takes a unique
/// seed per instance): the clock's nanoseconds.
pub(super) fn unseeded() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0x5eed)
}
