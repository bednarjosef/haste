# Task board

Status: `claimed` (brief sent) / `in progress` / `review` (branch ready, lead reviewing) / `done` (merged).
One line per task; details live in `docs/briefs/<ws>.md` and in the branch.

| ID | Workstream | Task | Milestone | Status |
|---|---|---|---|---|
| lead-0 | lead | Repo scaffold, haste-core, interfaces, boards | - | done |
| lead-1 | lead | Evaluate Pomme and Pumpkin for reuse (workers pomme-eval, pumpkin-eval) | - | done |
| world-and-data-1 | world-and-data | haste-data: GameData from reports, BlockShapes (oracle or fallback) | join milestone | done (PRs #1, #7) |
| world-and-data-2 | world-and-data | haste-world: paletted containers, chunk decode, light, dirty tracking, snapshots | join/first frame milestones | done (PR #5); unverified against a real chunk capture |
| entities-1 | entities | haste-entity: tracking + interpolation | join milestone | done |
| entities-2 | entities | entity models as data + animation | alive milestone | done |
| physics-1 | physics (Opus) | physics: collision sweep, tested | walk milestone | done |
| physics-2 | physics (Opus) | physics: movement tick from public docs | walk milestone | done (awaiting traces) |
| physics-3 | physics (Opus) | trace replay harness | walk milestone | done; replay runs when traces exist |
| jvm-host-1 | jvm-host (Opus) | JVM host + tier-3 allow-listed loader | vanilla screens milestone | done |
| jvm-host-2 | jvm-host (Opus) | proxies + GuiPipeline + Pause/Chat/Inventory screens (real vanilla classes, 1 JNI call/frame, p50 34 µs) | vanilla screens milestone | done (headless; needs the renderer to draw GuiFrame) |
| setup-1 | setup | ensure_installed: Mojang downloads, JDK, datagen | join milestone | done |
| setup-2 | setup | config, LocalServer, vanilla launcher, MSA auth | singleplayer and numbers milestone | done (PR #6); `haste setup`/`launch` subcommands pending (lead) |
| oracles-1 | oracles | server-side oracle dumps (block shapes, entity dims, block props) | first frame/walk milestones | done |
| oracles-2 | oracles | testkit formats + harness + packet proxy | join milestone | done |
| oracles-3 | oracles | client recorder + RECORDING.md (compiles; never run: needs the user's display) | walk milestone | done (code); recording pending user |
| oracles-4 | oracles | CI + TESTING.md | - | done |
| protocol-1 | protocol (Opus) | protocol codec + state machine (vendored Pumpkin transport) | join milestone | done |
| protocol-2 | protocol (Opus) | play packets (all 144 + 69) | join milestone | done |
| protocol-3 | protocol (Opus) | join milestone headless join (60 s on local server, 473 chunks, 0 unknown) | join milestone | done; stub world until haste-world lands |
| renderer-1 | renderer | haste-assets: jar reading, models, atlas, font (Pomme pieces reused) | first frame milestone | done |
| renderer-2 | renderer | haste-render: meshing, world rendering (211 FPS offscreen rd12 flat) | first frame milestone | done (PR #8); client wiring pending (lead) |
| renderer-3 | renderer | entities, HUD, GUI batch drawing | alive/vanilla screens milestones | not started |
