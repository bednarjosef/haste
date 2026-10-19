#!/usr/bin/env bash
# Run a haste client invocation at the bench-vs-vanilla spot (the spectator view over
# land): teleports the bench player there in a loop while the client joins, so
# `--pos`-waiting modes (--screenshot, --script with wait_until settled, --bench)
# settle at the spot. The reference server must be running (default port 25620).
#
# usage: scripts/bench-spot-run.sh <username> <haste args...>
#   e.g. scripts/bench-spot-run.sh meshvis --screenshot target/shots/spot.png \
#          --render-distance 12 --pos ... --yaw 180 --pitch 15
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PORT=${HASTE_BENCH_PORT:-25620}
USER_NAME=$1
shift
X=144.5 Y=100 Z=-191.5 YAW=180 PITCH=15

# Teleport until the player is at the spot, then stop: a tp also resets rotation, so
# looping past arrival would keep yanking the camera (the bench's still camera and a
# script's `look` turns both fight it).
( for _ in $(seq 90); do
    "$ROOT/scripts/vanilla-rcon.sh" --port "$PORT" "gamemode spectator $USER_NAME" >/dev/null 2>&1 || true
    "$ROOT/scripts/vanilla-rcon.sh" --port "$PORT" "tp $USER_NAME $X $Y $Z $YAW $PITCH" >/dev/null 2>&1 || true
    sleep 1
    "$ROOT/scripts/vanilla-rcon.sh" --port "$PORT" \
      "execute if entity @a[name=$USER_NAME,x=$X,y=$Y,z=$Z,distance=..0.5]" 2>/dev/null \
      | grep -q "Test passed" && break
  done ) &
TP=$!

timeout 120 "$ROOT/target/release/haste" --server "127.0.0.1:$PORT" --username "$USER_NAME" "$@"
RC=$?
kill "$TP" 2>/dev/null || true
wait "$TP" 2>/dev/null || true
exit $RC
