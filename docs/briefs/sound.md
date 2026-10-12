# sound: the haste-audio crate

Worker: GLM. Worktree `$HASTE_WT/sound`, branch `sound`, test server port **25615**.
Read `docs/briefs/COMMON.md` first.

You own `crates/haste-audio` (new, empty: only `Cargo.toml` and a header in `src/lib.rs`) and,
for task 4, `tools/recorder-mod/src/main/java/haste/recorder/ServerDumps.java`.
Dependencies available: `rodio` 0.23 (features `playback`, `vorbis`: cpal output and Ogg
Vorbis decoding), `haste-assets`, `haste-core`, `serde_json`, `glam`, `anyhow`, `tracing`.

## What vanilla does (minecraft.wiki: "Sounds.json", "Sound", "Java Edition protocol/Packets"
"Sound Effect", "Entity Sound Effect", "Stop Sound")
- `assets/minecraft/sounds.json` (in the assets index, not the jar) maps a sound event name to
  a list of sounds: each a name (`minecraft:dig/stone1` -> `sounds/dig/stone1.ogg`) or an object
  with `volume`, `pitch`, `weight`, `stream`, `attenuation_distance`, `preload`, `type: event`.
  One entry is picked at random by weight each time the event plays; `replace` and `subtitle`
  are fields too.
- The server's `sound` / `sound_entity` packets carry the event as a registry id
  (`minecraft:sound_event` in `cache/datagen/26.3/reports/registries.json`, id = protocol id) or
  inline (name + optional fixed range), a source category, position (fixed-point x8 for
  `sound`), volume, pitch and a seed. `stop_sound` stops by source and/or name.
- Volume above 1.0 extends the audible range (16 blocks x volume); attenuation is linear by
  distance; stereo panning by direction relative to the listener. Pitch is a playback-rate
  multiplier clamped to 0.5..2.0.

## API (put it in `lib.rs`; the lead copies it into docs/INTERFACES.md)
```rust
pub enum SoundSource { Master, Music, Record, Weather, Block, Hostile, Neutral, Player, Ambient, Voice, Ui }
pub struct SoundEngine { .. }
impl SoundEngine {
    /// Opens the default output device. With no device (CI, headless), returns an engine that
    /// accepts every call and plays nothing.
    pub fn new(assets: &haste_assets::Assets, sound_event_names: Vec<haste_core::Identifier>) -> anyhow::Result<SoundEngine>;
    pub fn play(&mut self, event: &haste_core::Identifier, source: SoundSource, pos: Option<glam::DVec3>, volume: f32, pitch: f32, seed: i64);
    pub fn play_registry(&mut self, id: u32, source: SoundSource, pos: Option<glam::DVec3>, volume: f32, pitch: f32, seed: i64);
    pub fn stop(&mut self, source: Option<SoundSource>, event: Option<&haste_core::Identifier>);
    pub fn set_listener(&mut self, pos: glam::DVec3, yaw: f32, pitch: f32);
    pub fn set_volume(&mut self, source: SoundSource, volume: f32);
    /// 20 Hz: update attenuation/panning of playing positional sounds, drop finished ones, music.
    pub fn tick(&mut self);
    pub fn playing(&self) -> usize;
}
```
Decoded sounds are cached (bounded, LRU); `stream: true` entries (music) are streamed, not
decoded whole. Playing must never block the caller (no decoding on the game thread: decode on a
worker thread or ahead of time). Use the `seed` with a small deterministic RNG for the weighted
pick, so the same packet picks the same variant every time (document what you could and could not
determine about vanilla's exact pick).

## Tasks, in order (commit after each)
1. `sounds.json` parsing + weighted pick (tests on a hand-written JSON).
2. The engine with rodio: positional volume and panning, sources and their volumes, stop,
   silent fallback. Unit tests for attenuation/panning math and the silent engine.
3. Music: the game music event (`minecraft:music.game`, creative `music.creative`, menu
   `music.menu`) with vanilla's delays between tracks (minecraft.wiki "Music"); the client will
   call `set_situation(MusicSituation)`; add that enum and method.
4. Block sound types for the client's own footsteps, block breaking and placing sounds (the
   server does not send these to the player who causes them). The mapping block -> sound type
   (break/step/place/hit/fall events, volume, pitch) is in vanilla code, not in the reports.
   Add it to the server-side oracle dump: extend `ServerDumps.java` to write
   `cache/oracle/26.3/block_sounds.json` (per block: the five event names, volume, pitch),
   rebuild with `tools/recorder-mod/build.sh` and regenerate with
   `tools/recorder-mod/dump-server-oracles.sh`. Load it in `haste-audio` (`BlockSounds::load`).
5. An `#[ignore]` test that opens the real assets (`cache/assets`, index from
   `haste_core::paths`) and plays `minecraft:block.note_block.harp` silently-or-not, checking that
   every sound event in the registry resolves to existing `.ogg` files.

## Done
`cargo test -p haste-audio` (and `--ignored`) and clippy clean; report the resolution check
numbers and the decode/cache memory for a typical session.
