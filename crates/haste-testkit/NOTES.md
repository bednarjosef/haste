# haste-testkit notes

## Task 2 state (formats + harness)

- `FORMATS.md` documents every file under `cache/oracle/26.3/`: the three server dumps, traces,
  screenshots (+camera JSON), packets (binary format with header), all versioned.
- `oracles.rs`: loaders for the three server dumps. `BlockShapes` implements
  `haste_core::view::CollisionShapes` (id-indexed lookup, O(1)); also `find(block, props)` for
  partial-property lookups used by the validation tests. `EntityDimensions` has per-type dims and
  the 18 player poses. `BlockProperties` has friction/speed/jump with vanilla defaults.
- `traces.rs`: `Trace::load` + `replay_trace(trace, tick_fn, tolerance) -> TraceReport`.
  Semantics: ticks record pos at the START of the tick, so applying tick i's input is compared
  against tick i+1's recorded pos (the last tick's outcome is not in the file). Errors are
  per-tick max-axis position deltas; first divergence tick, max/mean error, on_ground
  mismatches, and a pass flag.
- `screenshots.rs`: `compare_screenshot(a, b, tolerance) -> DiffResult` — per-pixel RGBA channel
  difference, diff fraction, mean/max error, and a red-highlighted diff image. Size mismatch is
  an error, not a resize.
- `bench.rs`: JSON-lines `BenchSample` (frame_ms, rss_kib from /proc/self/status VmRSS,
  sections, draw calls), `Bench::load`/`write_jsonl`, stats (mean/p percentiles/fps/max RSS),
  `BenchWriter` for streaming capture.
- `report.rs`: `Markdown` table writer + `bench_table`/`trace_table`/`diff_table` renderers.
- `packets.rs`: recording file parser/writer (magic HSTK\0, protocol version, timestamp, count,
  then length-prefixed decompressed frames; varint helpers round trip negative ids — a naive
  `i32 >>= 7` loop on a negative value never terminates, encode via `u32`).
- `proxy.rs` + `src/bin/haste-packet-proxy.rs`: TCP proxy for OFFLINE servers. Frame
  parse/re-frame with zlib compression support (`set_compression` threshold semantics incl.
  the data-length-0 "below threshold" case), state machine
  handshake->login(compression=3, success=2)->config(finish=2)->play, recording every
  clientbound play frame decompressed. Serverbound is piped raw. One client per run.
- Tests: 23, all fast (no `cache/` needed — the oracle loaders gracefully eprintln+skip when
  the dump files are absent; with the cache present they run against the real dumps).
  `cargo test -p haste-testkit` + `cargo clippy -p haste-testkit -- -D warnings` clean.

## Not done / approximate

- The proxy's state machine uses observed clientbound packet ids for login/config transitions
  (compression=3, login success=2, config finish=2). If 26.3's configuration protocol differs,
  the session still forwards everything (recording would just start late). WS1's
  `packets.json`-driven ids are the authoritative fix; a real end-to-end recording against the
  local server is still to be exercised (needs a client to connect through it).
- `compare_screenshot` does not tolerate subpixel shifts; fine for fixed-camera comparisons.
- Bench RSS is whatever the recorder writes (client mod reads /proc/self/status; Haste's own
  binary would read it directly).

## Dependency note

`bytes` and `flate2` were added to this crate's manifest — both are already workspace
dependencies (root Cargo.toml), no new external crates. Reported to the lead regardless.
