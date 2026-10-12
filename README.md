# Haste

A Minecraft: Java Edition 26.3 client written in Rust, designed to run Fabric mods through a
compatibility layer later (Phase 2). Phase 1, this repo's current state, is a fast vanilla client
with the mod architecture designed in. Licensed GPL-3.0-or-later.

The architecture was proven in two feasibility spikes:
https://github.com/bednarjosef/haste-spike (`REPORT.md`: Mixin handlers at Rust landmarks with
Rust-owned state; `REPORT-2.md`: vanilla objects as generated proxies, 30-mod surface scan,
pure-proxy vs mirrored-field benchmarks). Read `AGENTS.md` for the rules and
`docs/INTERFACES.md` for the crate contracts.

## Build

Pinned stable Rust (`rust-toolchain.toml`), no system-wide installs. First run:

```bash
cargo run --release --bin haste -- setup        # downloads 26.3 from Mojang into the cache, runs the data generator
```

Everything downloaded or derived from Mojang's files lives in `cache/` (gitignored) or
`$HASTE_CACHE`. Nothing from it is ever committed.

## Milestones

| | Demo | Status |
|---|---|---|
| Join milestone | `haste --headless --server 127.0.0.1:25565` joins a local 26.3 server and an online-mode server, tracks world and entities | done headless (local server); online mode pending Microsoft tokens |
| First frame milestone | the world renders around the player | renderer and world crates done (211 FPS offscreen); client wiring pending |
| Walk milestone | movement/collision match vanilla traces, no rubber-banding | physics done against documented numbers; traces not recorded yet |
| Alive milestone | entities render and move; HUD | entity store and models done; drawing pending |
| Vanilla screens milestone | chat, pause menu, inventory as real vanilla Java screens through the batched GUI pipeline | done headless (one JNI call per frame); drawing pending |
| Singleplayer and numbers milestone | embedded vanilla server; benchmarks vs vanilla and vanilla + Sodium | setup, local server and launcher done; benchmarks pending |

See `TASKS.md` (board) and `NOTES.md` (decisions). `PHASE1-REPORT.md` arrives with singleplayer and numbers milestone.
