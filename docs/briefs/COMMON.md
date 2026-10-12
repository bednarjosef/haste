# Common rules for every workstream brief

You are one of eight workstreams building Haste, a Minecraft 26.3 client in Rust. The lead designs
interfaces, reviews and merges. Read these first, in your worktree:
`AGENTS.md` (rules, architecture, legal), `docs/INTERFACES.md` (the contract your crate implements
and the ones you consume), `crates/haste-core/src/*.rs` (shared types; do not edit).

## Environment
- Your worktree: `$HASTE_WT/<ws>` on branch `<ws>`. Work only there.
- `cache/` (symlink to the shared cache, gitignored): `cache/jdk/bin/java` (JDK 25),
  `cache/versions/26.3/client.jar` and `server.jar` (Mojang, do not copy anywhere),
  `cache/datagen/26.3/reports/{blocks,registries,packets}.json` (data generator output),
  `cache/oracle/26.3/` (oracle dumps, may be empty until WS8 produces them),
  `cache/server-spike/` (a working Fabric 26.3 dedicated server from the spike: `fabric-server-launch.jar`, `mods/`),
  `cache/pomme/` (Pomme client, GPL-3.0) and `cache/pumpkin/` (Pumpkin server, GPL-3.0) for reference.
- The spike repo `../haste-spike` is read-only reference (its REPORT.md, REPORT-2.md).
- `target/` is pre-populated with the workspace dependencies; `cargo build` should be incremental.
- Toolchain is pinned stable (`rust-toolchain.toml`). No nightly, no azalea.

## Rules
- Edit only your crates (named in your brief) and your module in `crates/haste-client` if the brief
  names one. Never edit `haste-core`, `docs/INTERFACES.md`, other crates, `Cargo.toml` at the root
  (ask the lead for new workspace dependencies: say crate, version, why).
- If you need an interface change, write it in your report (exact Rust signature, why) and keep
  going on something that does not need it.
- Legal: never commit anything from `cache/`; never translate decompiled Minecraft code; implement
  from the public protocol/format documentation (minecraft.wiki), from observation and from oracle
  data. Pomme and Pumpkin code may be reused under GPL-3.0 with the file header kept and an entry
  added to `THIRD_PARTY.md`, except Pomme's physics/player code (provenance problem) which must not be reused.
- Tests define done: `cargo test -p <crate>` and `cargo clippy -p <crate> -- -D warnings` clean.
  Tests needing `cache/` are `#[ignore]` with a message, and you run them with `--ignored` yourself.
- Commit to your branch often (`git add -A && git commit -m ...`). Never push. The only merge you
  may run is `git merge main` into your own branch to pick up the lead's merges; never touch other
  branches, never rebase. The lead merges your branch into `main` through a pull request.
- Keep a short `crates/<crate>/NOTES.md`: decisions, dead ends, what is approximate.

## Report (send it when you finish a milestone task or get stuck)
1. What is done, with the test command and its output summary.
2. Commits on your branch (`git log --oneline main..HEAD`).
3. Interface change requests (exact signatures) and new dependency requests.
4. What is approximate, blocked, or not found. Never fill gaps with guesses: say "not determined".
