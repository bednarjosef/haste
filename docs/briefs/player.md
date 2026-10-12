# player: the local player (walk milestone) and then interaction

Worker: Opus. Worktree `$HASTE_WT/player`, branch `player`, test server port **25612**.
Read `docs/briefs/COMMON.md` first.

You own `crates/haste-physics` and `crates/haste-client/src/player.rs` (new). You may make the
minimal edits to `crates/haste-client/src/net.rs` and `headless.rs` needed to route the player's
packets to `player.rs` and call its tick (the client-app workstream owns those files and is
replacing the stub world in them right now; keep your edits small and local, the lead merges).

## Goal
The local player moves exactly like vanilla: the vanilla server never corrects us ("moved
wrongly", "moved too quickly", position resets) and never kicks us, for walking, sprinting,
sneaking, jumping, stairs/slabs/step-up, falling, swimming, flying in creative, and teleports.
There are no recorded traces from the real client yet. Your oracles are: the vanilla server's own
movement checks (its log and correction packets), the documented numbers already in
`crates/haste-physics/NOTES.md`, Pumpkin's player-movement handling, and minecraft.wiki.

## Tasks, in order (commit after each)
1. **`LocalPlayer` in `player.rs`** with the API in docs/INTERFACES.md "haste-client player":
   owns `haste_physics::PlayerState` + `PlayerAttributes`, consumes the player packets
   (`player_position` + `accept_teleportation`, `player_loaded` after teleports, `player_abilities`,
   `update_attributes` for our entity id: movement speed, jump strength, gravity, step height,
   scale, sneaking speed..., `set_entity_motion` for us, `respawn`, `set_health`, `game_event`
   game-mode changes, `player_rotation`, `set_held_slot`, `mount`/vehicle ignore for now) and
   produces the serverbound packets a vanilla client sends per tick, in vanilla's order:
   `player_input` (only when the input flags change), `player_command` (sprint start/stop, leave
   bed, elytra...), the right one of `move_player_pos/rot/pos_rot/status_only` with vanilla's
   send rules (position delta threshold, the forced resend every 20 ticks, on_ground and
   horizontal_collision flags), `client_tick_end`. Move the existing ad-hoc player code out of
   `net.rs` into it.
2. **Physics completeness against the server.** Run scripted inputs headless against your vanilla
   server (`HASTE_SERVER=vanilla`; add a `--walk-script <file>` or similar test hook in
   `headless.rs`): walk a square, sprint-jump, sneak to an edge (sneaking must not fall off),
   climb stairs, step up slabs, fall from 20 blocks (fall damage must match the server's health
   update), swim and sink in water, fly in creative (double-tap space semantics are input-side:
   expose `toggle_flight`). Build the test terrain with server commands (`/fill`, `/setblock` from
   the server console via stdin of the server process, or op the bot and send `chat_command`).
   Every divergence is a bug: fix it in `haste-physics`, keep a regression test.
3. **The open question in NOTES.md** (falling-water height 1.0 vs 8/9): settle it against the
   server and delete the physics-side oracle loader in favour of `haste_data`'s `BlockPhysics`.
4. **Interaction** (after 1-3 are merged; tell the lead when you get here): block breaking with
   vanilla's client timing (`player_action` start/abort/finish with the sequence number,
   `block_changed_ack` handling, creative instant break, the break progress the renderer draws
   as cracks), placing (`use_item_on` with cursor hit position and face, from a ray cast over the
   outline shapes in the oracle), `use_item`, attacking entities (`attack`) and `swing`'s 26.3
   equivalent (`punch`; see NOTES.md), picking the block (`pick_item_from_block`), hotbar slot
   changes (`set_carried_item`). Mining speed depends on the held tool, its components and
   effects: put that logic in `haste-physics` or a new module there, and report what data you
   need from the data generator or the oracle that we do not have yet.

## Done
- `cargo test -p haste-physics` and the client's tests clean; the scripted walk runs with zero
  corrections and zero "moved wrongly" lines in the vanilla server's log (quote the log tail).
- Report which movement cases you could not verify and why.
