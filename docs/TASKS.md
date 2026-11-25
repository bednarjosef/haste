# Task board

Status: `claimed` (brief sent) / `in progress` / `review` (branch ready, lead reviewing) / `done` (merged).
One line per task; details live in `docs/briefs/<workstream>.md` and in the branch.
Session 1 tasks (all done and merged) are in git history; their briefs are in `docs/briefs/archive/`.

| Workstream | Worker | Task | Milestone | Status |
|---|---|---|---|---|
| client-app | Opus | real World/EntityStore in the client, game + render threads, window, `--screenshot` | first frame | done (PRs #2, #8) |
| player | Opus | LocalPlayer movement verified on the vanilla server; interaction (break/place/use/attack/pick/hotbar) | walk | done (PRs #5, #12) |
| renderer | Opus | GUI, entities, sky, overlays, hand, GL fallback, batched draws, windowed frame time | alive / vanilla screens | done (PRs #7, #9, #11, #15); PR #15 introduced floating blocks (HANDOFF.md) |
| jvm-host | Opus | vanilla registries and `Hud` in tier 3, chat/death/chest/kick screens, `ui.rs` | alive / vanilla screens | done headless (PR #16); not wired into the window |
| protocol-compat | Opus | BitSet fix, decode audit, configuration both ways | join | tasks 0-2 done (PRs #10, #17); tasks 3-5 open |
| vanilla-reference | Opus | real vanilla client unattended, terrain comparison | all | task 1 done (PR #18); tasks 2-4 open |
| fluids-and-tint | Opus | biome colors and blending, water/lava | first frame | done (PR #19); texture animation and exact lighting open |
| sound | - | haste-audio (docs/briefs/sound.md) | alive | not started |
| world-and-data | - | block entities (one unreviewed commit on the branch), region-file oracle, section updates | first frame | not started |
| setup-cli | - | `haste setup/login/singleplayer/vanilla`, player certificates (WIP launch.rs on the branch) | singleplayer and numbers | not started |
