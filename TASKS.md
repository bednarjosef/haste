# Task board

Status: `claimed` (brief sent) / `in progress` / `review` (branch ready, lead reviewing) / `done` (merged).
One line per task; details live in `docs/briefs/<workstream>.md` and in the branch.
Session 1 tasks (all done and merged) are in git history; their briefs are in `docs/briefs/archive/`.

| Workstream | Worker | Task | Milestone | Status |
|---|---|---|---|---|
| client-app | Opus | real World/EntityStore in the client, game + render threads, window, screenshot mode | first frame | in progress |
| player | Opus | LocalPlayer: vanilla movement packets, physics verified against the vanilla server; then interaction | walk | in progress |
| renderer | Opus | GUI drawing, entities, sky/fog/weather, overlays, performance pass | alive / vanilla screens | in progress |
| jvm-host | Opus | registries into Java, vanilla `Gui` as the HUD, playable screens, `ui.rs` | alive / vanilla screens | in progress |
| sound | GLM | haste-audio: sounds.json, positional playback, music, block sounds oracle | alive | in progress |
| fluids-and-tint | GLM | water/lava meshing, biome colors and blending | first frame | in progress |
| world-and-data | GLM | chunk decoding checked against region files, block entities, section/light updates | first frame | in progress |
| setup-cli | GLM | `haste setup/login/singleplayer/vanilla`, player certificates, smoke script | singleplayer and numbers | in progress |
| protocol-compat | Opus (next free slot) | audit every vanilla client obligation: chat signing and acknowledgements, configuration re-entry, resource packs, cookies/transfer, hashed stacks | join | queued |
| automated-oracles | Opus (next free slot) | record physics traces and screenshots from the real client automatically with the recorder mod's scripted input | walk | queued |
