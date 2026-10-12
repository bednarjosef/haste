# Common rules for every workstream brief (session 2)

You are one of eight workstreams building Haste, a Minecraft: Java Edition 26.3 client in Rust that
will later run Fabric mods through a compatibility layer. The lead designs interfaces, reviews and
merges. Read these first, in your worktree: `AGENTS.md` (rules, architecture, legal),
`docs/INTERFACES.md` (the contract your crate implements and the ones you consume),
`crates/haste-core/src/*.rs` (shared types; do not edit), `NOTES.md` (decisions so far).

## The goal of this session
A client you can play: join a real 26.3 server, see the world in a window, walk, see the HUD,
chat and open the inventory, hear sounds. **Exact compatibility with vanilla Java servers is a
hard requirement**: every packet byte-exact, every state transition and acknowledgement a vanilla
client performs, no kicks, no "moved wrongly" corrections. Speed matters too: no per-block or
per-packet allocations in hot paths, measure before claiming.

## Environment
- Your worktree: `$HASTE_WT/<ws>` (`$HASTE_WT` is the `haste-wt` directory next to the repo) on
  branch `<ws>`. Work only there. Run `source scripts/env.sh` in every shell.
- `cache/` (symlink to the shared cache, gitignored): `cache/jdk/bin/java` (JDK 25),
  `cache/versions/26.3/client.jar` and `server.jar` (Mojang, never copy into the tree),
  `cache/datagen/26.3/reports/{blocks,registries,packets}.json` (data generator output),
  `cache/oracle/26.3/` (oracle dumps from the recorder mod: block shapes, entity dimensions, block
  properties; traces/screenshots are still empty), `cache/pumpkin/` (Pumpkin server, GPL-3.0),
  `cache/pomme/` (Pomme client, GPL-3.0).
- **Your own test server**: `scripts/test-server.sh <ws> <your port> [seconds]` starts a private
  copy of the local Fabric 26.3 server (offline mode, flat creative world) on the port your brief
  names; `HASTE_SERVER=vanilla` runs Mojang's plain server jar instead (the purer oracle). Its log
  (`cache/servers/<ws>/logs/latest.log`) is the server's verdict on our client: kicks, "moved
  wrongly", "moved too quickly", protocol errors. Kill your server when you are done with it.
  Headless join: `cargo run --release --bin haste -- --headless --server 127.0.0.1:<port> --seconds 30`.
- Builds: eight workstreams share 16 cores and 30 GB. Use `cargo build -j 6` / `cargo test -j 6`,
  prefer `-p <crate>` over whole-workspace builds, and do not run release builds in a loop.
- Toolchain is pinned stable (`rust-toolchain.toml`). No nightly, no azalea.

## Reuse
- **Pumpkin** (`cache/pumpkin`, GPL-3.0) may be copied or adapted freely where it helps us move
  fast and stay faithful: packet codecs, NBT, text components, item stack and data component
  encodings, registry handling, chat signing, block/fluid/item data handling, anything server-side
  that mirrors what the client must send or understand. Keep the GPL header and the Pumpkin
  attribution at the top of each copied or adapted file and add a line to `THIRD_PARTY.md`.
  Adapt it to our types (`haste_core`, our crates); do not import Pumpkin's crate structure,
  `pumpkin-data` generated tables or its build-time downloads. Pumpkin is a server and is not
  always vanilla-exact: when it and the vanilla server disagree, the vanilla server wins.
- **Pomme** (`cache/pomme`, GPL-3.0) likewise, except Pomme's physics, player and light-engine code
  (provenance problem: it is a port of vanilla) which must not be reused.
- Never translate decompiled Minecraft code. Sources: minecraft.wiki (protocol, formats),
  observation against our test servers, oracle data, Pumpkin/Pomme as above.

## Rules
- Edit only the crates and files your brief names. Never edit `haste-core`, `docs/INTERFACES.md`,
  `crates/haste-client/src/main.rs`, the root `Cargo.toml` or other workstreams' crates.
  Interface or dependency changes: write the exact Rust signature or crate+version and why in your
  report, and continue on something that does not need it. The lead answers quickly.
- Architecture (AGENTS.md): game state lives in Rust; tier-3 vanilla Java runs real classes loaded
  from the player's jar; one canonical Java proxy per Rust object; GUI is one batched `GuiFrame`
  per frame. Do not add designs that would make Phase 2 (Fabric mods, Mixin) harder: keep game
  state in plain Rust structs with clear ownership, no global singletons that Java proxies cannot
  reach.
- Legal: never commit anything from `cache/` or generated from Mojang files.
- Tests define done: `cargo test -p <crate>` and `cargo clippy -p <crate> --all-targets -- -D warnings`
  clean, `cargo fmt` applied. Tests needing `cache/` or a server are `#[ignore]` with a message;
  you run them with `--ignored` yourself and quote the output in your report.
- Commit to your branch often with clear messages (`git add -A && git commit -m ...`). No
  attribution trailers in commit messages. Never push. The only merge you may run is `git merge main`
  into your own branch when the lead tells you main has something you need. Never rebase, never
  touch other branches, never `rm -rf` anything outside `target/`.
- Keep `crates/<crate>/NOTES.md` short: decisions, dead ends, what is approximate.
- Plain names: name things after what they are, no code names or abbreviations.

## Report (when you finish your brief, or when blocked)
1. What is done, with the test commands and an output summary (and the server-log verdict for
   anything that talks to a server).
2. Commits on your branch (`git log --oneline main..HEAD`).
3. Interface change requests (exact signatures) and new dependency requests.
4. What is approximate, blocked, or not determined. Never fill gaps with guesses.
5. Two or three lines for `NOTES.md` (decisions and dead ends worth keeping).
