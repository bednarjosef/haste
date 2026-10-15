#!/usr/bin/env bash
# Runs the feature check scripts in scripts/checks/ against the local test server.
# usage: scripts/checks/run-feature-checks.sh [feature ...]
#   features: hud death drop modes f2 craft pause creative-hotbars  (default: all)
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
  local spec_before= spec_after= creative_check= adventure_check=
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

ALL=(hud death drop modes f2 craft pause creative-hotbars)
if [ $# -eq 0 ]; then
  set -- "${ALL[@]}"
fi
for f in "$@"; do
  case "$f" in
    hud | death | drop | modes | f2 | craft | pause) "$f" ;;
    # the feature keeps the name the CLI prints; shell function names have no hyphen
    creative-hotbars) creative_hotbars ;;
    *) echo "unknown feature: $f" ;;
  esac
done

echo
echo "== $PASS passed, $FAIL failed =="
[ "$FAIL" -eq 0 ] || { echo "failed: ${FAILED[*]}"; exit 1; }
