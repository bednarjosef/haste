# Recorder scenarios

Plain text, one step per line, `#` comments. `scripts/vanilla-client.sh <server> <scenario>`
passes the file to the recorder (`-Dhaste.scenario`); it starts 20 ticks after the player is in
the world with no screen open, and the game closes itself after the last step, on a failed
step, on a disconnect, or when the join never happens (120 s).

| step | meaning |
|------|---------|
| `wait <ticks>` | wait this many client ticks |
| `wait_world` | wait until the player's chunk is loaded and every visible section is compiled, unchanged for 40 ticks (60 s cap, then continue with a warning) |
| `cmd <command>` | send a server command (the player is op on the reference server) |
| `chat <text>` | send a chat message |
| `look <yaw> <pitch>` | set the camera client-side (prefer `cmd tp @s ~ ~ ~ <yaw> <pitch>`) |
| `slot <0-8>` | select a hotbar slot |
| `hud on\|off` | the game's F1 toggle (off also hides the hand) |
| `screen none\|inventory\|pause\|chat [text]` | open a screen; `screen use <x> <y> <z>` right-clicks a block (chests) |
| `screenshot <name>` | capture the last frame to `cache/oracle/26.3/screenshots/<name>.png` + `.json` |
| `record <name>` | run the scripted physics sequences, traces to `cache/oracle/26.3/traces/` |
| `dump_models` | write every baked model layer's geometry to `cache/oracle/26.3/entity_models.json` |
| `log <text>` | write a line to the client log |
| `quit` | close the game now |

The log lines `haste-recorder: scenario done` / `haste-recorder: scenario failed: <reason>`
tell the script how it went. Yaw: 0 = south, 180 = north; pitch: positive looks down.
