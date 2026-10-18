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
   runtime; derived data lives in the gitignored `cache/` (`haste_core::paths`). **Do not guess
   vanilla behaviour: look it up.** The 26.3 client decompiled by `scripts/decompile.sh` lives in
   `cache/source/26.3/client` (unobfuscated Mojang names); read it for exact constants, orders,
   formulas and the sequence of steps, and use the real jar (`javap`) and the recorder mod's dumps
   from the running game (`tools/recorder-mod`) to check. Then write Haste's own Rust: never paste
   Mojang code into the tree or translate a method line by line, and never quote it in comments,
   docs or commit messages (naming the vanilla class and method, e.g. `LocalPlayer.rideTick`, is
   fine and encouraged). Every agent and person on the project may read it; it stays only in
   `cache/`. Do not copy code from projects whose licence forbids it (Sodium: ideas only).
   GPL-compatible projects are fair game to reuse, Pumpkin (`cache/pumpkin`, GPL-3.0) first:
   reused code keeps its header and is listed in `THIRD_PARTY.md` with its licence.
6. **Toolchain.** Pinned stable Rust (`rust-toolchain.toml`). No nightly features. No azalea crates.
   Before writing a large subsystem from scratch, spend ten minutes checking for an existing crate
   or GPL-compatible project (Pomme, Pumpkin, Fabric Loader for Phase 2); write the result in
   `NOTES.md`.
7. **Data comes from the game, not from hand-written tables.** Block states, registries and packet
   ids come from Minecraft's data generator (`haste setup` runs the server jar with `--reports`
   into `cache/datagen/26.3/reports`). Collision shapes, entity dimensions and the like come from
   the recorder mod's oracle dumps in `cache/oracle/26.3`.

## Faithful wherever the player can feel it (read this whatever your task is)

Anything the player sees, hears or feels, and anything the server can observe, is **ported from
vanilla's code** (the decompiled 26.3 client in `cache/source/26.3/client`, rule 5): the same
constants, the same math, the same order of steps per tick, the same timings. That includes
movement and collision, sneaking and its camera smoothing, input timing (click delays, attack and
use cooldowns, block-breaking delays, key repeat), what can be attacked or used, the first-person
hand and item animations (swing, equip, bob), the camera (FOV changes, view bobbing, hurt and death
tilt), HUD and screen behaviour, sound timing and volumes, and every packet the client sends.
No guessed values, no "close enough", no wiki numbers when the source is there. Each PR that
touches such behaviour names the vanilla class and method it follows.

Everything the player cannot notice and the server cannot see (data structures, threading,
batching, caching, meshing, GPU work, decoding) is ours to design for speed (see "Performance is
the product"), but any constant it carries (light curves, colours, distances, sizes) still comes
from vanilla.

## Built for mods (read this whatever your task is)

Phase 1 is a vanilla client, but everything in it will later host Fabric mods through the
compatibility layer (Phase 2: Mixin handlers at Rust landmarks with Rust-owned state, vanilla
objects as generated proxies; see the architecture above). Write Phase 1 code so that layer can
be added without rewriting it:

- **Keep vanilla's seams.** Split behaviour where vanilla splits it (per-tick steps, render
  passes per renderer, one handler per packet, screens, block entity and entity renderers), so
  every place a mod injects into in vanilla has a matching function in Haste where a landmark can
  go. Do not fold several vanilla steps into one opaque loop.
- **Name things after vanilla's concepts** (Screen, EntityRenderer, BlockEntity, registry ids),
  so the proxy generator and Mixin targets map onto them.
- **Everything registry-driven.** Mods add blocks, items, entities, particles, sounds, screens:
  take them from registries and game data (rule 7), never from closed hard-coded lists, and
  handle ids you have never seen (draw a placeholder, play nothing, log once) instead of
  panicking.
- **Stable identity for game objects.** One Rust object per world/entity/block entity for its
  whole lifetime, addressable by a stable handle (rule 2): mods keep references and state on them.
- **Tier 3 stays real vanilla Java** (UI, codecs, value types), so mod screens and mod-added
  GUI code keep working.
- If a design choice might block mods later, say so in your report.

## Structured like vanilla (read this whatever your task is)

Minecraft's own code is well structured: one class per concept, small focused files. Haste aims
for the same, both for readability and because the Phase 2 mod layer maps Mixin targets onto
these seams.

- **One module per vanilla concept.** Each block entity renderer, entity renderer, particle
  type, screen bridge and group of packet handlers lives in its own file, named after the
  vanilla class it follows (`blockentity/chest.rs` for `ChestRenderer`, and so on). Shared
  plumbing goes into its own module, not into the file that first needed it.
- **Small files.** Source files stay within 600 lines and test files within 1000; tests live
  in their own files (`tests.rs` or `tests/`), never inline in a big source file. CI enforces
  this (`scripts/check-file-sizes.sh`): the files that were already larger are listed in
  `scripts/large-files.txt` and may shrink but never grow. When a change would push a file over
  its limit, put the new code into a new module named after the vanilla concept it follows
  instead, and move the code you touch out of a listed file when you can. After a split, run
  `scripts/check-file-sizes.sh --update` to lower the recorded sizes.
- **Plan the module before writing the code.** A new feature starts as its own file (or a
  directory of files) named after the vanilla class it ports, with shared plumbing in its own
  module; never append a new feature to whichever file happened to be open.
- **Split along responsibilities, never by size alone.** A split is a pure move: no behaviour
  change in the same commit, so the move can be reviewed as a move.

## Performance is the product

Haste exists to be faster than vanilla and than vanilla with Sodium; client performance comes
first, and for anything on the frame path (renderer, meshing, entities, particles, GUI drawing)
it is the top priority.

- **Measure every rendering change**, before and after: the renderer's per-frame timing
  breakdown (feed, entities, cull, encode, submit, GPU time per pass) in the window, and the
  offscreen benchmark at render distance 12 and 32 on a normal world. Put the numbers (average and
  worst frame) in your report and the PR. A change that makes frames slower is not done until it
  wins the time back or the lead accepts the trade.
- **Rules on the frame path:** no heap allocation per frame, per entity or per section (reuse
  buffers); no draw call per object (batch into one draw list per pass, instanced or merged);
  never block the render thread on the game thread, I/O or a GPU readback; anything that can run
  off the render thread does (meshing, decoding, uploads prepared ahead).
- **Elsewhere:** no per-block allocations in meshing or physics, no per-packet allocations beyond
  the packet itself, no locks held across I/O.
- Measure before claiming; numbers beat adjectives.

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
- Faithfulness, performance and mod-readiness rules are in the sections above; they apply to every task.
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
