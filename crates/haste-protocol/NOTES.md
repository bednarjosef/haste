# haste-protocol notes (protocol)

## Decisions
- **Reuse (lead's decision):** NBT, transport (framing, zlib, AES-CFB8) and VarInt codecs are vendored
  from Pumpkin (GPL-3.0, commit b0c48b2) and listed in THIRD_PARTY.md. NBT is a module (`src/nbt`), not a
  separate crate. Changes: edition-2021 fixes, a missing `TAG_End` is an error (Pumpkin treated EOF as the
  end of a compound, which hides truncated payloads), and the CFB8 writer was rewritten (Pumpkin's wrote
  one byte per inner `poll_write`).
- **Packets are our own structs** (`packet!` macro: one struct per `packets.json` entry, decode + encode,
  round-trip test each). Ids come from `packets.json` at runtime (`PacketIds`/`ProtocolTables`), and
  dispatch is an id-indexed table built at connect.
- **Strict decoding.** Every play packet must consume its whole body (`Trailing` error otherwise), and
  booleans must be 0 or 1. Vanilla is more lenient. Being strict turns layout mistakes into counted decode
  failures instead of silent garbage. A failed decode is delivered as `Unknown` and counted per name.
- **Raw where the layout is not needed or not determined:** registry entry NBT, block entity NBT, the
  chunk section payload (decoded by `haste_world`), `commands`, `update_recipes`, `recipe_book_*`, dialog
  NBT, debug packet payloads, `waypoint`, `test_instance_block_*`, and `place_ghost_recipe` display.
- **Item stacks:** data component patches have no length prefix, so skipping one needs each component's
  layout (`wire/slot.rs`, keyed by registry name from `registries.json`). The patch is kept raw in
  `ItemStackData::components_raw`. A component without a known layout fails the decode of the containing
  packet.
- **BitSets (26.3):** a VarInt byte count, then the longs as little-endian bytes with trailing zero bytes
  dropped (Pumpkin's `BitSet::encode_with_version`; the wiki still documents big-endian longs). Used by the
  light masks and the `player_chat` filter mask. `Reader::bit_set`/`Writer::bit_set` implement only this form.

- **Item stack templates (26.x):** `VarInt item, VarInt count, patch` (item first, never empty). Used by
  `container` (list of *optional* templates: bool + template), `bundle_contents`, `charged_projectiles`,
  `use_remainder`, `sulfur_cube_content`, `pot_decorations` (exactly four optional templates, no count)
  and advancement display icons. Observed on the vanilla server; `wire::slot::ItemStackTemplate`.
- `equippable` has five bools before the shearing sound (`can_be_sheared` is new). `attribute_modifiers`,
  `painting/variant` (Holder: id + 1) and `attack_animation` are implemented from observation/Pumpkin.
- `player_action` statuses were renumbered in 26.3 (`player_action_status`); the wiki is out of date.
  Lesson: check every enum-valued field's order against the server.

## Verified against the local 26.3 server (Fabric, offline, flat world, 2026-10-12)
- Login, configuration (known packs answered with none), play. 60 s headless run: 473 chunks, ~117
  entities, ~29k packets, **no unknown ids and no decode failures** with the full play packet set. That
  includes `update_advancements`, `update_attributes`, `set_passengers`, `entity_event` and
  `player_abilities`.
- **Dead end:** the light masks were first read as BitSets of VarLongs, which parsed flat-world chunks by
  accident (their small masks happen to line up) and failed every chunk and light packet on normal terrain.
  The byte form above decodes the vanilla server's normal world with zero failures (2026-10-13).
- Chunk sections carry a fluid count (short) after the block count (as the wiki says for 26.x).
- **Movement:** sending `move_player_pos_rot` right after `accept_teleportation`, before `player_loaded`,
  gets the client kicked with `multiplayer.disconnect.invalid_player_movement`. The driver now sends
  `accept_teleportation`, then `player_loaded`, and starts movement packets on the next tick.
- `accept_teleportation` carries position and rotation in 26.3 (as the wiki and Pumpkin say); the server
  accepted it.
- The local server has `white-list=true`. Offline user `haste` needs a whitelist entry (UUID
  78280295-106a-3287-91da-97b4b0c31f56).

## Not verified / approximate
- Online mode (encryption + session join) is tested only against the fake server, with
  `should_authenticate=false`. The session join request has not been made against Mojang.
- `MoveDeltas::total` for stepped entity movement assumes the step deltas are cumulative. Not determined.
- Layouts taken from Pumpkin where it disagrees with the wiki: `set_player_team` colour,
  `update_advancements` tree position, and `set_creative_mode_slot` (length-prefixed stack).
  `update_advancements` decoded on the server. The other two were never received or sent.
- Data components without a known layout are unsupported: `interact_animation`, `block_transformer`,
  `villager_food`, `compostable`, `cooking_fuel`, `brewing_fuel`, `mob_visibility`,
  `provides_pottery_pattern`, `sign_text_front`, `sign_text_back`, `waxed`, `cushion/color`.
- The inline painting variant (metadata and `painting/variant` id 0) reads width/height as Int; never
  observed (the server only sends registry ids).
- Particles `red/orange/yellow_poplar_leaves` have no documented data layout, so they are unsupported.
- Reconfiguration: the reader acknowledges, re-runs configuration with fresh registries and hands them to
  `Connection::registries()` when the game dequeues the following `login`. Whether vanilla resends brand and
  client information on reconfiguration is not determined (we do; servers accept it).
- Transfer: the old connection is closed first (connecting while it is open gets the old session kicked
  with "logged in from another location"), then handshake with intent 3; cookies (`CookieJar`) survive.
  Verified with vanilla `/transfer` to the same server. The pack is never downloaded: with
  `ResourcePackPolicy::Accept` we answer accepted, downloaded, loaded (a forced pack did not get us kicked).
- Known packs: we answer `select_known_packs` with none, so the server sends full registry data. A vanilla
  client answers `minecraft:core`; a server can tell the difference. Fixing it needs vanilla's registry
  data from the player's jar.

## Debugging
- `HASTE_PROTOCOL_DUMP=<dir>` writes the body of the first failing packet of each name to `<dir>/<name>.bin`.
