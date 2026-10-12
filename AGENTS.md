# Haste: rules for everyone working in this repo (humans and agents)

Haste is a Minecraft: Java Edition 26.3 client in Rust that will later run Fabric mods through a
compatibility layer. Two feasibility spikes proved the architecture
(https://github.com/bednarjosef/haste-spike, `REPORT.md` and `REPORT-2.md`). Phase 1 is a fast
vanilla client with the mod architecture designed in and no mod support yet.

## Architecture (fixed; from the spikes)

1. **Three tiers of game state.**
   - Tier 1: stateful game objects (world, chunks, entities, the player) live in **Rust**
     (`haste-world`, `haste-entity`, `haste-physics`).
   - Tier 2: Java-facing **proxies** of those objects (`haste-jvm`). Cheap primitives are mirrored
     into Java fields once per landmark; writes and anything with logic are natives into Rust.
   - Tier 3: everything without hot game state (value types like `Identifier`/`BlockPos`/
     `Component`, chat, Screens and widgets, codecs, registry data) runs as **real vanilla Java**
     loaded at runtime from the player's own 26.3 jar.
2. **One canonical Java proxy per Rust object for its whole lifetime.** Mods will store state on it.
   Proxies are created and owned through a handle table; never hand out a second proxy for the same
   object.
3. **Fat interfaces are batched.** GUI drawing and vertex consumers are collected into a Rust-side
   command list once per frame (`haste_core::gui::GuiFrame`), never one JNI call per draw.
4. **Mixin's real behaviour is the oracle, not its docs.** Spike 1 found HEAD injection order is the
   opposite of what the source suggests. Phase 1 has no mixins, but `phase2/oracle-mod` (the real-
   Mixin oracle harness) stays in the repo for Phase 2 and must keep working.
5. **Legal.** Never commit Mojang jars, assets, decompiled source or anything generated from them
   (data-generator reports, oracle dumps, extracted textures). Read the player's installation at
   runtime; derived data lives in the gitignored `cache/` (`haste_core::paths`). Implement behaviour
   from observation and our own oracle data (recorded by `tools/recorder-mod` from the real game),
   never by translating decompiled code. Do not copy code from projects whose licence forbids it
   (Sodium: ideas only). Reused open-source code keeps its header and is listed in
   `THIRD_PARTY.md` with its licence (GPL-3.0-compatible only).
6. **Toolchain.** Pinned stable Rust (`rust-toolchain.toml`). No nightly features. No azalea crates.
   Before writing a large subsystem from scratch, spend ten minutes checking for an existing crate
   or GPL-compatible project (Pomme, Pumpkin, Fabric Loader for Phase 2); write the result in
   `NOTES.md`.
7. **Data comes from the game, not from hand-written tables.** Block states, registries and packet
   ids come from Minecraft's data generator (`haste setup` runs the server jar with `--reports`
   into `cache/datagen/26.3/reports`). Collision shapes, entity dimensions and the like come from
   the recorder mod's oracle dumps in `cache/oracle/26.3`.

## How the team works

- The **lead** owns `haste-core`, `docs/INTERFACES.md`, `crates/haste-client/src/main.rs`,
  `TASKS.md`, `NOTES.md`, and merges. Workstreams own their crates (header of each `lib.rs`).
- **A workstream never edits another workstream's crate.** If you need an interface change, write
  the request (what, why, the exact signature) in your report to the lead and continue on
  something else. The lead edits `haste-core`/`docs/INTERFACES.md` and tells everyone.
- Work on your branch in your worktree (`haste-wt/<ws>`). Commit often with clear messages. Never
  push. Never rebase or merge other branches yourself.
- **Done means tests pass.** Every change comes with the tests named in your brief; `cargo test -p
  <your crate>` and `cargo clippy -p <your crate>` must be clean. Fast tests (milliseconds) live
  next to the code; anything needing the cache (jars, reports) is `#[ignore]`d unless the cache
  exists, and says so.
- Write `NOTES.md`-worthy decisions and dead ends in your report; the lead copies them in.
- Performance matters everywhere: no per-block allocations in meshing or physics, no per-packet
  allocations beyond the packet itself, no locks held across I/O. Measure before claiming.
- Absolute paths in reports. Cite `file:line` for anything you want the lead to look at.

## Branches and merging

- `main` is the integration branch and is never committed to directly. All development happens on
  branches (`ws<N>-<name>` for workstreams, `<topic>` for anything else) with small, clear commits.
- A branch reaches `main` only through a pull request on the GitHub remote. The lead agent pushes
  the branch, opens the PR (`gh pr create`), reviews it (tests and clippy clean, no lead-owned
  files touched without agreement, no derived Mojang data, attributions in `THIRD_PARTY.md` for
  any reused code) and merges it (`gh pr merge --merge`, a merge commit so each workstream's
  history stays visible). There is no human in the loop: the lead agent does all of this itself.
- Worker agents never push; the lead pushes their branches when opening the PR.
- Branches pick up `main` with `git merge main` (no rebasing of shared branches).
- The public remote receives `main` and reviewed branches only; `cache/`, worktrees, and local
  settings never leave the machine.

## Nothing personal in the tree

- No absolute home paths (`/home/<user>`, `/Users/<user>`) in committed files: use `$HASTE_ROOT`,
  `$HASTE_CACHE`, `$HASTE_WT`, `../haste-spike` or `~` (see `scripts/env.sh`).
- No email addresses in committed files except `*@users.noreply.github.com`; commits are authored
  with the GitHub noreply identity (the repo config sets it).
- CI fails on either. Secrets, tokens and Mojang-derived data never enter the tree (`cache/`).

## Layout

```
crates/haste-core       shared types + cross-crate traits (lead)
crates/haste-data       registries, block states, oracle dumps from cache (world-and-data)
crates/haste-protocol   26.3 protocol, auth, packets (protocol)
crates/haste-world      chunks, palettes, light, block updates (world-and-data)
crates/haste-assets     jar/assets reading: textures, models, fonts (renderer)
crates/haste-render     wgpu renderer, meshing, GUI batch drawing (renderer)
crates/haste-entity     entity tracking, interpolation, models (entities)
crates/haste-physics    vanilla-exact movement/collision (physics)
crates/haste-jvm        embedded JVM, tier 2/3, proxy generator, GUI pipeline (jvm-host)
crates/haste-setup      Mojang downloads, datagen, config, singleplayer server, launcher (setup)
crates/haste-testkit    oracle formats, replay, screenshot diff, bench harness (oracles)
crates/haste-client     the `haste` binary; one module per workstream (lead + all)
tools/recorder-mod      Fabric mod for the real game that records golden data (oracles)
phase2/                 spike code for Phase 2: Mixin bridge, surface scanner, oracle mod
cache/                  gitignored: jars, assets, reports, oracle dumps, JDK
```
