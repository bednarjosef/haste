# protocol-compat: everything a vanilla client owes a Java server

Worker: Opus. Worktree `$HASTE_WT/protocol-compat`, branch `protocol-compat`, test server port
**25619**. Read `docs/briefs/COMMON.md` first.

You own `crates/haste-protocol`. You may make the small edits in `crates/haste-client/src/net.rs`
needed to answer new packets (the lead merges; client-app has finished, player edits `player.rs`).

## Goal
Exact compatibility with vanilla and common modded/proxied Java servers: we decode every packet a
server sends correctly on every world, and we send everything a vanilla client sends, so no server,
proxy or anti-cheat can tell us apart at the protocol level.

## Task 0 (URGENT, first): chunk and light packets on a normal world
On a normal-terrain world (`scripts/test-server.sh protocol-compat 25619 1800 minecraft:normal`,
fresh `world/`), **every** `level_chunk_with_light` fails to decode ("9274 trailing bytes after
packet body") and `light_update` fails ("varint/varlong error: failed to fill whole buffer"). On
the flat test world they decode, so the bug is in a part a flat world barely exercises: heightmaps,
block entities, the light masks (NOTES.md claims "light-mask bit sets are VarLong-encoded": check
that claim, minecraft.wiki says BitSet = VarInt length + that many big-endian longs), the light
arrays. Fix it, with regression tests built from bytes you construct yourself. Then confirm with
`haste --headless` on the normal world (0 decode failures, non-empty sections > 0) and
`haste --screenshot` (send the lead the PNG path). Commit and SendMessage the lead "chunk decode
fixed" + hash immediately: the whole team is blocked on real terrain.

## Then, in order (commit after each)
1. **Decode audit.** Every clientbound play and configuration packet against minecraft.wiki's
   26.3 protocol page and Pumpkin's encoders (`cache/pumpkin`, server side: what it writes is
   what we read). Add a round-trip or golden test per packet where we can build the bytes.
   Run a long session on a normal world with mobs, chests, signs, item frames, players joining
   and leaving: zero decode failures.
2. **Configuration state, both directions.** `start_configuration` in play -> back to
   configuration (what Velocity/BungeeCord do on server switch) -> registry data again -> play,
   with the world reset; `select_known_packs`, `custom_payload` (`minecraft:brand` "vanilla"),
   `client_information` (locale, view distance, chat mode, skin parts, main hand, text
   filtering, server listings, particle status), cookies (`cookie_request`/`store_cookie`),
   `transfer` (reconnect to another host), `resource_pack_push/pop` (answer with the vanilla
   status sequence: accepted, downloaded, successfully loaded; or declined per config),
   `server_links`, `custom_report_details`, `code_of_conduct` if 26.3 has it, keep-alive and
   ping/pong in both states.
3. **Chat, exactly like vanilla.** Offline mode: unsigned `chat`/`chat_command` with the
   last-seen acknowledgement state (`message_signature` cache, `last_seen` window of 20,
   `chat_ack` when the pending count passes the threshold). Online mode: `chat_session_update`
   with the profile public key from setup-cli's `player_certificates` (being added on the
   setup-cli branch; define the type you need and say so), signed `chat` with salt, timestamp
   and signature over vanilla's signed body, `chat_command_signed` for commands with signable
   arguments. Pumpkin verifies signatures server-side: adapt its message-chain code. Test
   against the vanilla server in offline mode with `enforce-secure-profile=false`; online mode
   needs a real account (not available yet: build it, unit-test the signature bytes against a
   key you generate, and say what is unverified).
4. **Item stacks and data components.** Decode `ItemStackData::components_raw` into typed
   components (at least: custom_name, item_name, lore, enchantments, stored_enchantments,
   damage, max_damage, unbreakable, dyed_color, potion_contents, tool, item_model,
   custom_model_data, profile, container, bundle_contents; keep unknown ones raw), and the
   26.3 hashed stack form (`HashedStack`, CRC32C of each component's encoded value) for
   `container_click`. Pumpkin has component codecs. The jvm-host workstream needs both.
5. **Everything else a vanilla client sends unprompted**: `client_tick_end` (done by player),
   `chunk_batch_received` with vanilla's desired chunks-per-tick estimate, `configuration_acknowledged`,
   `accept_code_of_conduct` if present, `player_loaded` timing, `settings`. List what you could
   not determine.

## Done
`cargo test -p haste-protocol` and clippy clean; normal world 10 minutes, zero decode failures;
each task's verification described. Report what remains unverified (online mode, proxies).
