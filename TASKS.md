# Task board

Status: `claimed` (brief sent) / `in progress` / `review` (branch ready, lead reviewing) / `done` (merged).
One line per task; details live in `docs/briefs/<workstream>.md` and in the branch.
Session 1 tasks (all done and merged) are in git history; their briefs are in `docs/briefs/archive/`.

| Workstream | Worker | Task | Milestone | Status |
|---|---|---|---|---|
| client-app | Opus | real World/EntityStore in the client, game + render threads, window, `--screenshot` | first frame | done (PRs #2, #8) |
| player | Opus | LocalPlayer movement verified on the vanilla server; interaction (break/place/use/attack/pick/hotbar) | walk | done (PRs #5, #12) |
| renderer | Opus | GUI, entities, sky, overlays, hand, GL fallback, batched draws; now: windowed frame time | alive / vanilla screens | in progress (PRs #7, #9, #11 merged) |
| jvm-host | Opus | registries into Java, vanilla `Gui` HUD, playable screens, `ui.rs` | alive / vanilla screens | in progress |
| protocol-compat | Opus | every vanilla client obligation; BitSet fix done (PR #10) | join | in progress |
| vanilla-reference | Opus | real vanilla client unattended: reference screenshots, GUI shots, physics traces, compare scripts | walk / all | in progress |
| fluids-and-tint | Opus (next free slot) | water/lava meshing, biome colors and blending (brief: docs/briefs/fluids-and-tint.md) | first frame | queued (GLM produced nothing) |
| sound | Opus (queued) | haste-audio (brief: docs/briefs/sound.md) | alive | queued |
| world-and-data | Opus (queued) | block entities (one commit on the branch, unreviewed), region-file oracle, section updates | first frame | queued |
| setup-cli | Opus (queued) | `haste setup/login/singleplayer/vanilla`, player certificates (WIP launch.rs on the branch, unreviewed) | singleplayer and numbers | queued |
