#!/usr/bin/env bash
# Elytra live check: joins the vanilla reference server with the headless client, equips an
# elytra and teleports the player into the air over RCON, drives the walk script
# (crates/haste-client/walks/elytra.txt: fresh jump presses until one lands airborne) and reads
# the server's own verdict back over RCON: FallFlying must be 1b and the player must move
# forward. Also greps the server log for the movement checks ("moved wrongly", "moved too
# quickly", kicks). Prints PASS or FAIL, exit 0/1. Always stops the server it started.
# usage: scripts/elytra-check.sh [port]   (default 25630)
set -uo pipefail
PORT=${1:-25630}
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CACHE="${HASTE_CACHE:-$ROOT/cache}"
NAME=vanilla-reference
LOG="$CACHE/servers/$NAME/logs/latest.log"
cd "$ROOT" || exit 1
mkdir -p target/elytra
RCON="$ROOT/scripts/vanilla-rcon.sh --port $PORT"

cleanup() {
  local pid children
  if [ -n "${SERVER_PID:-}" ]; then
    children=$(pgrep -P "$SERVER_PID" 2>/dev/null || true)
    kill "$SERVER_PID" 2>/dev/null || true
    for pid in $children; do kill "$pid" 2>/dev/null || true; done
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  for pid in $(pgrep -f fabric-server-launch 2>/dev/null || true); do
    [ "$(readlink "/proc/$pid/cwd" 2>/dev/null || true)" = "$CACHE/servers/$NAME" ] && kill "$pid" 2>/dev/null || true
  done
}
trap cleanup EXIT

echo "== building release haste"
cargo build --release -q --bin haste || { echo "FAIL: cargo build"; exit 1; }

echo "== starting the vanilla reference server on port $PORT"
"$ROOT/scripts/vanilla-server.sh" "$PORT" 240 > target/elytra/server.out 2>&1 &
SERVER_PID=$!

echo -n "== waiting for RCON "
up=
for _ in $(seq 1 240); do
  if $RCON list >/dev/null 2>&1; then up=1; break; fi
  kill -0 "$SERVER_PID" 2>/dev/null || break
  sleep 0.5
done
if [ -z "$up" ]; then
  echo; echo "FAIL: RCON did not come up; target/elytra/server.out tail:"
  tail -5 target/elytra/server.out
  exit 1
fi
echo up
$RCON "gamerule send_command_feedback true" >/dev/null 2>&1 || true

echo "== joining headless as 'haste' (walk script crates/haste-client/walks/elytra.txt)"
HASTE_WALK_SCRIPT="$ROOT/crates/haste-client/walks/elytra.txt" \
  target/release/haste --headless --server "127.0.0.1:$PORT" --username haste --seconds 40 \
  > target/elytra/haste.log 2>&1 &
CLIENT_PID=$!

sleep 4
echo "== equipping the elytra and teleporting into the air (RCON)"
$RCON "item replace entity haste armor.chest with elytra" "tp haste 0 150 0"

sleep 7
echo "== reading the server's verdict (RCON)"
FALL=$($RCON "data get entity haste FallFlying")
POS=$($RCON "data get entity haste Pos")
echo "$FALL"
echo "$POS"
# The horizontal distance moved from the teleport at (0.5, 150, 0.5): a glide moves along the
# look direction, whichever way the spawn rotation points.
TRIPLE=$(echo "$POS" | grep -oE '\[[^]]+\]' | tr -d '[]d')
X=$(echo "$TRIPLE" | awk -F', *' '{print $1}')
Z=$(echo "$TRIPLE" | awk -F', *' '{print $3}')
MOVED=$(awk -v x="${X:-0.5}" -v z="${Z:-0.5}" 'BEGIN { print ((x-0.5)^2 + (z-0.5)^2 > 1) ? "yes" : "no" }')

wait "$CLIENT_PID" 2>/dev/null
RC=$?

FAIL=
echo "$FALL" | grep -q '1b' || FAIL="FallFlying is not 1b"
[ "$MOVED" = yes ] || FAIL="${FAIL:+$FAIL; }the player did not move horizontally (pos ${TRIPLE:-?})"
[ "$RC" -eq 0 ] || FAIL="${FAIL:+$FAIL; }haste exited $RC"

while IFS= read -r line; do
  case "$line" in
    *"lost connection: Disconnected") echo "server (allowed): $line";;
    *) echo "server: $line"; FAIL="${FAIL:+$FAIL; }server: $line";;
  esac
done < <(grep -iE 'moved wrongly|moved too quickly|kicked|Exception' "$LOG" 2>/dev/null || true)

for pat in 'panicked' 'Error:'; do
  while IFS= read -r line; do
    echo "client: $line"
    FAIL="${FAIL:+$FAIL; }client: $line"
  done < <(grep -F -- "$pat" target/elytra/haste.log 2>/dev/null || true)
done

grep -E 'mark ' target/elytra/haste.log | tail -8

if [ -n "$FAIL" ]; then
  echo "FAIL: $FAIL"
  exit 1
fi
echo PASS
