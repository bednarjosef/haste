# Notes: decisions and dead ends

Newest at the top. One entry per decision, with the why; link the brief or report it came from.

- **2026-10-12 Physics sources.** haste-physics is built from minecraft.wiki plus mcpk.wiki (the
  parkour community's observed formulas; only numbers and prose were used, not the decompiled
  snippets that site also quotes). Every constant is tabled in `crates/haste-physics/NOTES.md`
  with its source; water/lava gravity, swim up/down and flight vertical speed are marked
  UNVERIFIED until the recorder traces exist. Pumpkin's entity movement was not reused (it is
  organised after yarn-named vanilla methods). Full tick ~1.2 µs, collide ~450 ns.
- **2026-10-12 Team mix.** Eight GLM workers on two API keys caused HTTP 429 storms and one gate
  stall (restart fixed it). Protocol (WS1), physics (WS5) and the JVM host (WS6), the precision- and
  judgement-heavy streams, moved to Claude Opus subagents (medium effort) in the same worktrees;
  GLM keeps world/data, entities, renderer assets, setup and oracles (five concurrent).
- **2026-10-12 Renderer: wgpu, with Pomme's azalea-free pieces.** Pomme's renderer is raw Vulkan
  (pyronyx + shaderc GLSL), ~41k lines, and its net/physics/light are either azalea-bound or
  self-declared vanilla ports. We keep wgpu and reuse Pomme's asset index, block model baking,
  atlas stitching, greedy mesher (MIT upstream), occlusion graph and the mesher/buffer design
  (GPL-3.0, attributed). Pomme's Tauri launcher is not reused in Phase 1 (CLI flow), but its
  azalea-free MSA OAuth flow (`pomme-launcher/.../auth.rs`) is the reference for WS7. Evaluation
  report: worker `glm-pomme-eval` (2026-10-12).
- **2026-10-12 Protocol: vendor Pumpkin's transport, own packets.** Pumpkin (`cache/pumpkin`, GPL-3.0,
  stable Rust, protocol 777) has a tested framing/compression/encryption layer and NBT, but only
  ~27% of clientbound packets decode (it is a server) and its `pumpkin-data` crate is 1.5M generated
  lines that downloads the server jar at build time. We vendor ~7k lines (transport, wire
  primitives, NBT) into `haste-protocol` and write packet structs on `haste_core` types with ids
  from `packets.json` at runtime. Evaluation report: worker `glm-pumpkin-eval` (2026-10-12).
- **2026-10-12 Vulkan on the dev box.** No system ICD (`vulkan-radeon` not installed, no sudo). A
  project-local RADV built by the fastcraft project works via `VK_DRIVER_FILES` (see `scripts/env.sh`).

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
