#!/usr/bin/env bash
# Runs the feature check scripts in scripts/checks/ against the local test server.
# usage: scripts/checks/run-feature-checks.sh [feature ...]
#   features: hud death drop modes f2 craft pause creative-hotbars language particles recipe-book sign book commandblock containers-crafting-table containers-furnace containers-anvil  (default: all)
# The client runs offscreen (`--script`); all server state is set up over RCON from here,
# never from the script (the player is not an operator and the server kicks command spam).
# Evidence is printed and grepped: frame text commands (not images), server log lines and
# `data get entity` results.
#
# The test server is shared with other workers, so every run is scoped to itself:
#  - its own unique player name (feature prefix + pid): two clients with one username get
#    the server's chunk delivery tangled — the later join steals the player entity and the
#    victim receives no chunks at all, so `wait_until settled` never passes;
#  - world setup and cleanup in a per-run area (a pid/clock-derived centre), never at the
#    shared spawn, and item/block queries scoped with `execute at` + `distance=..` around
#    the run's own area instead of global `@e` selectors;
#  - `set -u`-safe variables (every local initialised) and waits on evidence (poll for the
#    expected state) instead of fixed sleeps.
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

client_start() { # script log [extra args...]
  : >"$2"
  local script=$1 log=$2
  shift 2
  timeout 200 "$BIN" --server "127.0.0.1:$PORT" --username "$NAME" --script "$script" \
    "$@" --seconds 120 >"$log" 2>&1 &
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

# Polls an RCON command until its output contains `needle`; prints the last output either
# way and returns whether the needle was seen. Evidence instead of a fixed sleep: the state
# is observed, not assumed to hold after a delay.
# usage: wait_rcon <command> <needle> [timeout_s]
wait_rcon() {
  local cmd=$1 needle=$2 t=${3:-30} out= i
  for ((i = 0; i < t * 2; i++)); do
    out=$(rcon "$cmd" 2>/dev/null)
    if has "$out" "$needle"; then
      printf '%s\n' "$out"
      return 0
    fi
    sleep 0.5
  done
  printf '%s\n' "$out"
  return 1
}

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

# A per-run world area, away from the shared spawn and from other runs': a centre derived
# from the pid and the clock, so concurrent runs of a check do not share their platform.
# Sets CX/CZ.
area() {
  CX=$(( ($$ % 8) * 160 - 560 ))
  CZ=$(( ($(date +%s) % 8) * 160 - 560 ))
}

# Item entities within `dist` of a player. Never a global @e: other workers drop items on
# this server too, and a selector with no anchor counts every one of them. The player has to
# be online: `execute at` needs an entity to anchor on.
items_around_player() { # player dist
  rcon "execute at $1 as @e[type=item,distance=..$2] run data get entity @s Item" 2>/dev/null
}

# ---- 1. HUD elements: player list, boss bar, sidebar, title, action bar ----
hud() {
  local log="$LOGDIR/hud.log"
  NAME="hud_$$"
  # This run's own boss bar and scoreboard objective: the default names ("t"/"s") would
  # race with another worker's hud check on the shared server.
  local bar="t$$" obj="s$$" marker="Side$$"
  client_start "$ROOT/scripts/checks/hud-elements.txt" "$log"
  wait_online || { check hud "player never joined" 1; return; }
  rcon "gamemode creative $NAME" >/dev/null
  if wait_shot "$log" hud-setup-bars 90; then
    # Remove any leftovers first: an idempotent add/setdisplay sends no packet, so a
    # reconnecting client would never learn the state (this is what a fresh join sees).
    rcon "bossbar remove $bar" >/dev/null
    rcon "scoreboard objectives remove $obj" >/dev/null
    rcon "bossbar add $bar \"BossBarText\"" >/dev/null
    rcon "bossbar set $bar players $NAME" >/dev/null
    rcon "scoreboard objectives add $obj dummy \"SidebarText\"" >/dev/null
    rcon "scoreboard objectives setdisplay sidebar $obj" >/dev/null
    # A fake holder so the sidebar does not print our name: the name in the frame then
    # comes from the held-Tab player list.
    rcon "scoreboard players set $marker $obj 7" >/dev/null
  fi
  if wait_shot "$log" hud-setup-title 60; then
    rcon "title $NAME times 20 600 20" >/dev/null
    rcon "title $NAME subtitle SubTitleText" >/dev/null
    rcon "title $NAME title Hi" >/dev/null
    rcon "title $NAME actionbar Bar" >/dev/null
  fi
  wait_exit
  # Leave no state behind for the next run (or a concurrent one).
  rcon "bossbar remove $bar" >/dev/null
  rcon "scoreboard objectives remove $obj" >/dev/null
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
  NAME="dea_$$"
  local before_lines=0
  [ -n "$SERVER_LOG" ] && before_lines=$(wc -l <"$SERVER_LOG" 2>/dev/null || echo 0)
  client_start "$ROOT/scripts/checks/death-respawn.txt" "$log"
  wait_online || { check death "player never joined" 1; return; }
  rcon "gamemode survival $NAME" >/dev/null
  local dead_health= health= after_pos= died= death_text=
  if wait_shot "$log" death-setup 90; then
    rcon "kill $NAME" >/dev/null
  fi
  if wait_shot "$log" death-screen 60; then
    dead_health=$(rcon "data get entity $NAME Health" 2>/dev/null)
  fi
  if wait_shot "$log" death-after 60; then
    health=$(rcon "data get entity $NAME Health" 2>/dev/null)
    after_pos=$(rcon "data get entity $NAME Pos" 2>/dev/null)
  fi
  wait_exit
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
  NAME="dro_$$"
  area
  client_start "$ROOT/scripts/checks/drop-swap.txt" "$log"
  wait_online || { check drop "player never joined" 1; return; }
  rcon "gamemode survival $NAME" >/dev/null
  local before= after= items=
  if wait_shot "$log" drop-setup 90; then
    # Move the player to our own area and build the platform under them there, not before
    # the client joins and not somewhere the player is not: a platform filled into a chunk
    # nobody keeps loaded is unloaded again with nothing saved, and the player that is
    # teleported onto it later finds a hole. `execute at` also keeps every query and the
    # cleanup around the player, so other players' items are never counted.
    rcon "tp $NAME $CX 100 $CZ 0 0" >/dev/null
    rcon "execute at $NAME run fill ~-4 ~-1 ~-4 ~4 ~-1 ~4 stone" >/dev/null
    # Evidence: the player really stands on the platform (they are not still falling).
    wait_rcon "execute at $NAME if block ~ ~-1 ~ stone" "Test passed" 30 >/dev/null
    rcon "clear $NAME" >/dev/null
    rcon "give $NAME minecraft:stone 10" >/dev/null
    rcon "item replace entity $NAME weapon.offhand with minecraft:torch 3" >/dev/null
    wait_rcon "data get entity $NAME Inventory" "stone" 15 >/dev/null
    before=$(rcon "data get entity $NAME SelectedItem" 2>/dev/null)
  fi
  if wait_shot "$log" drop-swap 90; then
    after=$(rcon "data get entity $NAME SelectedItem" 2>/dev/null)
    items=$(items_around_player "$NAME" 10)
    # Our dropped items and our platform, while the player is still here to anchor
    # `execute at` (after they leave, the selectors find no entity at all).
    rcon "execute at $NAME as @e[type=item,distance=..10] run kill @s" >/dev/null
    rcon "execute at $NAME run fill ~-4 ~-1 ~-4 ~4 ~-1 ~4 air" >/dev/null
  fi
  wait_exit
  echo "--- drop evidence ---"
  echo "SelectedItem before: ${before:-<none>}"
  echo "SelectedItem after:  ${after:-<none>}"
  echo "item entities: ${items:-<none>}"
  local r=0
  has "${before:-}" "stone" || { echo "no stone before"; r=1; }
  has "${after:-}" "torch" || { echo "F did not swap in the off-hand torch"; r=1; }
  has "${items:-}" "stone" || { echo "no dropped stone item entities"; r=1; }
  check drop "Q dropped one, Ctrl+Q dropped the stack, F swapped the off-hand item" "$r"
}

# ---- 4. Game modes: spectator walks through a wall, adventure cannot break ----
modes() {
  local log="$LOGDIR/modes.log"
  NAME="mod_$$"
  area
  client_start "$ROOT/scripts/checks/game-modes.txt" "$log"
  wait_online || { check modes "player never joined" 1; return; }
  # Where the player stands after `tp $NAME $CX 100 $CZ`: the z of the .5-shifted feet.
  local zneedle
  if [ "$CZ" -lt 0 ]; then zneedle="$((CZ + 1)).5d"; else zneedle="${CZ}.5d"; fi
  local spec_before="" spec_after="" creative_check="" adventure_check=""
  if wait_shot "$log" gm-spectator-setup 90; then
    rcon "gamemode spectator $NAME" >/dev/null
    rcon "fill $((CX - 8)) 99 $((CZ - 8)) $((CX + 8)) 99 $((CZ + 8)) stone" >/dev/null
    rcon "fill $((CX - 8)) 100 $((CZ - 8)) $((CX + 8)) 104 $((CZ + 8)) air" >/dev/null
    rcon "fill $((CX - 1)) 100 $((CZ + 2)) $((CX + 1)) 102 $((CZ + 2)) stone" >/dev/null
    rcon "tp $NAME $CX 100 $CZ 0 0" >/dev/null
    if wait_rcon "data get entity $NAME Pos" "$zneedle" 10; then
      spec_before=$(pos_z "$(rcon "data get entity $NAME Pos")")
    fi
  fi
  if wait_shot "$log" gm-spectator-wall 60; then
    spec_after=$(pos_z "$(rcon "data get entity $NAME Pos")")
  fi
  if wait_shot "$log" gm-creative-setup 60; then
    rcon "gamemode creative $NAME" >/dev/null
    rcon "tp $NAME $CX 100 $CZ 0 0" >/dev/null
    rcon "setblock $CX 101 $((CZ + 2)) stone" >/dev/null
    wait_rcon "execute if block $CX 101 $((CZ + 2)) stone" "Test passed" 10 >/dev/null
  fi
  if wait_shot "$log" gm-creative-broken 60; then
    creative_check=$(rcon "execute if block $CX 101 $((CZ + 2)) air" | tail -1)
    rcon "gamemode adventure $NAME" >/dev/null
    rcon "tp $NAME $CX 100 $CZ 0 0" >/dev/null
    rcon "setblock $CX 101 $((CZ + 2)) stone" >/dev/null
    wait_rcon "execute if block $CX 101 $((CZ + 2)) stone" "Test passed" 10 >/dev/null
  fi
  if wait_shot "$log" gm-adventure-attack 60; then
    adventure_check=$(rcon "execute if block $CX 101 $((CZ + 2)) stone" | tail -1)
  fi
  wait_exit
  # Our platform, wall and column, not the shared spawn's.
  rcon "fill $((CX - 8)) 99 $((CZ - 8)) $((CX + 8)) 104 $((CZ + 8)) air" >/dev/null
  echo "--- game modes evidence ---"
  echo "spectator z: ${spec_before:-?} -> ${spec_after:-?} (wall at z=$((CZ + 2)))"
  echo "creative break check:  ${creative_check:-<none>}"
  echo "adventure break check: ${adventure_check:-<none>}"
  local r=0
  awk -v z="${spec_after:-0}" -v w="$((CZ + 2))" 'BEGIN{exit !(z>w)}' || { echo "spectator did not pass the wall"; r=1; }
  has "${creative_check:-}" "Test passed" || { echo "creative did not break the block"; r=1; }
  has "${adventure_check:-}" "Test passed" || { echo "adventure broke the block"; r=1; }
  check modes "spectator walked through the wall; adventure kept the block (creative broke it)" "$r"
}

# ---- 5. F2 screenshot ----
f2() {
  local log="$LOGDIR/f2.log"
  NAME="f2_$$"
  client_start "$ROOT/scripts/checks/screenshot-f2.txt" "$log"
  wait_online || { check f2 "player never joined" 1; return; }
  wait_exit
  local line= path=
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
  NAME="cra_$$"
  client_start "$ROOT/scripts/checks/craft-planks.txt" "$log"
  wait_online || { check craft "player never joined" 1; return; }
  rcon "gamemode survival $NAME" >/dev/null
  local before= after=
  if wait_shot "$log" craft-setup 90; then
    rcon "clear $NAME" >/dev/null
    rcon "give $NAME minecraft:oak_log 1" >/dev/null
    wait_rcon "data get entity $NAME Inventory" "oak_log" 15 >/dev/null
  fi
  sleep 2
  local before="" after=""
  before=$(rcon "data get entity $NAME Inventory" 2>/dev/null)
  if wait_shot "$log" craft-final 90; then
    after=$(rcon "data get entity $NAME Inventory" 2>/dev/null)
  fi
  wait_exit
  echo "--- crafting evidence ---"
  echo "Inventory before: $before"
  echo "Inventory after:  $after"
  local r=0
  has "${before:-}" "oak_log" || { echo "no oak_log before"; r=1; }
  has "${after:-}" "oak_planks" || { echo "no oak_planks after"; r=1; }
  has "${after:-}" "count: 4" || { echo "not 4 planks"; r=1; }
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
containers_crafting_table() {
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

# ---- 11. Block particles: the client's own destroy grid and mining cracks ----
particles() {
  local log="$LOGDIR/particles.log"
  # A name of our own: another agent's feature check may be running as $NAME on the
  # shared server, and a duplicate login kicks our client mid-script.
  local NAME=ParticlesCheck
  client_start "$ROOT/scripts/checks/block-particles.txt" "$log"
  wait_online || { check particles "player never joined" 1; return; }
  # The script waits 15 s after its setup screenshot; set the stage in that window:
  # survival, an empty hotbar, a stone platform under the player, an iron pickaxe.
  # (`execute as/at` selectors: plain name arguments race other agents' players on
  # the shared server; the selector forms resolve at run time.)
  while ! grep -q "script-setup" "$log" 2>/dev/null; do
    sleep 1
    grep -q "game thread stopping" "$log" 2>/dev/null && break
  done
  rcon "execute as $NAME run tp @s 0 100 0 0 0" >/dev/null
  rcon "execute as $NAME run gamemode survival @s" >/dev/null
  rcon "execute as $NAME run clear @s" >/dev/null
  rcon "execute at $NAME run fill ~-4 ~-1 ~-4 ~4 ~-1 ~4 stone" >/dev/null
  rcon "execute at $NAME run fill ~-3 ~ ~-3 ~3 ~4 ~3 air" >/dev/null
  rcon "execute as $NAME run give @s minecraft:iron_pickaxe 1" >/dev/null
  # Torches around the player for the ambient animateTick particles (placed after
  # the mining screenshots appear, so they do not get mined).
  if wait_shot "$log" block-broken 60; then
    rcon "execute at $NAME run setblock ~2 ~ ~2 torch" >/dev/null
    rcon "execute at $NAME run setblock ~-2 ~ ~-1 torch" >/dev/null
    rcon "execute at $NAME run setblock ~1 ~ ~-3 torch" >/dev/null
  fi
  wait_exit
  echo "--- block particles evidence ---"
  grep "particles drawn" "$log" | tail -4
  local cracks broken ambient r=0
  cracks=$(grep "script-mining-cracks" "$log" | grep -o '[0-9]* particles drawn' | grep -o '[0-9]*')
  broken=$(grep "script-block-broken" "$log" | grep -o '[0-9]* particles drawn' | grep -o '[0-9]*')
  ambient=$(grep "script-ambient-torches" "$log" | grep -o '[0-9]* particles drawn' | grep -o '[0-9]*')
  [ -n "$cracks" ] && [ "$cracks" -gt 0 ] || { echo "no crack particles while mining"; r=1; }
  [ -n "$broken" ] && [ "$broken" -gt 20 ] || { echo "no destroy grid on break"; r=1; }
  [ -n "$ambient" ] && [ "$ambient" -gt 0 ] || { echo "no ambient torch particles"; r=1; }
  check particles "mining drew crack particles ($cracks), the break the destroy grid ($broken), torches ambient particles ($ambient)" "$r"
}

# ---- 7. Every pause-menu screen ----
pause() {
  local log="$LOGDIR/pause.log"
  NAME="pau_$$"
  area
  client_start "$ROOT/scripts/checks/pause-all.txt" "$log"
  wait_online || { check pause "player never joined" 1; return; }
  if wait_shot "$log" advance-setup 90; then
    # The advancement tree only travels to players that have made progress: a player that
    # never earned one is sent the recipe root, which has no display and so shows no tab at
    # all (vanilla `PlayerAdvancements.flushDirty` sends only the roots it marked dirty).
    # Granting our own player the tree is the setup the screen's tabs need.
    rcon "advancement grant $NAME everything" >/dev/null
  fi
  wait_exit
  local ev="$LOGDIR/pause.evidence"
  grep -E "script text\[" "$log" >"$ev"
  echo "--- pause evidence ---"; cat "$ev"
  local r=0
  local t
  for t in advancements statistics report-bugs send-feedback friends player-reporting world-options; do
    grep -q "script frame\[$t\]" "$log" || { echo "missing screenshot $t"; r=1; }
  done
  # The advancements screen: a tab drawn, and the drawn nodes' icons. The server sends the
  # tree's roots from a HashSet, so which root is first (and so which tab is selected)
  # varies: any of vanilla's five root titles counts, and the icons are what prove the
  # tree arrived rather than an empty screen.
  local tab="" t
  for t in Minecraft Adventure Husbandry Nether "The End"; do
    if has "$(cat "$ev")" "$t"; then tab=$t; fi
  done
  [ -n "$tab" ] || { echo "no advancement tab drawn"; r=1; }
  grep -q "frame\[advancements\].*items=\[[^]]" "$log" || { echo "advancements screen drew no tree"; r=1; }
  echo "advancement tab drawn: ${tab:-<none>}"
  # the stats screen's rows: the values arrive between the click and the screenshot
  has "$(cat "$ev")" "Animals Bred" || { echo "statistics screen never showed its rows"; r=1; }
  has "$(cat "$ev")" "Difficulty" || { echo "missing World Options difficulty row"; r=1; }
  has "$(cat "$ev")" "Social Interactions" || { echo "missing player reporting screen"; r=1; }
  has "$(cat "$ev")" "open this link" || { echo "missing the report-bugs link confirmation"; r=1; }
  if grep -qE "haste proxy:|Java exception" "$log"; then
    echo "proxy stubs or Java exceptions:"; grep -E "haste proxy:|Java exception" "$log" | head -5
    r=1
  fi
  check pause "every pause-menu screen opens: advancements (with the server's tree), statistics, the icon buttons, world options" "$r"
}

# ---- 8. The creative inventory's Saved Hotbars tab ----
creative_hotbars() {
  local log="$LOGDIR/creative-hotbars.log"
  NAME="chb_$$"
  client_start "$ROOT/scripts/checks/creative-hotbars.txt" "$log"
  wait_online || { check creative-hotbars "player never joined" 1; return; }
  rcon "gamemode creative $NAME" >/dev/null
  wait_exit
  local ev="$LOGDIR/creative-hotbars.evidence"
  grep -E "script text\[" "$log" >"$ev"
  echo "--- creative hotbars evidence ---"; cat "$ev"
  local r=0
  has "$(cat "$ev")" "Saved Hotbars" || { echo "missing the Saved Hotbars tab"; r=1; }
  if grep -qE "haste proxy:|Java exception" "$log"; then
    echo "proxy stubs or Java exceptions:"; grep -E "haste proxy:|Java exception" "$log" | head -5
    r=1
  fi
  check creative-hotbars "the creative inventory's Saved Hotbars tab shows the saved hotbars" "$r"
}

# ---- 9. The recipe book: open it, search, click a recipe, take the result ----
recipe_book() {
  local log="$LOGDIR/recipe-book.log"
  client_start "$ROOT/scripts/checks/recipe-book.txt" "$log"
  wait_online || { check recipe-book "player never joined" 1; return; }
  # the script's wait_until settled needs the player at the plain the camera shoots: the spawn
  # point can be a wall (a suffocation death would kill the check)
  rcon "tp $NAME 9.5 71 6.5" >/dev/null
  rcon "gamemode survival $NAME" >/dev/null
  if wait_shot "$log" recipe-book-open 90; then
    rcon "clear $NAME" >/dev/null
    rcon "give $NAME minecraft:oak_log 1" >/dev/null
  fi
  sleep 2
  local before="" after=""
  before=$(rcon "data get entity $NAME Inventory")
  if wait_shot "$log" recipe-book-final 90; then
    after=$(rcon "data get entity $NAME Inventory")
  fi
  wait_exit
  echo "--- recipe book evidence ---"
  echo "Inventory before: $before"
  echo "Inventory after:  $after"
  local r=0
  has "$before" "oak_log" || { echo "no oak_log before"; r=1; }
  has "$after" "oak_planks" || { echo "no oak_planks after"; r=1; }
  has "$after" "count: 4" || { echo "not 4 planks"; r=1; }
  if grep -qE "haste proxy:|Java exception" "$log"; then
    echo "proxy stubs or Java exceptions:"; grep -E "haste proxy:|Java exception" "$log" | head -5
    r=1
  fi
  check recipe-book "the book opens and lists recipes; searching finds oak planks; clicking it crafts 4 planks from one log" "$r"
}

# The furnace: smelt 1 raw iron with 1 coal, take the iron ingot. The furnace sits at
# (128, 71, 5) in the check's own area, on its own stone platform, the player 2.5 blocks
# north of it.
containers_furnace() {
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

# The anvil: rename a wooden sword with one XP level. The anvil sits at (122, 71, 5) in
# the check's own area, on its own stone platform, the player 2.5 blocks north of it.
containers_anvil() {
  local log="$LOGDIR/containers-anvil.log"
  plain_client "$ROOT/scripts/checks/containers-anvil.txt" "$log" \
    || { check containers-anvil "player never joined" 1; return; }
  plain_setup "$log" canvil-setup "122.5 71 2.5 0 0" \
    "clear $NAME" \
    "give $NAME minecraft:wooden_sword 1" \
    "xp add $NAME 1 levels" \
    "fill 121 70 1 123 74 6 air replace" \
    "fill 121 70 1 123 70 6 minecraft:stone replace" \
    "setblock 122 71 5 minecraft:anvil"
  local before after
  before=$(rcon "data get entity $NAME Inventory")
  if wait_shot "$log" canvil-closed 90; then
    after=$(rcon "data get entity $NAME Inventory")
  fi
  wait_exit
  echo "--- anvil evidence ---"
  echo "Inventory before: $before"
  echo "Inventory after:  $after"
  local r=0
  has "$before" "wooden_sword" || { echo "no wooden_sword before"; r=1; }
  has "$after" "wooden_sword" || { echo "no renamed sword after"; r=1; }
  has "$after" "RenamedSword" || { echo "no custom name RenamedSword"; r=1; }
  check containers-anvil "a wooden sword was renamed to RenamedSword for 1 XP level" "$r"
}

# ---- 10. Languages other than English ----
language() {
  local log="$LOGDIR/language.log"
  client_start "$ROOT/scripts/checks/language.txt" "$log"
  wait_online || { check language "player never joined" 1; return; }
  wait_exit
  local ev="$LOGDIR/language.evidence"
  grep -E "script text\[" "$log" >"$ev"
  echo "--- language evidence (pause menu after switching to de_de) ---"
  grep -E "script text\[pause-de\]" "$log"
  local r=0
  has "$(grep -E 'script text\[pause-de\]' "$log")" "Zurück zum Spiel" \
    || { echo "pause menu not in German after switching"; r=1; }
  if grep -qE "haste proxy:|Java exception" "$log"; then
    echo "proxy stubs or Java exceptions:"; grep -E "haste proxy:|Java exception" "$log" | head -5
    r=1
  fi
  check language "Options > Language switched to de_de and the pause menu drew in German" "$r"
}


# ---- 12. Signs: place an oak sign, the edit screen opens, type two lines, Done ----
# The script is self-contained (the check player is an operator, so it clears, teleports to
# the camera and gives itself the sign through the chat); the runner only ops the player and
# reads the evidence. Looking down-forward from the camera (9.5 71 6.5), the placement use
# targets the ground block ahead: the sign lands at (9, 70, 7) with the typed lines.
sign() {
  local log="$LOGDIR/sign.log"
  client_start "$ROOT/scripts/checks/edit-sign.txt" "$log" --pos 9.5 71 6.5
  wait_online || { check sign "player never joined" 1; return; }
  rcon "op $NAME" >/dev/null
  wait_exit
  echo "--- sign evidence ---"
  local out
  out=$(rcon "data get block 9 70 7 front_text")
  echo "front_text: $out"
  local r=0
  has "$out" "LineOne" || { echo "missing line one"; r=1; }
  has "$out" "LineTwo" || { echo "missing line two"; r=1; }
  grep -E "haste proxy:|Java exception" "$log" && r=1
  check sign "placed an oak sign, typed two lines, Escape (Done) sent the sign update" "$r"
}

# ---- 13. Books: use a writable book, type, sign ----
# The script is self-contained (the check player is an operator); the runner ops the player
# and reads the evidence while the player is still online (the entity's data is only
# queryable while it exists: the final screenshot fires before the client leaves). Signing
# the book makes the server replace the held writable book with a written book.
book() {
  local log="$LOGDIR/book.log"
  client_start "$ROOT/scripts/checks/edit-book.txt" "$log"
  wait_online || { check book "player never joined" 1; return; }
  rcon "op $NAME" >/dev/null
  local held=""
  if wait_shot "$log" book-done 90; then
    held=$(rcon "data get entity $NAME SelectedItem")
  fi
  wait_exit
  echo "--- book evidence ---"
  echo "SelectedItem: ${held:-<not read>}"
  local r=0
  has "$held" "written_book" || { echo "no written book after signing"; r=1; }
  has "$held" "MyBook" || { echo "book title missing"; r=1; }
  grep -E "haste proxy:|Java exception" "$log" && r=1
  check book "used a writable book, typed, signed it into a written book" "$r"
}

# ---- 14. Command blocks: ops, place, use, type a command, Done ----
# The script is self-contained (the check player is an operator: creative mode, its own command
# block ahead of the camera, the use opens the editor); the runner ops the player and reads the
# evidence. The server sends block_entity_data when the player uses the command block, which
# activates the editor's buttons (vanilla ClientPacketListener.handleBlockEntityData).
commandblock() {
  local log="$LOGDIR/commandblock.log"
  client_start "$ROOT/scripts/checks/edit-command-block.txt" "$log" --pos 9.5 71 6.5
  wait_online || { check commandblock "player never joined" 1; return; }
  rcon "op $NAME" >/dev/null
  wait_exit
  echo "--- command block evidence ---"
  local out
  out=$(rcon "data get block 9 71 9 Command")
  echo "Command: $out"
  local r=0
  has "$out" "say hello" || { echo "command not set"; r=1; }
  grep -E "haste proxy:|Java exception" "$log" && r=1
  check commandblock "used the command block, typed a command, Done sent the update" "$r"
}


# ---- 15. Sprint: double-tapping W starts sprinting (vanilla LocalPlayer.aiStep) ----
# The server's player NBT has no sprint field, so the evidence is the speed: the script
# double-taps W (press, 2-tick release, press and hold); while the hold runs the runner
# samples the server's own position twice, one second apart, and computes the covered
# distance. Sprinting covers ~5.61 m/s (sprint speed 5.612), walking only ~4.32 m/s
# (vanilla `Player.getSprintingSpeed` and the walk speed): more than 5 m/s in one second
# is sprinting, less than 4.8 is walking.
sprint() {
  local log="$LOGDIR/sprint.log"
  NAME="spr_$$"
  area
  client_start "$ROOT/scripts/checks/sprint.txt" "$log"
  wait_online || { check sprint "player never joined" 1; return; }
  rcon "gamemode survival $NAME" >/dev/null
  local p1="" p2="" z1="" z2=""
  if wait_shot "$log" sprint-setup 90; then
    rcon "tp $NAME $CX 100 $CZ 0 0" >/dev/null
    rcon "execute at $NAME run fill ~-4 ~-1 ~-4 ~4 ~-1 ~4 stone" >/dev/null
    rcon "execute at $NAME run fill ~-3 ~ ~-3 ~3 ~6 ~3 air" >/dev/null
    wait_rcon "execute at $NAME if block ~ ~-1 ~ stone" "Test passed" 30 >/dev/null
  fi
  # Sample the position twice while the W hold runs (the script holds it 15 s).
  p1=$(rcon "data get entity $NAME Pos" 2>/dev/null)
  z1=$(pos_z "$p1")
  sleep 1
  p2=$(rcon "data get entity $NAME Pos" 2>/dev/null)
  z2=$(pos_z "$p2")
  wait_exit
  # Our platform, not the shared spawn's.
  rcon "fill $((CX - 4)) 99 $((CZ - 4)) $((CX + 4)) 106 $((CZ + 4)) air" >/dev/null
  echo "--- sprint evidence ---"
  echo "pos at sample 1: ${p1:-<none>}"
  echo "pos at sample 2: ${p2:-<none>}"
  local dist=0
  if [ -n "$z1" ] && [ -n "$z2" ]; then
    dist=$(awk -v a="$z1" -v b="$z2" 'BEGIN{d=b-a; if (d<0) d=-d; printf "%.2f", d}')
  fi
  echo "distance in ~1 s of the held W: ${dist} blocks"
  local r=0
  awk -v d="$dist" 'BEGIN{exit !(d>5.0)}' || { echo "the double tap did not reach sprint speed"; r=1; }
  check sprint "double-tapping W made the player cover ${dist} blocks/s (sprint speed)" "$r"
}

ALL=(hud death drop modes f2 craft pause creative-hotbars language particles recipe-book sign book commandblock sprint containers-crafting-table containers-furnace containers-anvil)
if [ $# -eq 0 ]; then
  set -- "${ALL[@]}"
fi
for f in "$@"; do
  case "$f" in
    hud | death | drop | modes | f2 | craft | pause | creative-hotbars | language | particles | recipe-book | sign | book | commandblock | sprint | containers-crafting-table | containers-furnace | containers-anvil) fn="${f//-/_}"; if declare -F "$fn" >/dev/null; then "$fn"; else check "$f" "no check function $fn" 1; fi ;;
    *) echo "unknown feature: $f" ;;
  esac
done

echo
echo "== $PASS passed, $FAIL failed =="
[ "$FAIL" -eq 0 ] || { echo "failed: ${FAILED[*]}"; exit 1; }
