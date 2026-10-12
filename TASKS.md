# Task board

Status: `claimed` (brief sent) / `in progress` / `review` (branch ready, lead reviewing) / `done` (merged).
One line per task; details live in `docs/briefs/<ws>.md` and in the branch.

| ID | Workstream | Task | Milestone | Status |
|---|---|---|---|---|
| L-0 | lead | Repo scaffold, haste-core, interfaces, boards | - | done |
| L-1 | lead | Evaluate Pomme and Pumpkin for reuse (workers pomme-eval, pumpkin-eval) | - | done |
| WS2-1 | WS2 | haste-data: GameData from reports, BlockShapes (oracle or fallback) | M1 | in progress |
| WS2-2 | WS2 | haste-world: paletted containers, chunk decode, light, dirty tracking, snapshots | M1/M2 | claimed |
| WS4-1 | WS4 | haste-entity: tracking + interpolation | M1 | done |
| WS4-2 | WS4 | entity models as data + animation | M4 | done |
| WS5-1 | WS5 (Opus) | physics: collision sweep, tested | M3 | done |
| WS5-2 | WS5 (Opus) | physics: movement tick from public docs | M3 | done (awaiting traces) |
| WS5-3 | WS5 (Opus) | trace replay harness | M3 | done; replay runs when traces exist |
| WS6-1 | WS6 (Opus) | JVM host + tier-3 allow-listed loader | M5 | done |
| WS6-2 | WS6 (Opus) | proxies + GuiPipeline + Pause/Chat/Inventory screens (real vanilla classes, 1 JNI call/frame, p50 34 µs) | M5 | done (headless; needs the renderer to draw GuiFrame) |
| WS7-1 | WS7 | ensure_installed: Mojang downloads, JDK, datagen | M1 | done |
| WS7-2 | WS7 | config, LocalServer, vanilla launcher, MSA auth | M6 | in progress (checkpoint only this session) |
| WS8-1 | WS8 | server-side oracle dumps (block shapes, entity dims, block props) | M2/M3 | done |
| WS8-2 | WS8 | testkit formats + harness + packet proxy | M1 | done |
| WS8-3 | WS8 | client recorder + RECORDING.md (compiles; never run: needs the user's display) | M3 | done (code); recording pending user |
| WS8-4 | WS8 | CI + TESTING.md | - | done |
| WS1-1 | WS1 (Opus) | protocol codec + state machine (vendored Pumpkin transport) | M1 | done |
| WS1-2 | WS1 (Opus) | play packets (all 144 + 69) | M1 | done |
| WS1-3 | WS1 (Opus) | M1 headless join (60 s on local server, 473 chunks, 0 unknown) | M1 | done; stub world until haste-world lands |
| WS3-1 | WS3 | haste-assets: jar reading, models, atlas, font (Pomme pieces reused) | M2 | done |
| WS3-2 | WS3 | haste-render: meshing, world rendering | M2 | in progress |
| WS3-3 | WS3 | entities, HUD, GUI batch drawing | M4/M5 | pending |
