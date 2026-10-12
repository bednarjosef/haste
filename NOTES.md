# Notes: decisions and dead ends

Newest at the top. One entry per decision, with the why; link the brief or report it came from.

- **2026-10-12 Runtime data, not build-time codegen.** Block states, registries and packet ids are
  loaded at startup from the data-generator reports in the cache (`haste-data`, `PacketIds::load`).
  Build-time codegen from gitignored data would make every build depend on the cache and every
  worktree need it before `cargo build` works; runtime loading costs ~100 ms and keeps the legal
  rule simple (nothing derived from Mojang data is ever in the tree).
- **2026-10-12 Collision shapes come from the oracle, with a fallback.** `blocks.json` has no shapes.
  The recorder mod dumps `block_shapes.json` from the real game (server side, headless); until then
  `haste-data` uses a documented heuristic fallback so M2/M3 work can proceed.
- **2026-10-12 No azalea.** Its 26.3 branch needs shifting nightly features and failed to build on
  three nightlies in the spikes. Pomme depends on azalea's 26.2 branch for protocol/crypto/auth, so
  Pomme's protocol layer is not reusable as-is either (evaluation in progress).
- **2026-10-12 Worktrees share a prebuilt target.** `cargo build` of the workspace deps (wgpu, winit,
  tokio) is copied into each worktree's `target/` so eight workers do not compile them eight times.
