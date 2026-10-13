# Testing

Three tiers, ordered by how long they take and what they need. Every workstream plugs its
tests into the lowest tier it can; the ladder from fast to slow is deliberate — `cargo test`
on CI (no `cache/`, no display) must stay green in well under a minute.

## Tier 1 — fast unit tests (ms: pure functions)

- `cargo test -p <crate>` — plain `#[test]`s on pure functions: parsers, math, formats,
  protocol round trips, block-shape lookups. No I/O beyond `std::env::temp_dir()`.
- This is the only tier CI runs on every push/PR (`.github/workflows/ci.yml`): fmt + clippy
  `-D warnings` + `cargo test --workspace`, pinned stable toolchain (`rust-toolchain.toml`),
  no `cache/` needed. Oracle loaders in `haste-testkit` gracefully skip their
  cache-dependent assertions when `cache/oracle/26.3/` is absent, so the same tests also
  run against the real dumps on a dev machine that has them.

## Tier 2 — headless replays against oracle data (s: on a dev machine with `cache/`)

- `cargo test -p haste-testkit` with `cache/oracle/26.3/` present: loaders accept the real
  dumps (35723 block states, 161 entity types, 1286 blocks).
- `haste_testkit::traces::replay_trace` — tick function under test against recorded vanilla
  physics (`traces/*.json`); physics's physics should be the first consumer: per-tick max-axis
  position error, first divergence tick, on_ground mismatches. This is the tier where
  "behaves like vanilla" becomes checkable without a display.
- `python3 tools/recorder-mod/validate-traces.py` — sanity checks on freshly recorded
  traces (walk moved, jump left ground, sneak slower than walk, ...) before replaying them.
- Requires the client-recorded traces; see [RECORDING.md](RECORDING.md) for how they are
  produced (they need the real vanilla client and a display, so a human runs that part).

## Tier 3 — screenshot diffs and benchmarks (before merge, human-reviewed)

- `haste_testkit::screenshots::compare_screenshot` — Haste render vs `cache/oracle/26.3/
  screenshots/*.png` (fixed camera, HUD hidden; camera parameters recorded in the sibling
  JSON). Diff fraction + mean/max per-channel error + a red-highlighted diff image.
  Needs renderer's renderer to produce a framebuffer to compare; the harness side is done.
- `haste_testkit::bench` + `cargo run -p haste-testkit --example bench-report -- <dir>` —
  frame times/RSS comparison against vanilla's recorded `/haste bench` runs.
- Packet recordings (`cache/oracle/26.3/packets/*.bin`, captured via
  `cargo run -p haste-testkit --bin haste-packet-proxy`) are protocol/world-and-data's replay fixtures at
  this tier.
- `REVIEW_SERVER=host:port scripts/review.sh <branch>` — one screenshot of a live normal world
  (fixed yaw 180 / pitch 25 camera) into the branch worktree's `target/review/`; catches
  renderer regressions the fixed-camera oracle diffs miss (e.g. missing chunks).

## How each workstream plugs in

- **Pure logic (parsers, math, IDs): tier 1.** Plain `#[test]` in the crate, no fixtures.
- **Behaviour that vanilla defines (physics, movement, dimensions): tier 2.** Load the
  oracle dumps through `haste-testkit` (`oracles.rs` implements `haste_core`'s
  `CollisionShapes` etc.), replay traces through `replay_trace`, assert tolerances.
- **Rendering / end-to-end: tier 3.** Screenshot diff against the recorded PNGs with a
  fixed camera; bench comparisons for performance PRs.

File formats for everything under `cache/oracle/26.3/` are documented in
`crates/haste-testkit/FORMATS.md`; nothing in Haste may be written from decompiled code —
the oracle data is the ground truth.
