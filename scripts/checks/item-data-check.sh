#!/usr/bin/env bash
# Item-data check: the held stack's item data (minecraft:tool component, Efficiency
# enchantment, attack_speed attribute modifiers) drives block breaking and the attack
# strength delay, with vanilla's exact tick counts.
#
# The client (offscreen, --script) holds attack on a floating block this runner places
# over RCON on the shared test server; the log's "finished breaking ... after N destroy
# ticks" lines are compared with vanilla's expected counts (computed from
# BlockBehaviour.getDestroyProgress / Player.getDestroySpeed / MultiPlayerGameMode.
# continueDestroyBlock; pinned by haste-physics mining.rs item_data_expected_break_ticks).
# Evidence the server agrees: the block is air on the server right after each finish,
# and no "Mismatch in destroy block pos" in the server log.
#
# usage: bash scripts/checks/item-data-check.sh
# Everything happens in this check's own area (x 140..150, z 0..10, y 71+); the player
# is teleported there and never touches the shared spawn floor.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PORT=25691
NAME=ItemDataCheck
BIN="${HASTE_BIN:-$ROOT/target/debug/haste}"
LOG="$ROOT/target/feature-checks/item-data.log"
EV="$ROOT/target/feature-checks/item-data.evidence"
mkdir -p "$ROOT/target/feature-checks"
rcon() { "$ROOT/scripts/vanilla-rcon.sh" --port "$PORT" "$@"; }

# The test server's own log (for the destroy-mismatch evidence): the java process whose
# working directory holds a server.properties naming our port.
SERVER_LOG=""
for p in /proc/[0-9]*; do
  cwd=$(readlink "$p/cwd" 2>/dev/null) || continue
  if [ -f "$cwd/server.properties" ] && grep -q "^server-port=$PORT\$" "$cwd/server.properties" 2>/dev/null; then
    SERVER_LOG="$cwd/logs/latest.log"
  fi
done
SERVER_LINES_BEFORE=0
[ -n "$SERVER_LOG" ] && SERVER_LINES_BEFORE=$(wc -l <"$SERVER_LOG" 2>/dev/null || echo 0)

# The target block floats at (145, 72, 6) — one block above the floor line, with air on
# every side along the ray (the corridor x 144..146, y 71..73, z 4..10 is kept clear), so
# once it breaks the still-held attack hits nothing out of reach and no stray break
# starts. The player stands at (145.5, 71, 4.5) facing +z (pitch 4 puts the crosshair on
# the block's centre).
TX=145
TY=72
TZ=6
MAINHAND="item replace entity $NAME weapon.mainhand with"

: >"$LOG"
# The evidence lines ("finished breaking ... destroy ticks", "held item components:
# attack_speed ...") are debug logs in the interaction module; the client's own
# `info` default caps everything else (main.rs adds the bare "info" directive).
timeout 240 env RUST_LOG="haste::player::interact=debug" "$BIN" \
  --server "127.0.0.1:$PORT" --username "$NAME" \
  --script "$ROOT/scripts/checks/item-data.txt" --seconds 220 >"$LOG" 2>&1 &
CLIENT_PID=$!

wait_online() {
  for _ in $(seq 1 200); do
    rcon list 2>/dev/null | grep -q "$NAME" && return 0
    sleep 0.5
  done
  return 1
}

wait_shot() { # name
  for _ in $(seq 1 900); do
    grep -q "script-$1.png" "$LOG" 2>/dev/null && return 0
    grep -q "game thread stopping" "$LOG" 2>/dev/null && return 1
    sleep 0.1
  done
  return 1
}

place_block() { # block id
  rcon "setblock $TX $TY $TZ $1" >/dev/null
}

fail=0
if wait_online; then
  # Our own area, away from the shared floor: repair the ground under the corridor,
  # carve the corridor, then stand the player at its mouth. The fills come before the
  # teleport so the player is never inside the filled region.
  rcon "forceload add 145 5" >/dev/null
  rcon "fill 144 70 4 146 70 6 minecraft:stone" >/dev/null
  rcon "setblock 145 69 7 minecraft:stone" >/dev/null
  rcon "fill 144 71 4 146 73 10 minecraft:air" >/dev/null
  rcon "tp $NAME 145.5 71.0 4.5 0 0" >/dev/null
  rcon "gamemode survival $NAME" >/dev/null
  rcon "clear $NAME" >/dev/null

  # Case 1: stone with the bare hand (expected 151 destroy ticks).
  if wait_shot item-data-setup; then
    rcon "$MAINHAND minecraft:air" >/dev/null
    place_block minecraft:stone
  fi
  # Case 2: stone with a wooden pickaxe (23).
  if wait_shot case-hand-done; then
    rcon "$MAINHAND minecraft:wooden_pickaxe" >/dev/null
    place_block minecraft:stone
  fi
  # Case 3: stone with a diamond pickaxe (6).
  if wait_shot case-wooden-done; then
    rcon "$MAINHAND minecraft:diamond_pickaxe" >/dev/null
    place_block minecraft:stone
  fi
  # Case 4: stone with a diamond pickaxe with Efficiency V (2).
  if wait_shot case-diamond-done; then
    rcon "$MAINHAND minecraft:diamond_pickaxe[minecraft:enchantments={\"minecraft:efficiency\":5}]" >/dev/null
    place_block minecraft:stone
  fi
  # Case 5: obsidian with a diamond pickaxe (188).
  if wait_shot case-efficiency-done; then
    rcon "$MAINHAND minecraft:diamond_pickaxe" >/dev/null
    place_block minecraft:obsidian
  fi
  # Case 6: cobweb with a diamond sword (8).
  if wait_shot case-obsidian-done; then
    rcon "$MAINHAND minecraft:diamond_sword" >/dev/null
    place_block minecraft:cobweb
  fi
  # Case 7: cobweb with shears (8).
  if wait_shot case-sword-done; then
    rcon "$MAINHAND minecraft:shears" >/dev/null
    place_block minecraft:cobweb
  fi
  # Attack strength delay evidence: a diamond axe held (the sword's line came with
  # case 6). Player.getCurrentItemAttackStrengthDelay = 20 / attack_speed.
  if wait_shot case-shears-done; then
    rcon "$MAINHAND minecraft:diamond_axe" >/dev/null
  fi
fi
wait "$CLIENT_PID"; client_rc=$?

# ---- evidence ----
{
  echo "--- break ticks (vanilla: BlockBehaviour.getDestroyProgress per tick, f32"
  echo "--- accumulation, one addition per tick starting on the press tick) ---"
} >"$EV"

# Each case's "finished breaking (x, y, z) after N destroy ticks" line for the target
# block, in order (stray breaks elsewhere would fail the count match below).
declare -a WANT=(151 23 6 2 188 8 8)
declare -a LABELS=(
  "stone with the bare hand"
  "stone with a wooden pickaxe"
  "stone with a diamond pickaxe"
  "stone with a diamond pickaxe with Efficiency V"
  "obsidian with a diamond pickaxe"
  "cobweb with a diamond sword"
  "cobweb with shears"
)
mapfile -t FINISHED < <(grep -F "finished breaking BlockPos { x: $TX, y: $TY, z: $TZ }" "$LOG" | grep -o 'after [0-9]* destroy ticks' | grep -o '[0-9]*' || true)
for i in "${!LABELS[@]}"; do
  want=${WANT[$i]}
  got=${FINISHED[$i]:-<none>}
  verdict="FAIL"
  [ "$got" = "$want" ] && verdict="ok"
  printf '%-50s expected %4s  measured %4s  %s\n' "${LABELS[$i]}" "$want" "$got" "$verdict" >>"$EV"
  [ "$got" = "$want" ] || fail=$((fail + 1))
done
# A break anywhere but the target block means the check's geometry leaked (a held
# attack caught something behind); that is a failure even when the counts match.
STRAYS=$(grep -c "finished breaking" "$LOG" || true)
[ "$STRAYS" = "7" ] || { echo "FAIL  breaks outside the target block: $((STRAYS - 7))" >>"$EV"; fail=$((fail + 1)); }

# The attack strength delay the client used, from the held-component log lines
# (f32 1.6 prints as 1.5999999046325684; the delay 20/1.6 as 12.5000007).
echo >>"$EV"
echo "--- attack strength delay (Player.getCurrentItemAttackStrengthDelay) ---" >>"$EV"
check_speed() { # label pattern
  line=$(grep -E "held item components: $2" "$LOG" | head -1)
  if [ -n "$line" ]; then
    echo "ok    $1: ${line##*attack_speed }"
  else
    echo "FAIL  $1: no held-component line matches '$2'"
    fail=$((fail + 1))
  fi
}
check_speed "diamond sword held (1.6 -> 12.5 delay ticks)" 'attack_speed 1\.59+[0-9]* \(12\.5[0-9]* delay ticks'
check_speed "diamond axe held (1.0 -> 20 delay ticks)" 'attack_speed 1 \(20 delay ticks'

# The server agrees: the last target block is air now (all seven broke server-side
# too; each earlier one was replaced by the next case's placement), and the server log
# has no destroy-mismatch warnings for our positions.
echo >>"$EV"
echo "--- server agreement ---" >>"$EV"
AIR=$(rcon "execute if block $TX $TY $TZ minecraft:air" | tail -1)
echo "target block now: ${AIR:-<no answer>}" >>"$EV"
echo "$AIR" | grep -q "Test passed" || fail=$((fail + 1))
if [ -n "$SERVER_LOG" ]; then
  MISMATCH=$(tail -n +"$((SERVER_LINES_BEFORE + 1))" "$SERVER_LOG" | grep -c "Mismatch in destroy block pos" || true)
  echo "server 'Mismatch in destroy block pos' lines during the run: $MISMATCH" >>"$EV"
  [ "$MISMATCH" = "0" ] || fail=$((fail + 1))
else
  echo "server log not found for port $PORT; mismatch check skipped" >>"$EV"
fi

cat "$EV"
# Clean up after ourselves: empty the main hand, clear the corridor back to air (the
# floating block is gone with the last case, any leftovers go now) and give up the
# forceload we asked for.
rcon "$MAINHAND minecraft:air" >/dev/null
rcon "fill 144 71 4 146 73 10 minecraft:air" >/dev/null
rcon "forceload remove 145 5" >/dev/null
if [ "$client_rc" = 0 ] && [ "$fail" = 0 ]; then
  echo "item-data check passed"
  exit 0
fi
echo "item-data check FAILED (client exit $client_rc, $fail evidence failures)"
exit 1
