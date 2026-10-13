# vanilla-reference: the real client as our oracle, automatically

Worker: Opus. Worktree `$HASTE_WT/vanilla-reference`, branch `vanilla-reference`, test server
port **25620**. Read `docs/briefs/COMMON.md` first, then `docs/RECORDING.md`,
`tools/recorder-mod/NOTES.md`, `crates/haste-testkit/FORMATS.md`.

You own `tools/recorder-mod`, `crates/haste-testkit`, `docs/RECORDING.md`, and new scripts under
`scripts/` (vanilla-*.sh, compare-*.sh).

## Goal
Haste must look and behave exactly like vanilla where it matters: the world, HUD, menus,
inventories, physics. The judge is the real vanilla 26.3 client. Make it run **unattended**
(the user allows a window on their screen, Wayland session, `DISPLAY=:0` / `WAYLAND_DISPLAY`
set) with the recorder mod's scripted input, against our test server, and produce reference
data the other workstreams compare against with one command. No human in the loop.

## Tasks, in order (commit after each)
1. **Launch vanilla unattended**: Fabric Loader + the recorder mod + Fabric API on the client jar
   in `cache/`, offline username, joining `127.0.0.1:<port>` directly (quick play
   `--quickPlayMultiplayer`), fixed window size (e.g. 1280x720), fixed settings written to
   `options.txt` before launch (render distance, GUI scale, FOV, fancy/fast, smooth lighting,
   biome blend, vsync off, no tutorial/onboarding/narrator popups, no realms news). Script:
   `scripts/vanilla-client.sh <server> <scenario>`; it exits by itself. `haste-setup`'s launcher
   may help (`crates/haste-setup/src/launch.rs`); you may use it but do not edit haste-setup
   (setup-cli owns it): copy what you need into the script. Report what blocks offline join if
   anything does, and how you solved it.
2. **Screenshot scenarios**: the recorder takes screenshots at scripted camera positions and
   states: terrain at fixed positions/angles/times on a fixed-seed normal world (put the seed in
   the script so both clients see the same world), the HUD (hotbar with items, health, hunger,
   xp), the inventory screen, the pause menu, chat open with lines, a chest screen, F3 off.
   Hide the hand/crosshair variants where needed. Output: `cache/oracle/26.3/screenshots/<name>.png`
   + `<name>.json` (camera pos, yaw, pitch, fov, time, window size, GUI scale) in the testkit
   format.
3. **The same shots from Haste**: `scripts/compare-vanilla.sh <scenario>` runs `haste
   --screenshot` with the same camera/time/size for each terrain shot (extend the haste
   `--screenshot` flags in `crates/haste-client` only through a request to the lead if you need
   more than `--yaw/--pitch/--width/--height`; position comes from teleporting the bot with a
   server command), then `haste_testkit::compare_screenshot` and writes a side-by-side +
   difference PNG and a percentage per shot to `target/vanilla-compare/`. GUI scenarios are
   compared when the HUD/screens land in Haste (jvm-host); prepare them now.
4. **Physics traces**: the recorder's scripted movement (walk, sprint-jump, sneak edge, stairs,
   fall, swim, fly) recorded per tick into `cache/oracle/26.3/traces/*.json`; then
   `cargo test -p haste-physics --test replay -- --ignored` and report the first divergence per
   trace (physics owner fixes them; you report exact numbers).

## Done
One command per kind of check, runnable by any workstream; a report with the first comparison
numbers (terrain shots: percent differing pixels; traces: first divergent tick and error).
