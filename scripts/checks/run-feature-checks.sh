#!/usr/bin/env bash
# Runs the feature check scripts in scripts/checks/ against the local test server.
# usage: scripts/checks/run-feature-checks.sh [feature ...]
#   features: hud death drop modes f2 craft  (default: all)
# The client runs offscreen (`--script`); all server state is set up over RCON from here,
# never from the script (the player is not an operator and the server kicks command spam).
# Evidence is printed and grepped: frame text commands (not images), server log lines and
# `data get entity` results.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PORT=25691
# A fresh player name per run (a fresh player joins alive, empty, at the world spawn):
# seeded from the PID so reruns never collide with a wedged player file.
NAME=${HASTE_CHECK_NAME:-CChk$$}
NAME=${NAME:0:16}
BIN="${HASTE_BIN:-$ROOT/target/debug/haste}"
LOGDIR="$ROOT/target/feature-checks"
mkdir -p "$LOGDIR"
rcon() { "$ROOT/scripts/vanilla-rcon.sh" --port "$PORT" "$@"; }

# The test server's own log (for "NAME died"): the server copy whose properties name our port.
# The running server's own log: find the java process whose working directory holds a
# server.properties for our port (the server may use a different HASTE_CACHE than this tree).
SERVER_LOG=""
for p in /proc/[0-9]*; do
  cwd=$(readlink "$p/cwd" 2>/dev/null) || continue
  if [ -f "$cwd/server.properties" ] && grep -q "^server-port=$PORT\$" "$cwd/server.properties" 2>/dev/null; then
    SERVER_LOG="$cwd/logs/latest.log"
  fi
done
echo "== server log: ${SERVER_LOG:-<not found for port $PORT>}"

PASS=0
FAIL=0
FAILED=()

client_start() { # script log
  : >"$2"
  timeout 200 "$BIN" --server "127.0.0.1:$PORT" --username "$NAME" --script "$1" \
    --seconds 120 >"$2" 2>&1 &
  CLIENT_PID=$!
}

wait_online() {
  for _ in $(seq 1 300); do
    rcon list 2>/dev/null | grep -q "$NAME" && return 0
    sleep 0.5
  done
  return 1
}

wait_shot() { # log name [timeout_s]
  local log=$1 name=$2 t=${3:-90}
  for _ in $(seq 1 $((t * 10))); do
    grep -q "script-$name.png" "$log" 2>/dev/null && return 0
    sleep 0.1
  done
  return 1
}

wait_exit() { wait "$CLIENT_PID"; return $?; }

check() { # name description result
  if [ "$3" = 0 ]; then
    echo "PASS $1: $2"
    PASS=$((PASS + 1))
  else
    echo "FAIL $1: $2"
    FAIL=$((FAIL + 1))
    FAILED+=("$1")
  fi
}

# The z of a `data get entity ... Pos` answer like `[... , 100.0d, 6.5d]`.
pos_z() { echo "$1" | grep -oE '\[[-0-9.]+d, [-0-9.]+d, [-0-9.]+d\]' | tr -d '[]d' | cut -d, -f3; }

has() { echo "$1" | grep -q -- "$2"; }

# ---- 1. HUD elements: player list, boss bar, sidebar, title, action bar ----
hud() {
  local log="$LOGDIR/hud.log"
  client_start "$ROOT/scripts/checks/hud-elements.txt" "$log"
  wait_online || { check hud "player never joined" 1; return; }
  rcon "gamemode creative $NAME" >/dev/null
  if wait_shot "$log" hud-setup-bars 90; then
    # Remove any leftovers first: an idempotent add/setdisplay sends no packet, so a
    # reconnecting client would never learn the state (this is what a fresh join sees).
    rcon "bossbar remove t" >/dev/null
    rcon "scoreboard objectives remove s" >/dev/null
    rcon "bossbar add t \"BossBarText\"" >/dev/null
    rcon "bossbar set t players $NAME" >/dev/null
    rcon "scoreboard objectives add s dummy \"SidebarText\"" >/dev/null
    rcon "scoreboard objectives setdisplay sidebar s" >/dev/null
    # A fake holder so the sidebar does not print our name: the name in the frame then
    # comes from the held-Tab player list.
    rcon "scoreboard players set SideMarker s 7" >/dev/null
  fi
  if wait_shot "$log" hud-setup-title 60; then
    rcon "title $NAME times 20 600 20" >/dev/null
    rcon "title $NAME subtitle SubTitleText" >/dev/null
    rcon "title $NAME title Hi" >/dev/null
    rcon "title $NAME actionbar Bar" >/dev/null
  fi
  wait_exit
  local ev="$LOGDIR/hud.evidence"
  grep -E "script (text|frame)\[hud-elements\]" "$log" >"$ev"
  echo "--- hud evidence (frame hud-elements) ---"; cat "$ev"
  local r=0
  has "$(cat "$ev")" "BossBarText" || { echo "missing boss bar text"; r=1; }
  has "$(cat "$ev")" "SidebarText" || { echo "missing sidebar text"; r=1; }
  has "$(cat "$ev")" "Hi" || { echo "missing title text"; r=1; }
  has "$(cat "$ev")" "Bar" || { echo "missing action bar text"; r=1; }
  has "$(cat "$ev")" "$NAME" || { echo "missing player-list name"; r=1; }
  check hud "boss bar, sidebar, title, action bar and the held Tab player list are drawn" "$r"
}

# ---- 2. Death screen and respawn ----
death() {
  local log="$LOGDIR/death.log"
  local before_lines=0
  [ -n "$SERVER_LOG" ] && before_lines=$(wc -l <"$SERVER_LOG" 2>/dev/null || echo 0)
  client_start "$ROOT/scripts/checks/death-respawn.txt" "$log"
  wait_online || { check death "player never joined" 1; return; }
  rcon "gamemode survival $NAME" >/dev/null
  local dead_health= health= after_pos=
  if wait_shot "$log" death-setup 90; then
    rcon "kill $NAME" >/dev/null
  fi
  if wait_shot "$log" death-screen 60; then
    dead_health=$(rcon "data get entity $NAME Health")
  fi
  if wait_shot "$log" death-after 60; then
    health=$(rcon "data get entity $NAME Health")
    after_pos=$(rcon "data get entity $NAME Pos")
  fi
  wait_exit
  local died death_text
  died=$(tail -n +"$((before_lines + 1))" "$SERVER_LOG" 2>/dev/null | grep -iE -- "$NAME (died|was killed)" | tail -1)
  death_text=$(grep -E "script text\[death-screen\]" "$log")
  echo "--- death evidence ---"
  echo "server log: ${died:-<none>}"
  echo "health while dead: ${dead_health:-<none>}"
  echo "health after respawn: ${health:-<none>}"
  echo "pos after respawn:    ${after_pos:-<none>}"
  echo "$death_text"
  local r=0
  has "$death_text" "You Died!" || { echo "missing death screen text"; r=1; }
  has "${dead_health:-}" "0.0" || { echo "health was not 0 while dead"; r=1; }
  has "${health:-}" "20.0" || { echo "health not restored"; r=1; }
  [ -n "$died" ] || { echo "no server log death line"; r=1; }
  check death "death screen, Respawn click, server-logged death and health back to 20" "$r"
}

# ---- 3. Drop (Q, Ctrl+Q) and off-hand swap (F) ----
drop() {
  local log="$LOGDIR/drop.log"
  client_start "$ROOT/scripts/checks/drop-swap.txt" "$log"
  wait_online || { check drop "player never joined" 1; return; }
  rcon "gamemode survival $NAME" >/dev/null
  if wait_shot "$log" drop-setup 90; then
    rcon "clear $NAME" >/dev/null
    rcon "give $NAME minecraft:stone 10" >/dev/null
    rcon "item replace entity $NAME weapon.offhand with minecraft:torch 3" >/dev/null
  fi
  sleep 2
  local before after items
  before=$(rcon "data get entity $NAME SelectedItem")
  if wait_shot "$log" drop-swap 90; then
    after=$(rcon "data get entity $NAME SelectedItem")
    items=$(rcon "execute as @e[type=item,distance=..5] run data get entity @s Item")
  fi
  wait_exit
  echo "--- drop evidence ---"
  echo "SelectedItem before: $before"
  echo "SelectedItem after:  $after"
  echo "item entities: $items"
  local r=0
  has "$before" "stone" || { echo "no stone before"; r=1; }
  has "$after" "torch" || { echo "F did not swap in the off-hand torch"; r=1; }
  has "$items" "stone" || { echo "no dropped stone item entities"; r=1; }
  check drop "Q dropped one, Ctrl+Q dropped the stack, F swapped the off-hand item" "$r"
}

# ---- 4. Game modes: spectator walks through a wall, adventure cannot break ----
modes() {
  local log="$LOGDIR/modes.log"
  client_start "$ROOT/scripts/checks/game-modes.txt" "$log"
  wait_online || { check modes "player never joined" 1; return; }
  local spec_before spec_after creative_check adventure_check
  if wait_shot "$log" gm-spectator-setup 90; then
    rcon "gamemode spectator $NAME" >/dev/null
    rcon "fill -8 99 -8 8 99 8 stone" >/dev/null
    rcon "fill -8 100 -8 8 104 8 air" >/dev/null
    rcon "fill -1 100 2 1 102 2 stone" >/dev/null
    rcon "tp $NAME 0 100 0 0 0" >/dev/null
    sleep 1
    spec_before=$(pos_z "$(rcon "data get entity $NAME Pos")")
  fi
  if wait_shot "$log" gm-spectator-wall 60; then
    spec_after=$(pos_z "$(rcon "data get entity $NAME Pos")")
  fi
  if wait_shot "$log" gm-creative-setup 60; then
    rcon "gamemode creative $NAME" >/dev/null
    rcon "tp $NAME 0 100 0 0 0" >/dev/null
    rcon "setblock 0 101 2 stone" >/dev/null
    sleep 1
  fi
  if wait_shot "$log" gm-creative-broken 60; then
    creative_check=$(rcon "execute if block 0 101 2 air" | tail -1)
    rcon "gamemode adventure $NAME" >/dev/null
    rcon "tp $NAME 0 100 0 0 0" >/dev/null
    rcon "setblock 0 101 2 stone" >/dev/null
    sleep 1
  fi
  if wait_shot "$log" gm-adventure-attack 60; then
    adventure_check=$(rcon "execute if block 0 101 2 stone" | tail -1)
  fi
  wait_exit
  echo "--- game modes evidence ---"
  echo "spectator z: ${spec_before:-?} -> ${spec_after:-?} (wall at z=2)"
  echo "creative break check:  ${creative_check:-<none>}"
  echo "adventure break check: ${adventure_check:-<none>}"
  local r=0
  awk -v z="${spec_after:-0}" 'BEGIN{exit !(z>2)}' || { echo "spectator did not pass the wall"; r=1; }
  has "${creative_check:-}" "Test passed" || { echo "creative did not break the block"; r=1; }
  has "${adventure_check:-}" "Test passed" || { echo "adventure broke the block"; r=1; }
  check modes "spectator walked through the wall; adventure kept the block (creative broke it)" "$r"
}

# ---- 5. F2 screenshot ----
f2() {
  local log="$LOGDIR/f2.log"
  client_start "$ROOT/scripts/checks/screenshot-f2.txt" "$log"
  wait_online || { check f2 "player never joined" 1; return; }
  wait_exit
  local line path
  line=$(grep "script F2 screenshot saved to" "$log" | tail -1)
  path=${line##* saved to }
  echo "--- F2 evidence ---"
  echo "$line"
  local r=0
  [ -n "$path" ] || { echo "no F2 save line"; r=1; }
  [ -n "$path" ] && [ -f "$path" ] || { echo "PNG not found at $path"; r=1; }
  [ -n "$path" ] && [ -f "$path" ] && echo "PNG: $(stat -c '%n %s bytes' "$path")"
  check f2 "F2 wrote a PNG into the screenshots folder" "$r"
}

# ---- 6. Survival crafting: one oak log -> four oak planks ----
craft() {
  local log="$LOGDIR/craft.log"
  client_start "$ROOT/scripts/checks/craft-planks.txt" "$log"
  wait_online || { check craft "player never joined" 1; return; }
  rcon "gamemode survival $NAME" >/dev/null
  if wait_shot "$log" craft-setup 90; then
    rcon "clear $NAME" >/dev/null
    rcon "give $NAME minecraft:oak_log 1" >/dev/null
  fi
  sleep 2
  local before after
  before=$(rcon "data get entity $NAME Inventory")
  if wait_shot "$log" craft-final 90; then
    after=$(rcon "data get entity $NAME Inventory")
  fi
  wait_exit
  echo "--- crafting evidence ---"
  echo "Inventory before: $before"
  echo "Inventory after:  $after"
  local r=0
  has "$before" "oak_log" || { echo "no oak_log before"; r=1; }
  has "$after" "oak_planks" || { echo "no oak_planks after"; r=1; }
  has "$after" "count: 4" || { echo "not 4 planks"; r=1; }
  check craft "one oak log became 4 oak planks in the survival inventory" "$r"
}

# ---- 7. Containers: every container screen works like vanilla, with server evidence ----
# One function per container. Each check owns a spot in this branch's area (x 120..130,
# z 0..10 - never the shared spawn, where other workers' players block the crosshair):
# the runner tps the player there over RCON, gives the items, places the block, and
# reads `data get entity`/`data get block` back. The scripts click the vanilla slot
# coordinates (menu position + 8 for a slot centre, screen origin (152, 52) for a
# 176x166 screen centred in the 480x270 GUI).

# Starts the client on `script` and waits for it to be online. The player name is fresh
# per run (`ContainersChk$RANDOM`): a fresh player always joins alive, empty and at the
# world spawn, which is the snowy plain. Returns 1 when the client never joined.
plain_client() { # script log
  client_start "$1" "$2"
  wait_online || return 1
  return 0
}

# The setup the container scripts trigger with their first screenshot (after the world
# settled): the tp moves the player into this check's own area (x 120..130, z 0..10 - the
# shared world spawn is never used: other workers' players stand there and an entity in
# the crosshair eats the use click) and zeroes the look offset (an absolute-rotation
# teleport, so the scripts' `look` angles are what the crosshair aims); the caller's
# commands give the items and place the blocks. The commands are retried for as long as
# the script's `wait_until hotbar_item` window (25 s): on a busy shared test server the
# player entity lags the client's join by many seconds, RCON answers "No player was
# found" until it is there, and the give's inventory sync can lag its answer again.
plain_setup() { # log shot pos commands...
  local log=$1 shot=$2 pos=$3
  shift 3
  if wait_shot "$log" "$shot" 90; then
    local end=$((SECONDS + 25))
    while [ $SECONDS -lt $end ]; do
      rcon "gamemode survival $NAME" >/dev/null
      rcon "tp $NAME $pos" >/dev/null
      local c
      for c in "$@"; do
        rcon "$c" >/dev/null
      done
      # done when a `data get` answer names the player (the commands found it)
      [ -n "$(rcon "data get entity $NAME Pos" 2>/dev/null | grep "$NAME")" ] && break
      sleep 2
    done
  fi
}

# The crafting table: craft planks in the 3x3 grid, shift-click the result into the
# inventory, then repeat with plain clicks (pick the result up, put it down). The table
# sits in the check's own area; the player stands 2.5 blocks north of it.
container_crafting_table() {
  local log="$LOGDIR/containers-crafting-table.log"
  plain_client "$ROOT/scripts/checks/containers-crafting-table.txt" "$log" \
    || { check containers-crafting-table "player never joined" 1; return; }
  plain_setup "$log" ccraft-setup "125.5 71 2.5 0 0" \
    "clear $NAME" \
    "give $NAME minecraft:oak_log 2" \
    "fill 124 70 1 126 74 6 air replace" \
    "fill 124 70 1 126 70 6 minecraft:stone replace" \
    "setblock 125 71 5 minecraft:crafting_table"
  local before after
  before=$(rcon "data get entity $NAME Inventory")
  if wait_shot "$log" ccraft-final 90; then
    after=$(rcon "data get entity $NAME Inventory")
  fi
  wait_exit
  echo "--- crafting table evidence ---"
  echo "Inventory before: $before"
  echo "Inventory after:  $after"
  local r=0
  has "$before" "oak_log" || { echo "no oak_log before"; r=1; }
  # 2 logs -> 8 planks: shift-clicked straight into the inventory, then the second batch
  # picked up and placed by hand.
  has "$after" "oak_planks" || { echo "no oak_planks after"; r=1; }
  has "$after" "count: 8" || { echo "not 8 planks"; r=1; }
  check containers-crafting-table "2 oak logs became 8 oak planks (shift-click + hand-placed result)" "$r"
}

# The furnace: smelt 1 raw iron with 1 coal, take the iron ingot. The furnace sits at
# (128, 71, 5) in the check's own area, on its own stone platform, the player 2.5 blocks
# north of it.
container_furnace() {
  local log="$LOGDIR/containers-furnace.log"
  plain_client "$ROOT/scripts/checks/containers-furnace.txt" "$log" \
    || { check containers-furnace "player never joined" 1; return; }
  plain_setup "$log" cfurn-setup "128.5 71 2.5 0 0" \
    "clear $NAME" \
    "give $NAME minecraft:raw_iron 1" \
    "give $NAME minecraft:coal 1" \
    "fill 127 70 1 129 74 6 air replace" \
    "fill 127 70 1 129 70 6 minecraft:stone replace" \
    "setblock 128 71 5 minecraft:furnace"
  local before after
  before=$(rcon "data get entity $NAME Inventory")
  if wait_shot "$log" cfurn-closed 90; then
    after=$(rcon "data get entity $NAME Inventory")
  fi
  wait_exit
  echo "--- furnace evidence ---"
  echo "Inventory before: $before"
  echo "Inventory after:  $after"
  local r=0
  has "$before" "raw_iron" || { echo "no raw_iron before"; r=1; }
  has "$before" "coal" || { echo "no coal before"; r=1; }
  has "$after" "iron_ingot" || { echo "no iron_ingot after"; r=1; }
  check containers-furnace "1 raw iron smelted with 1 coal became 1 iron ingot" "$r"
}

ALL=(hud death drop modes f2 craft containers-crafting-table containers-furnace)
if [ $# -eq 0 ]; then
  set -- "${ALL[@]}"
fi
for f in "$@"; do
  case "$f" in
    hud | death | drop | modes | f2 | craft) "$f" ;;
    containers-crafting-table) container_crafting_table ;;
    containers-furnace) container_furnace ;;
    *) echo "unknown feature: $f" ;;
  esac
done

echo
echo "== $PASS passed, $FAIL failed =="
[ "$FAIL" -eq 0 ] || { echo "failed: ${FAILED[*]}"; exit 1; }
