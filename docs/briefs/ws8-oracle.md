# WS8: oracles, test tiers, CI, benchmarks

You own `crates/haste-testkit`, `tools/recorder-mod` (Java, Fabric), `.github/workflows`,
`crates/haste-testkit/FORMATS.md`, and `docs/RECORDING.md`. Nothing in Haste may be written from
decompiled code, so your oracle data is how the other workstreams learn vanilla's behaviour.
Everything you record goes to `cache/oracle/26.3/` (gitignored); the formats and the tools are
committed.

## Task 1 (do this first, others are waiting): server-side oracle dumps
A Fabric mod that runs on the DEDICATED server (headless, no display) and on startup writes:
- `block_shapes.json`: for every block state id (iterate `Block.BLOCK_STATE_REGISTRY` / the state
  definition of every registered block): collision shape boxes and outline shape boxes
  (`getCollisionShape`/`getShape` with an empty level and origin; note states whose shape depends
  on neighbours/position and mark them), plus `light_emission`, `is_air`, `is_solid_render`
  (opaque full cube), `fluid` state if any. Format in FORMATS.md, versioned, e.g.
  `{"version":1,"states":[{"id":0,"collision":[],"outline":[],"light":0,"air":true,"opaque":false}]}`.
- `entity_dimensions.json`: every entity type's default width/height/eye height, plus per-pose
  player dimensions.
- `block_properties.json`: friction (slipperiness), speed factor, jump factor per block (these
  are public getters on `Block`).
Use the mod's own API calls at runtime (observation), not decompiled code. Build it the spike way
(javac against `cache/versions/26.3/client.jar` + Fabric loader + sponge-mixin + the libraries in
`cache/server-spike/libraries`, see `../haste-spike/scripts/build-java.sh`),
run it on `cache/server-spike` (`fabric-server-launch.jar`, offline, see the spike's
`scripts/server.sh`; copy the server dir to `cache/oracle-server` so you do not disturb the spike).
Script: `tools/recorder-mod/dump-server-oracles.sh` producing the three files. Report sizes and
counts. Validate: `minecraft:stone` is a full cube, `minecraft:oak_slab[type=bottom]` is half,
`minecraft:torch` has empty collision, water has empty collision.

## Task 2: formats + harness (`haste-testkit`)
- `FORMATS.md` for everything in `cache/oracle/26.3`: the three dumps above, `traces/<name>.json`
  (physics: per tick input/pos/velocity/on_ground/pose/dimensions + the world seed and the fixed
  spawn used), `screenshots/<name>.png` + `<name>.json` (camera pos/yaw/pitch/fov, time of day,
  render distance, GUI hidden), `packets/<name>.bin` (length-prefixed clientbound play packets,
  decompressed, with a small header: protocol version, timestamp).
- Rust loaders for each (`serde`), `replay_trace`, `compare_screenshot` (per-pixel tolerance +
  percentage + diff image), a `Bench` recorder (JSON lines with frame times, RSS from
  `/proc/self/status`, section counts) and a Markdown table writer.
- A packet recorder binary `haste-packet-proxy` (TCP proxy for OFFLINE servers: handles the
  handshake/login/config/play framing and compression threshold, records decompressed clientbound
  play packets, forwards everything unchanged). Useful for WS1/WS2 fixtures from any client.
- Tests: format round trips; screenshot compare on generated images; proxy framing unit tests.

## Task 3: client-side recorder + instructions
The vanilla CLIENT (with Fabric) is needed for physics traces, screenshots and benchmark
comparisons; it needs a display and the user's machine. Write the client part of the recorder mod:
- a `/haste record <trace-name>` command (or a key) that runs scripted input sequences (walk,
  sprint, jump, sprint-jump, stop, sneak, walk into a wall, step up a slab, swim, fall) by setting
  the key states programmatically each tick, and records the LocalPlayer's input/pos/velocity/
  on_ground/pose each tick to `traces/<name>.json`;
- `/haste screenshot <name>` at fixed camera positions in a fixed-seed flat+normal world; writes
  the PNG and the camera JSON;
- `/haste bench <route-name>` flies a fixed route for 60 s recording frame times + RSS (for the
  vanilla and vanilla+Sodium numbers).
Then write `docs/RECORDING.md`: exactly what the user runs (one command via WS7's launcher when it
exists; until then the manual java command line), what to click, how long it takes, where files
land, how to verify. Do not run the client yourself; report that it needs the user.

## Task 4: CI and test tiers
- `.github/workflows/ci.yml`: fmt, clippy `-D warnings`, `cargo test` fast tier (no cache), on
  push/PR, pinned toolchain, cached target. A nightly job placeholder for the cache-dependent tier.
- `docs/TESTING.md`: the three tiers (ms: pure functions; s: headless replays against oracle
  data; before merge: screenshot diffs) and how each workstream plugs in.

Do not touch other crates. Interface needs (e.g. a screenshot hook in the renderer) go to the lead.
