# WS1: haste-protocol + the M1 headless join

You own `crates/haste-protocol` and `crates/haste-client/src/{net.rs,headless.rs}`. Interfaces in
docs/INTERFACES.md. Milestone M1 is yours: `haste --headless --server 127.0.0.1:25565` joins a local
26.3 vanilla server (offline mode), stays connected for the requested time, tracks the world and
entities through `haste-world`/`haste-entity` (WS2/WS4 crates; consume their public API, if it is
not there yet call into stubs you keep in `net.rs` and swap later), prints chunk/entity counts and
the player position each second, and leaves cleanly. Then the same against an online-mode server
with a Microsoft account (tokens come from WS7's auth; coordinate the `Account` struct via the lead).

## Reuse decision
REUSE_DECISION

## Task 1: codec + connection state machine
- Wire primitives (VarInt/VarLong, strings with length limits, UUID, positions as packed longs
  via `BlockPos::to_packed`, angles as bytes, `Option`, length-prefixed arrays, bit sets, JSON
  text, network NBT (unnamed root, 1.20.2+), `Component` in NBT form), packet framing
  (length-prefixed; compression with threshold from `login_compression` using `flate2`), AES/CFB8
  encryption after `encryption_request` (`aes` + `cfb8` crates, RSA-encrypt the shared secret and
  verify token with the server's DER public key, Mojang session join
  `https://sessionserver.mojang.com/session/minecraft/join` for online mode), SRV lookup
  (`_minecraft._tcp`), handshake -> status (server list ping, for a `haste ping` subcommand) ->
  login -> configuration (answer `clientbound_known_packs` with none selected so the server sends
  full registry data; store `registry_data` by registry id with raw NBT entries; `finish_configuration`
  ack; `keep_alive` and `ping` answered in the task) -> play.
- Packet ids from `cache/datagen/26.3/reports/packets.json` (`PacketIds::load()`), per state and
  direction, mapping id <-> name. Never hard-code numbers; the enum variant names match the
  `minecraft:` names in that file.
- Tests: VarInt vectors from the wiki; framing/compression round trips; encryption round trip
  with a generated key; a handshake/login/config flow against a fake server (tokio in-process)
  that you write in tests; `PacketIds::load` on the real file (#[ignore]).

## Task 2: play packets, the ones M1 needs first, then all
Clientbound first: `login`, `respawn`, `player_position` (teleport, with the 26.x relative flags,
answer `accept_teleportation`), `set_chunk_cache_center`, `set_chunk_cache_radius`,
`level_chunk_with_light` (fill `haste_core::chunk::ChunkData`; the section bytes stay opaque to you;
`haste_world::wire` decodes them, WS2 provides it), `forget_level_chunk`, `light_update`,
`block_update`, `section_blocks_update`, `add_entity`, `remove_entities`, `move_entity_pos`,
`move_entity_pos_rot`, `move_entity_rot`, `teleport_entity`/`entity_position_sync`,
`rotate_head`, `set_entity_motion`, `set_entity_data` (metadata decode into `MetadataEntry`),
`set_equipment`, `player_info_update`/`player_info_remove`, `set_time`, `system_chat`,
`player_chat` (signed chat: decode, do not verify yet), `disconnect`, `keep_alive`, `ping`,
`set_health`, `set_experience`, `container_set_slot`/`container_set_content` (hotbar),
`set_held_slot`, `game_event`, `change_difficulty`, `server_data`, `tab_list`, `sound`/`sound_entity`
(decode only), `level_event`, `block_entity_data`, `update_tags`, `commands` (skip body), `recipe_*` (skip body).
Serverbound: `accept_teleportation`, `move_player_pos`/`pos_rot`/`rot`/`status_only`,
`client_information` (send after login with locale/view distance), `chat`, `chat_command`,
`keep_alive`, `pong`, `player_command` (sprint/sneak), `set_carried_item`, `swing`,
`player_action`, `use_item_on`, `use_item`, `interact`, `client_tick_end`, `player_input`.
Everything else: decode to `Unknown` for now, with a counter per id printed at disconnect; then
fill them in by frequency. Field layouts: minecraft.wiki `Java Edition protocol/Packets` for
26.3 (confirm the page's version; where the page lags, verify empirically against the local
server and write what you verified in NOTES.md). Pumpkin's packet structs (GPL-3.0,
`cache/pumpkin`) can be reused with attribution where they exist for 26.3 clientbound decoding.
- Tests: each packet has an encode/decode round trip on a hand-built instance; a recorded-packet
  replay test over `cache/oracle/26.3/packets/*.bin` when WS8 provides them (#[ignore]).

## Task 3: M1 headless join (`crates/haste-client/src/{net.rs,headless.rs}`)
- `net.rs`: a `NetDriver` that owns the `Connection`, drains packets each tick and applies them
  to `World` and `EntityStore` and to a small `LocalPlayerState` (position from teleports, health,
  held slot); sends `move_player_pos_rot` every tick with the current position (standing still is
  fine), `client_tick_end`, `player_input`; answers teleports. Exposes counters.
- `headless.rs`: tokio runtime, connect, 20 Hz loop for `--seconds`, print once per second:
  chunks loaded, sections non-empty, entities, player pos, packets/s, unknown packet ids seen.
  Exit code 0 if it stayed connected the whole time and received >= 1 chunk and the login packet.
- Local server for testing: `cache/server-spike` (Fabric, offline, `127.0.0.1:25565`, start with
  `cd cache/server-spike && ../jdk/bin/java -jar fabric-server-launch.jar nogui`; it has
  `pause-when-empty-seconds=-1`). Make a `scripts/local-server.sh` in your crate dir for it.
  Use `timeout` when you run it; stop it when done (`pkill -f fabric-server-launch`).
- Done when the headless run against the local server passes 60 s with zero disconnects, and the
  unknown-packet list is reported.

Do not touch: haste-world, haste-entity, haste-render, haste-physics internals; `main.rs` (ask the
lead for flags).
